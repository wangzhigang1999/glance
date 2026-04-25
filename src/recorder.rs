//! 基于 esp-sr AFE 的 VAD 触发录音(双麦 BSS + WebRTC NS + WebRTC VAD)。
//!
//! 旧版是手搓能量阈值 VAD,关门 / 键盘 / 空调一来就误开,段切碎到大模型 ASR
//! 拿不到完整句子。现在用 esp-sr 官方 pipeline:对人声敏感,对噪声沉默。
//!
//! 数据流(两线程):
//! ```text
//! [feed thread]                       [fetch thread]
//! Mic.read(stereo i16) ──▶ afe.feed()
//!                                      afe.fetch() ──▶ FetchFrame
//!                                                       ├─ vad_speech?
//!                                                       ├─ data: 单声道去噪
//!                                                       └─ vad_cache: 触发前预滚
//!                                                          │
//!                                                          ▼
//!                                                       state machine
//!                                                          │
//!                                                          ▼
//!                                                       WavWriter → /storage/*.wav
//! ```
//!
//! AFE 内部已经做了:
//! - 双麦波束(BSS)→ 单声道
//! - WebRTC NS → 压低稳态噪声
//! - WebRTC VAD + min_speech/min_noise 时间门 → vad_state 是去抖过的
//! - 触发前的几帧自动缓存到 `vad_cache`,保证首字不被切
//!
//! 我们这层只管 vad_speech 转换 + 段长上下限 + 写文件。

use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    sync::{
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::{
    hw::{afe::Afe, mic::Mic},
    net::time::{unix_secs, utc_from_unix},
};

// ============================================================================
// 录音索引(内存 cache)—— 让 HTTP /api/recordings 不必每次都 SPIFFS read_dir。
// SPIFFS 目录扫描 O(N),68 个文件就要 12 秒,超时不可避免。
// ============================================================================

#[derive(Clone, Debug)]
pub struct RecEntry {
    pub name: String,
    pub size: u64,
}

static INDEX: Mutex<Vec<RecEntry>> = Mutex::new(Vec::new());

/// 启动时扫一次 /storage 把已经在的 wav 灌进 index。
/// 这是**唯一一次** SPIFFS read_dir,以后 HTTP 全走 index。
pub fn index_scan_storage() {
    let mut g = INDEX.lock().unwrap();
    g.clear();
    if let Ok(dir) = fs::read_dir("/storage") {
        for entry in dir.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !name.ends_with(".wav") {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            g.push(RecEntry { name, size });
        }
    }
    g.sort_by(|a, b| b.name.cmp(&a.name));
    log::info!("recordings index seeded: {} entries", g.len());
}

fn index_push(name: String, size: u64) {
    let mut g = INDEX.lock().unwrap();
    // 文件名是时间戳格式,新的字典序更大,直接插队首
    g.insert(0, RecEntry { name, size });
}

pub fn index_remove(name: &str) -> bool {
    let mut g = INDEX.lock().unwrap();
    if let Some(pos) = g.iter().position(|e| e.name == name) {
        g.remove(pos);
        true
    } else {
        false
    }
}

pub fn index_clear() {
    INDEX.lock().unwrap().clear();
}

/// 返回 (entries_window, total_count, total_bytes)。`limit=0` = 不限。
pub fn index_list_paged(offset: usize, limit: usize) -> (Vec<RecEntry>, usize, u64) {
    let g = INDEX.lock().unwrap();
    let total = g.len();
    let total_size: u64 = g.iter().map(|e| e.size).sum();
    let take = if limit == 0 { total } else { limit };
    let entries: Vec<RecEntry> = g.iter().skip(offset).take(take).cloned().collect();
    (entries, total, total_size)
}

const SAMPLE_RATE: u32 = 16_000;

// ----- 帧时间换算(AFE_TYPE_SR/HIGH_PERF 的 chunk 是 256 sample = 16ms;实际从 Afe 取) -----
// 下面的"frames"全部按 fetch 帧数算

/// 录音过程中允许的尾静音(AFE 已经有 2s min_noise 内部去抖,上层再叠 ~1s 收尾)。
/// 总尾部 ≈ 3s,自然换气/思考/找词都够。之前设 60 帧(~1.9s)是想宽一点,但叠
/// 上 AFE 的 2s 一共 4s,误触发段后面 ~4s 全是底噪,文件听着就是纯噪音。
const HANGOVER_FRAMES: u32 = 30;
/// 段太短直接 discard,认作误触发(< ~1s,通常是关门/咳嗽)。
const MIN_SEG_FRAMES: u32 = 32;
/// 段结束时的能量闸刀:整段单帧 RMS 的最大值低于这个就 Discard。
/// 32ms 帧的 i16 RMS 单位。典型人声 ≥ 2000(说话时声门振动),纯底噪 < 500。
/// 800 是中间值留点余量,CLOSE 日志会打 max_rms 方便回头调。
/// 这是真正能识别"全段都是底噪"的最后一道关 —— VAD 看不出来的 tail 它能看出来。
const RMS_DISCARD_THRESHOLD: u32 = 800;

/// 流式 chunk 阈值:fetch 攒够这么多 i16 就 send 给 finalizer 落盘。
/// 256KB ≈ 8s 音频(单声道 16kHz × 2 byte = 32KB/s)。给 finalizer 的 channel
/// capacity=2,所以最多在 PSRAM 里压 ~16s 未写入数据(512KB),安全 OK。
const CHUNK_SAMPLES: usize = 256 * 1024 / 2;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    Recording,
}

/// 启动 feed + fetch + finalizer 三个线程。`mic` 被 feed 线程拿走,`afe` 用 Arc 共享。
///
/// 三线程拆分原因:
/// - feed: 阻塞在 I2S read,绝不能被别的事卡住
/// - fetch: 阻塞在 AFE fetch,只做状态机 + 内存 push,不碰 SPIFFS
/// - finalizer: 段结束时一次性把 PSRAM 里的整段 PCM 写到 SPIFFS,慢就让它慢
///
/// 之前的两线程把 SPIFFS write 放在 fetch 里,SPIFFS 单次 write 吃 30-100ms,
/// 长段累积下来 fetch 跟不上 AFE feed,ringbuf 持续 full。
pub fn spawn_afe_pipeline(
    mut mic: Mic,
    afe: Arc<Afe>,
    has_storage: bool,
    tz_off_s: i64,
) -> Result<()> {
    let feed_chunk = afe.feed_chunksize();
    let feed_ch = afe.feed_channels();
    let total_in = feed_chunk * feed_ch;
    log::info!(
        "AFE pipeline start: feed_chunk={} sample × {} ch = {} i16/frame",
        feed_chunk,
        feed_ch,
        total_in
    );

    // ---- finalizer channel:fetch → finalizer ----
    // capacity=2:一段在写,下一段开始攒;再多堆积时 fetch 会短暂 block,
    // 这种背压比无界更安全(防 RAM 爆)。
    let (tx, rx) = mpsc::sync_channel::<FinalizerCmd>(2);

    // ---- feed 线程:I2S read → afe.feed ----
    let afe_feed = Arc::clone(&afe);
    thread::Builder::new()
        .name("afe_feed".into())
        // 8KB:就读 I2S + memcpy + feed,栈占用很少
        .stack_size(8192)
        .spawn(move || {
            let mut buf = vec![0i16; total_in];
            loop {
                match mic.read(&mut buf, 1000) {
                    Ok(n) => {
                        // n = i16 sample count;期望 == total_in。少了就用 0 填齐。
                        if n < total_in {
                            for s in &mut buf[n..] {
                                *s = 0;
                            }
                        }
                        if let Err(e) = afe_feed.feed(&buf) {
                            log::warn!("afe.feed: {e:#}");
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                    Err(e) => {
                        log::error!("mic.read: {e:#}");
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
        })?;

    // ---- finalizer 线程:把 (path, Vec<i16>) 写成 WAV ----
    thread::Builder::new()
        .name("afe_finalizer".into())
        // 12KB:File::create + write_all + flush,SPIFFS API 偶尔吃栈
        .stack_size(12288)
        .spawn(move || finalizer_loop(rx))?;

    // ---- fetch 线程:afe.fetch → 状态机 → 内存 push,close 时 send 给 finalizer ----
    thread::Builder::new()
        .name("afe_fetch".into())
        // 12KB:状态机 + log::info! + format!(filename),不碰 SPIFFS
        .stack_size(12288)
        .spawn(move || fetch_loop(afe, has_storage, tz_off_s, tx))?;

    Ok(())
}

/// finalizer 接受的流式命令。Begin → 多次 Chunk → End/Discard。
/// 命令必须按顺序到达(SyncSender 是 FIFO,fetch 线程串行 send,OK)。
enum FinalizerCmd {
    /// 开新段:打开文件,写 0-size 占位 WAV 头(crash 也是合法 0 长度 WAV)
    Begin { path: String },
    /// 追加一段 PCM(已经是 fetch 累积的 chunk)
    Chunk { samples: Vec<i16> },
    /// 段结束:flush + seek 回头补 RIFF/data 真实 size + 落 index
    End,
    /// 段太短(< MIN_SEG_FRAMES):删文件,index 也不进
    Discard,
}

/// finalizer 线程内部维护当前打开的文件 + 已写 sample 数。
struct OpenSeg {
    file: File,
    path: String,
    total_samples: u32,
}

fn finalizer_loop(rx: mpsc::Receiver<FinalizerCmd>) {
    let mut cur: Option<OpenSeg> = None;
    let mut t_open = Instant::now();
    while let Ok(cmd) = rx.recv() {
        match cmd {
            FinalizerCmd::Begin { path } => {
                if let Some(prev) = cur.take() {
                    log::warn!(
                        "finalizer: Begin without End on previous {}, dropping",
                        prev.path
                    );
                    let _ = fs::remove_file(&prev.path);
                }
                t_open = Instant::now();
                match File::create(&path).and_then(|mut f| {
                    f.write_all(&wav_header(0))?;
                    Ok(f)
                }) {
                    Ok(file) => {
                        cur = Some(OpenSeg {
                            file,
                            path,
                            total_samples: 0,
                        });
                    }
                    Err(e) => log::warn!("finalizer: Begin {path} failed: {e:#}"),
                }
            }
            FinalizerCmd::Chunk { samples } => {
                if let Some(seg) = cur.as_mut() {
                    let bytes: &[u8] = unsafe {
                        core::slice::from_raw_parts(
                            samples.as_ptr() as *const u8,
                            samples.len() * 2,
                        )
                    };
                    if let Err(e) = seg.file.write_all(bytes) {
                        log::warn!("finalizer: Chunk write {} failed: {e:#}", seg.path);
                        // 保持 cur,后续 End 会去补 header(已写部分仍是合法 prefix)
                    }
                    seg.total_samples += samples.len() as u32;
                } else {
                    log::warn!(
                        "finalizer: Chunk without Begin, dropping {} samples",
                        samples.len()
                    );
                }
            }
            FinalizerCmd::End => {
                if let Some(mut seg) = cur.take() {
                    let data_size = seg.total_samples * 2;
                    let res = seg
                        .file
                        .seek(SeekFrom::Start(0))
                        .and_then(|_| seg.file.write_all(&wav_header(data_size)))
                        .and_then(|_| seg.file.flush());
                    if let Err(e) = res {
                        log::warn!("finalizer: End rewrite header {} failed: {e:#}", seg.path);
                        let _ = fs::remove_file(&seg.path);
                    } else {
                        let total_bytes = data_size as u64 + 44;
                        let basename = seg
                            .path
                            .rsplit_once('/')
                            .map(|(_, n)| n)
                            .unwrap_or(seg.path.as_str())
                            .to_string();
                        index_push(basename, total_bytes);
                        log::info!(
                            "finalizer: saved {} ({} samples, {:.1}s wall total)",
                            seg.path,
                            seg.total_samples,
                            t_open.elapsed().as_secs_f32()
                        );
                    }
                }
            }
            FinalizerCmd::Discard => {
                if let Some(seg) = cur.take() {
                    drop(seg.file);
                    let _ = fs::remove_file(&seg.path);
                    log::info!("finalizer: discarded {}", seg.path);
                }
            }
        }
    }
    log::warn!("finalizer: channel closed, exiting");
}

fn fetch_loop(afe: Arc<Afe>, has_storage: bool, tz_off_s: i64, tx: SyncSender<FinalizerCmd>) -> ! {
    let fetch_chunk = afe.fetch_chunksize();
    let fps = SAMPLE_RATE as f32 / fetch_chunk as f32;
    log::info!(
        "AFE fetch loop start (fetch_chunk={} sample, fps={:.1}, hangover={}f min={}f chunk={}KB tz={}h storage={})",
        fetch_chunk,
        fps,
        HANGOVER_FRAMES,
        MIN_SEG_FRAMES,
        CHUNK_SAMPLES * 2 / 1024,
        tz_off_s / 3600,
        has_storage,
    );

    // 给 SNTP / RTC 一点时间灌系统时间。最多等 30s,文件名不至于用 uptime。
    if has_storage {
        wait_for_clock(Duration::from_secs(30));
    }

    let mut state = State::Idle;
    let mut silence: u32 = 0;
    let mut seg_frames: u32 = 0;
    let mut seg: Option<SegBuf> = None;
    let mut log_tick: u32 = 0;

    loop {
        let Some(frame) = afe.fetch() else {
            log::warn!("AFE fetch returned None (ret_value < 0)");
            thread::sleep(Duration::from_millis(50));
            continue;
        };

        // 周期性日志:每 ~62 帧 ≈ 1s
        log_tick = (log_tick + 1) % 62;
        if log_tick == 0 {
            log::info!(
                "AFE: state={:?} speech={} seg={} cache={}",
                state,
                frame.vad_speech,
                seg_frames,
                frame.vad_cache.len()
            );
        }

        match state {
            State::Idle => {
                if frame.vad_speech && has_storage {
                    let path = make_filename(tz_off_s);
                    log::info!("AFE: OPEN {}", path);
                    // 通知 finalizer 开文件 + 占位头(0 size,crash 也是合法 WAV)
                    if let Err(e) = tx.send(FinalizerCmd::Begin { path: path.clone() }) {
                        log::warn!("AFE: send Begin failed: {e}");
                        continue;
                    }
                    let mut s = SegBuf::new(path);
                    // AFE 内置 pre-roll:vad_cache 是触发前缓存的音频
                    if !frame.vad_cache.is_empty() {
                        s.push(frame.vad_cache);
                    }
                    s.push(frame.data);
                    s.flush_if_full(&tx);
                    seg = Some(s);
                    state = State::Recording;
                    silence = 0;
                    // cache 帧不算独立帧,只算当前这帧
                    seg_frames = 1;
                } else if frame.vad_speech && !has_storage {
                    log::info!("AFE: would-open (speech, no storage)");
                }
            }
            State::Recording => {
                // 尾静音也照写,保留段末尾的呼吸/收尾,避免 ASR 看着像被截断。
                if let Some(s) = seg.as_mut() {
                    s.push(frame.data);
                    s.flush_if_full(&tx);
                }
                seg_frames += 1;

                if frame.vad_speech {
                    silence = 0;
                } else {
                    silence += 1;
                }

                if silence >= HANGOVER_FRAMES {
                    if let Some(mut s) = seg.take() {
                        let max_rms = s.max_rms();
                        let too_short = seg_frames < MIN_SEG_FRAMES;
                        let too_quiet = max_rms < RMS_DISCARD_THRESHOLD;
                        if too_short || too_quiet {
                            // VAD 看不出来的"全段都是底噪"在这里被 RMS 闸刀兜住:
                            // 误触发后强制录的 ~3s tail 里没有人声能量,max_rms 通常 < 500
                            let reason = if too_short { "short" } else { "quiet" };
                            log::info!(
                                "AFE: DISCARD {} ({} frames, max_rms={}) {}",
                                reason,
                                seg_frames,
                                max_rms,
                                s.path()
                            );
                            let _ = tx.send(FinalizerCmd::Discard);
                        } else {
                            // fetch_chunksize=512 sample @ 16kHz = 32ms/frame ⇒ 31.25 fps
                            let secs = seg_frames as f32 * fetch_chunk as f32 / SAMPLE_RATE as f32;
                            log::info!(
                                "AFE: CLOSE {} ({} frames, {:.1}s, max_rms={}) → finalizer",
                                s.path(),
                                seg_frames,
                                secs,
                                max_rms,
                            );
                            s.flush(&tx);
                            let _ = tx.send(FinalizerCmd::End);
                        }
                    }
                    state = State::Idle;
                    silence = 0;
                    seg_frames = 0;
                }
            }
        }
    }
}

/// 等系统时间被同步(从 RTC 灌或 SNTP 拉),最多等 `timeout`。
fn wait_for_clock(timeout: Duration) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if unix_secs().is_some() {
            log::info!("AFE: system clock ready, recording with timestamps");
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
    log::warn!(
        "AFE: clock not synced after {}s, recordings will use uptime-based names",
        timeout.as_secs()
    );
}

fn make_filename(tz_off_s: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    if let Some(unix) = unix_secs() {
        // 加毫秒后缀防同秒撞名
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| (d.as_millis() % 1000) as u32)
            .unwrap_or(0);
        // 文件名用本地时间(UTC + tz_off);RTC 仍存 UTC,这里只影响显示
        let (y, mo, d, h, mi, s) = utc_from_unix(unix + tz_off_s);
        format!(
            "/storage/{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}.wav",
            y, mo, d, h, mi, s, ms
        )
    } else {
        // 时间没同步退回 monotonic clock 取唯一后缀
        let us = unsafe { esp_idf_svc::sys::esp_timer_get_time() };
        format!("/storage/up-{}.wav", us / 1000)
    }
}

// ============================================================================
// WAV 写盘:开文件时写合法的 0-data-size 头(crash 也是合法 WAV);关文件时
// seek 回头补 RIFF/data 长度。
// ============================================================================

/// fetch 线程侧的当前段累积器:每攒满 CHUNK_SAMPLES 就 send 给 finalizer,
/// 段彻底结束(close)再 send End/Discard。fetch 完全不碰 SPIFFS。
///
/// SPIRAM_MALLOC_ALWAYSINTERNAL=4096 → buf 自动落 PSRAM,不挤内部 SRAM。
struct SegBuf {
    buf: Vec<i16>,
    path: String,
    /// 整段里"单个 fetch 帧 RMS"的最大值。close 时拿来跟 RMS_DISCARD_THRESHOLD 比。
    /// 单帧 32ms 够长能反映浊辅音/元音的能量,够短不会被静音段稀释。
    max_rms: u32,
}

impl SegBuf {
    fn new(path: String) -> Self {
        Self {
            buf: Vec::with_capacity(CHUNK_SAMPLES),
            path,
            max_rms: 0,
        }
    }

    fn push(&mut self, s: &[i16]) {
        if !s.is_empty() {
            // sum_sq 上限:512 sample × 32768² ≈ 5.5e11,落在 u64
            let sum_sq: u64 = s.iter().map(|&v| (v as i32 * v as i32) as u64).sum();
            let mean_sq = (sum_sq / s.len() as u64) as f32;
            // ESP32-S3 有 FPU,f32 sqrt 单周期。每 32ms 一次,开销可忽略
            let rms = mean_sq.sqrt() as u32;
            if rms > self.max_rms {
                self.max_rms = rms;
            }
        }
        self.buf.extend_from_slice(s);
    }

    fn max_rms(&self) -> u32 {
        self.max_rms
    }

    /// buf 满了就 send 一次 Chunk 到 finalizer,buf 清空准备下一块。
    /// channel 满会阻塞 fetch(背压),罕见情况:finalizer 在写上一个超大 chunk。
    fn flush_if_full(&mut self, tx: &SyncSender<FinalizerCmd>) {
        if self.buf.len() >= CHUNK_SAMPLES {
            self.flush(tx);
        }
    }

    fn flush(&mut self, tx: &SyncSender<FinalizerCmd>) {
        if self.buf.is_empty() {
            return;
        }
        let chunk = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK_SAMPLES));
        let _ = tx.send(FinalizerCmd::Chunk { samples: chunk });
    }

    fn path(&self) -> &str {
        &self.path
    }
}

/// 16kHz / mono / 16-bit 的 WAV 头,`data_size` 是 PCM data chunk 的字节数。
fn wav_header(data_size: u32) -> [u8; 44] {
    let total_size = data_size + 36;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&total_size.to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    h[32..34].copy_from_slice(&2u16.to_le_bytes()); // block align
    h[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits/sample
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_size.to_le_bytes());
    h
}

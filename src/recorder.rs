//! VAD 触发的录音线程:听到人说话就开 WAV 文件,静音 1.2s 关闭。
//!
//! VAD 算法:**自适应能量阈值 + 磁滞 + "宽进严出"**
//! - 空闲态:EMA 跟踪噪声底(NF),阈值 = max(NF × 4, 200)
//! - 录音态:close 阈值放低到 NF × 1.8(磁滞,中间小停顿不会出戏)
//! - 静音连续 38 帧(~1.2s)→ 关段
//! - 段长 < 16 帧(~512ms)→ 直接删,认作误触发(关门 / 咳嗽)
//! - 段长 ≥ 940 帧(~30s)→ 强制切段,防背景噪声常驻把整盘录满
//! - 每段开头预滚 200ms,确保第一个字不被切
//!
//! 文件名:`/storage/YYYYMMDD-HHMMSS.wav`(系统时间没同步则 `/storage/up{seconds}.wav`)
//!
//! 输入:I2S 16kHz / 16-bit / 立体声(L=MIC1,R=MIC2),只取 left 写盘(单声道 WAV)。

use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{
    hw::mic::Mic,
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
/// 每帧 512 个 mono i16(左声道),~32ms
const FRAME_MONO: usize = 512;
/// 31.25 fps;参数都按 fps≈31 算
const _FPS: u32 = SAMPLE_RATE / FRAME_MONO as u32;

/// 预滚:段开头前 ~224ms 放进文件,避免第一个音节被切
const PREROLL_FRAMES: usize = 7;
/// hangover:静音连续 ~2.4s 才判定段结束。
/// 自然句间停顿 / 思考 / 换气常 1-2s,设短了一段对话被切成 N 段。
const HANGOVER_FRAMES: u32 = 75;
/// 最短段:< ~512ms 直接 discard,认作误触发
const MIN_SEG_FRAMES: u32 = 16;
/// 最长段:~60s 上限,防背景噪声常驻把卡录爆;长独白也够
const MAX_SEG_FRAMES: u32 = 1900;
/// 启动头 ~2s 不开新段,避开 ES7210/I2S 上电瞬态噗
const STARTUP_BLANK_FRAMES: u32 = 60;

/// 噪声底初值。环境会很快被 EMA 拉到真实底
const NF_INIT: f32 = 50.0;
/// 空闲态 EMA 平滑系数(每帧 2% 拉向当前 RMS)
const NF_ALPHA: f32 = 0.02;
/// 开门阈值倍率:RMS > NF × 4 触发录音
const OPEN_RATIO: f32 = 4.0;
/// 关门阈值倍率:录音中 RMS < NF × 1.3 才计 silence。
/// 旧值 1.8 太敏感 → 句中弱音节一掉就计为静音 → hangover 触发 → 切段。
/// 现在只有 RMS 真的接近底噪才计,正常说话不会出戏。
const CLOSE_RATIO: f32 = 1.3;
/// 噪声底极低时(完全安静)也别太敏感,绝对地板 200
const ABS_MIN_OPEN: f32 = 200.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    Recording,
}

/// 阻塞式录音线程。`has_storage = false` 时只跑 VAD 日志,不写盘。
/// `tz_off_s` 用来把 RTC 的 UTC 转换成本地时间写文件名。
pub fn vad_record_loop(mut mic: Mic, has_storage: bool, tz_off_s: i64) -> ! {
    log::info!(
        "VAD recorder start (preroll={}f hangover={}f({}s) min={}f max={}f open=NF×{} close=NF×{} tz={}h storage={})",
        PREROLL_FRAMES,
        HANGOVER_FRAMES,
        HANGOVER_FRAMES as f32 * FRAME_MONO as f32 / SAMPLE_RATE as f32,
        MIN_SEG_FRAMES,
        MAX_SEG_FRAMES,
        OPEN_RATIO,
        CLOSE_RATIO,
        tz_off_s / 3600,
        has_storage,
    );

    // 给 SNTP / RTC 一点时间灌系统时间(0 帧 = 不等)。最多等 30s,文件名不至于用 uptime。
    if has_storage {
        wait_for_clock(Duration::from_secs(30));
    }

    let mut stereo = [0i16; 1024]; // 1024 i16 = 512 stereo points = 32ms
    let mut mono = [0i16; FRAME_MONO];
    let mut preroll: VecDeque<[i16; FRAME_MONO]> = VecDeque::with_capacity(PREROLL_FRAMES + 1);

    let mut nf = NF_INIT;
    let mut state = State::Idle;
    let mut silence: u32 = 0;
    let mut seg_frames: u32 = 0;
    let mut writer: Option<WavWriter> = None;
    let mut total_frames: u32 = 0;

    // 周期性 dump VAD 状态(每 ~30 帧 ≈ 1s)
    let mut log_tick: u32 = 0;

    loop {
        let n = match mic.read(&mut stereo, 1000) {
            Ok(n) => n,
            Err(e) => {
                log::error!("mic read: {e:#}");
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        // n = i16 sample count (L+R 算两点);左声道在偶数 idx
        let mono_n = (n / 2).min(FRAME_MONO);
        for i in 0..mono_n {
            mono[i] = stereo[i * 2];
        }
        let frame = &mono[..mono_n];
        let rms = compute_rms(frame);
        total_frames += 1;
        let in_blank = total_frames < STARTUP_BLANK_FRAMES;

        // --- 周期性日志 ---
        log_tick = (log_tick + 1) % 30;
        if log_tick == 0 {
            log::info!(
                "VAD: rms={:.0} nf={:.0} state={:?} seg={}{}",
                rms,
                nf,
                state,
                seg_frames,
                if in_blank { " (blank)" } else { "" }
            );
        }

        let open_th = (nf * OPEN_RATIO).max(ABS_MIN_OPEN);
        let close_th = nf * CLOSE_RATIO;

        match state {
            State::Idle => {
                // EMA 噪声底
                nf = nf * (1.0 - NF_ALPHA) + rms * NF_ALPHA;

                // 维护 pre-roll 环形 buffer
                if preroll.len() >= PREROLL_FRAMES {
                    preroll.pop_front();
                }
                let mut snap = [0i16; FRAME_MONO];
                snap[..mono_n].copy_from_slice(frame);
                preroll.push_back(snap);

                if rms > open_th && has_storage && !in_blank {
                    let path = make_filename(tz_off_s);
                    match WavWriter::create(&path) {
                        Ok(mut w) => {
                            log::info!(
                                "VAD: OPEN segment → {} (rms={:.0} > open={:.0})",
                                path,
                                rms,
                                open_th
                            );
                            // 把 pre-roll 全部冲进文件
                            for prev_frame in preroll.drain(..) {
                                let _ = w.write_samples(&prev_frame);
                            }
                            // 当前帧也写
                            let _ = w.write_samples(frame);
                            writer = Some(w);
                            state = State::Recording;
                            silence = 0;
                            // 已写入帧数:pre-roll + 当前 = N+1
                            seg_frames = (PREROLL_FRAMES as u32) + 1;
                        }
                        Err(e) => log::warn!("VAD: open WAV failed, stay Idle: {e:#}"),
                    }
                } else if rms > open_th && !has_storage {
                    // 没卡也至少打个标记,方便调阈值
                    log::info!(
                        "VAD: would-open (rms={:.0} > open={:.0}), no storage",
                        rms,
                        open_th
                    );
                }
            }
            State::Recording => {
                if let Some(w) = writer.as_mut() {
                    let _ = w.write_samples(frame);
                }
                seg_frames += 1;

                if rms < close_th {
                    silence += 1;
                } else {
                    silence = 0;
                }

                let stop_silence = silence >= HANGOVER_FRAMES;
                let stop_max = seg_frames >= MAX_SEG_FRAMES;

                if stop_silence || stop_max {
                    if let Some(w) = writer.take() {
                        let reason = if stop_max { "max" } else { "silence" };
                        if seg_frames < MIN_SEG_FRAMES {
                            log::info!(
                                "VAD: DISCARD short ({} frames < {}) {}",
                                seg_frames,
                                MIN_SEG_FRAMES,
                                w.path()
                            );
                            let _ = w.discard();
                        } else {
                            let secs = seg_frames as f32 * FRAME_MONO as f32 / SAMPLE_RATE as f32;
                            log::info!(
                                "VAD: CLOSE {} ({} frames, {:.1}s, reason={})",
                                w.path(),
                                seg_frames,
                                secs,
                                reason
                            );
                            // finalize 出错(写盘 / seek / flush) → 文件可能半写,直接清掉,
                            // 避免留个 0 字节 header 孤儿在盘上
                            let path_for_cleanup = w.path().to_string();
                            if let Err(e) = w.finalize() {
                                log::warn!("VAD: finalize failed, removing partial: {e:#}");
                                let _ = fs::remove_file(&path_for_cleanup);
                            }
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

fn compute_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sum_sq: u64 = 0;
    for &s in samples {
        let v = s as i32;
        sum_sq += (v * v) as u64;
    }
    ((sum_sq / samples.len() as u64) as f64).sqrt() as f32
}

/// 等系统时间被同步(从 RTC 灌或 SNTP 拉),最多等 `timeout`。
fn wait_for_clock(timeout: Duration) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if unix_secs().is_some() {
            log::info!("VAD: system clock ready, recording with timestamps");
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    log::warn!(
        "VAD: clock not synced after {}s, recordings will use uptime-based names",
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
// WAV 写盘:开文件时占位 44 字节,关文件时 seek 回头补 RIFF/data 长度。
// ============================================================================

struct WavWriter {
    file: File,
    path: String,
    samples_written: u32,
}

impl WavWriter {
    fn create(path: &str) -> std::io::Result<Self> {
        let mut file = File::create(path)?;
        // 写 data_size=0 的合法 WAV 头。finalize 会回填真实大小;
        // 即使断电没等到 finalize,文件本身也是合法 WAV(只是 0 长度),
        // 而不是 44 字节零头 + 孤儿 PCM(soundfile / PyAV 不认)。
        file.write_all(&wav_header(0))?;
        Ok(Self {
            file,
            path: path.to_string(),
            samples_written: 0,
        })
    }

    fn write_samples(&mut self, samples: &[i16]) -> std::io::Result<()> {
        // i16 LE → u8 切片;ESP32-S3 小端,直接 transmute
        let bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 2)
        };
        self.file.write_all(bytes)?;
        self.samples_written += samples.len() as u32;
        Ok(())
    }

    fn finalize(mut self) -> std::io::Result<()> {
        let data_size = self.samples_written * 2;
        let hdr = wav_header(data_size);
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&hdr)?;
        self.file.flush()?;

        // 推进内存 index,HTTP /api/recordings 直接读这里,不走 SPIFFS read_dir
        let total_bytes = (data_size as u64) + 44;
        let basename = self
            .path
            .rsplit_once('/')
            .map(|(_, n)| n)
            .unwrap_or(self.path.as_str())
            .to_string();
        index_push(basename, total_bytes);
        Ok(())
    }

    fn discard(self) -> std::io::Result<()> {
        // file 在 self drop 时自动 close,然后 remove
        let path = self.path.clone();
        drop(self);
        std::fs::remove_file(&path)
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

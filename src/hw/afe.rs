//! esp-sr AFE 安全 wrapper(双麦 BSS + WebRTC NS + WebRTC VAD,无唤醒词无 AEC)。
//!
//! 取代之前手搓的能量阈值 VAD —— 后者在关门 / 键盘 / 空调噪声面前必误判,
//! 段切碎到大模型 ASR 拿不到完整句子。AFE 是 Espressif 官方做的语音前端
//! pipeline,带子带能量 + 高斯模型 VAD,正合"对人声敏感、对噪声沉默"。
//!
//! 数据流:
//! ```text
//! ES7210 双麦 (L=MIC1, R=MIC2)
//!         ↓ I2S 16kHz / 16-bit / stereo (interleaved)
//!     Afe::feed(&stereo_chunk)              // chunk_size = feed_chunksize() × 2
//!         ↓ 内部线程跑 BSS+NS+VAD
//!     Afe::fetch() → FetchFrame { data: &[i16], vad_speech, vad_cache }
//!         ↓ data 是去噪后的单声道 i16,直接写 WAV
//! ```
//!
//! 线程模型:
//! - feed 和 fetch 用 `&self`(esp-sr 文档保证同 handle 两线程并发安全)
//! - 上层把 `Arc<Afe>` 分给 feed_thread + fetch_thread
//! - VAD 状态由 fetch 返回,fetch_thread 一手处理段开关 + WAV 写

use anyhow::{anyhow, Result};
use esp_idf_svc::sys::afe::{
    afe_config_free, afe_config_init, afe_config_t, afe_fetch_result_t,
    afe_mode_t_AFE_MODE_HIGH_PERF, afe_type_t_AFE_TYPE_SR, esp_afe_handle_from_config,
    esp_afe_sr_data_t, esp_afe_sr_iface_t, esp_srmodel_init, srmodel_list_t, vad_mode_t_VAD_MODE_2,
    vad_state_t_VAD_SPEECH,
};

/// AFE 输入格式:两路麦克风,无 playback reference,无 unknown 通道。
/// 对应 ES7210 SDOUT1 → I2S Philips stereo L/R。
const INPUT_FORMAT: &[u8] = b"MM\0";

/// 模型分区标签(没建分区时返回 NULL,这里只是给 srmodel 一个正常名字,
/// 不影响后续 NS/VAD 走 WebRTC 算法)。
const MODEL_PARTITION: &[u8] = b"model\0";

/// AFE 安全 wrapper。Drop 时自动 destroy。
///
/// `*const`/`*mut` 指针来自 esp-sr 内部静态/堆分配。esp-sr 自己保证 feed/fetch
/// 线程安全,这里把 wrapper 标 `Send + Sync` 让 `Arc<Afe>` 可以跨线程共享。
pub struct Afe {
    iface: *const esp_afe_sr_iface_t,
    data: *mut esp_afe_sr_data_t,
}

unsafe impl Send for Afe {}
unsafe impl Sync for Afe {}

/// 一次 fetch 的结果。借用 `&self`,下次 fetch 之前指针仍有效;
/// 调用方写 WAV 立即处理完即可,不要跨 fetch 持有。
pub struct FetchFrame<'a> {
    /// 去噪后的单声道 i16 PCM(`get_fetch_chunksize` 个 sample)
    pub data: &'a [i16],
    /// VAD trigger 之前缓存的音频(只有第一次 SPEECH 帧非空)。
    /// AFE 内置的预滚 buffer,用来补回"VAD 真正判定为 speech 之前"的 1-3 帧,
    /// 否则首字常被切。空切片 = 没缓存。
    pub vad_cache: &'a [i16],
    /// 当前帧是否人声
    pub vad_speech: bool,
}

impl Afe {
    /// 创建 AFE 实例。失败原因:内存不足 / 配置冲突 / esp-sr 内部 init 错。
    pub fn new() -> Result<Self> {
        // 1. srmodel:没分区返回 NULL,afe_config_init 收到 NULL 会让 NS/VAD 走 WebRTC
        //    (esp_afe_config.h 注释:`vad_model_name` 为 null 时使用 WebRTC VAD)
        let models: *mut srmodel_list_t =
            unsafe { esp_srmodel_init(MODEL_PARTITION.as_ptr() as *const _) };
        if models.is_null() {
            log::info!("AFE: no model partition (OK, will use WebRTC NS+VAD)");
        }

        // 2. 默认配置:input="MM"(2 mic 0 ref) + SR 类型 + HIGH_PERF
        //    AFE_TYPE_SR 比 _VC 更适合本场景:线性 NS + BSS + VAD,不带非线性 NS。
        let cfg: *mut afe_config_t = unsafe {
            afe_config_init(
                INPUT_FORMAT.as_ptr() as *const _,
                models,
                afe_type_t_AFE_TYPE_SR,
                afe_mode_t_AFE_MODE_HIGH_PERF,
            )
        };
        if cfg.is_null() {
            return Err(anyhow!("afe_config_init returned NULL"));
        }

        // 3. 微调:VAD 触发更宽松(MODE_2 = 中等敏感),min_speech 短一点能更早开段
        //    afe_ringbuf_size 加大:默认 2-3 帧,SPIFFS 偶尔卡顿就会丢帧。给 16 帧
        //    (~256ms 缓冲)能吃下偶发的 100ms 写盘抖动。
        unsafe {
            (*cfg).vad_mode = vad_mode_t_VAD_MODE_2;
            (*cfg).vad_min_speech_ms = 64; // ≥32,默认 128;短一点能录到"嗯"
            (*cfg).vad_min_noise_ms = 2000; // 静音 ≥2s 才算 silence;自然换气/思考常 1-2s,
                                            // 设短了一句话被切成 N 段
            (*cfg).vad_init = true;
            (*cfg).ns_init = true;
            (*cfg).se_init = true; // BSS 双麦波束
            (*cfg).agc_init = true;
            (*cfg).aec_init = false; // 没接喇叭,无 reference 通道
            (*cfg).wakenet_init = false; // 不要唤醒词
            (*cfg).afe_ringbuf_size = 16; // 抗 SPIFFS 写盘抖动
                                          // vad_model_name / ns_model_name / wakenet_model_name 留 NULL → WebRTC 实现
        }

        // 4. 取 iface + 创建 data
        let iface = unsafe { esp_afe_handle_from_config(cfg) };
        if iface.is_null() {
            unsafe { afe_config_free(cfg) };
            return Err(anyhow!("esp_afe_handle_from_config returned NULL"));
        }
        let create = unsafe { (*iface).create_from_config }
            .ok_or_else(|| anyhow!("iface.create_from_config is null"))?;
        let data = unsafe { create(cfg) };
        if data.is_null() {
            unsafe { afe_config_free(cfg) };
            return Err(anyhow!("AFE create_from_config returned NULL (OOM?)"));
        }

        // 5. config 已经被 create 吃进去了,可以释放
        unsafe { afe_config_free(cfg) };

        let afe = Afe { iface, data };
        log::info!(
            "AFE: ready (feed_chunk={} samples × {} ch, fetch_chunk={} samples, sr={} Hz)",
            afe.feed_chunksize(),
            afe.feed_channels(),
            afe.fetch_chunksize(),
            afe.sample_rate(),
        );
        // 让 esp-sr 自己打 pipeline 一行,看是不是 BSS+NS+VAD 都在
        if let Some(print) = unsafe { (*iface).print_pipeline } {
            unsafe { print(data) };
        }
        Ok(afe)
    }

    /// feed 时一次要传几个采样(每通道,不含通道数)。AFE_TYPE_SR/HIGH_PERF 通常 256(=16ms)。
    pub fn feed_chunksize(&self) -> usize {
        unsafe {
            let f = (*self.iface)
                .get_feed_chunksize
                .expect("iface.get_feed_chunksize");
            f(self.data) as usize
        }
    }

    /// fetch 出来的单通道音频长度(samples)。
    pub fn fetch_chunksize(&self) -> usize {
        unsafe {
            let f = (*self.iface)
                .get_fetch_chunksize
                .expect("iface.get_fetch_chunksize");
            f(self.data) as usize
        }
    }

    /// feed 期望的输入通道数(对 input="MM" 来说就是 2)。
    pub fn feed_channels(&self) -> usize {
        unsafe {
            let f = (*self.iface)
                .get_feed_channel_num
                .expect("iface.get_feed_channel_num");
            f(self.data) as usize
        }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe {
            let f = (*self.iface).get_samp_rate.expect("iface.get_samp_rate");
            f(self.data) as u32
        }
    }

    /// 喂入一帧 interleaved i16(L,R,L,R,...)。长度必须是 `feed_chunksize() * feed_channels()`。
    pub fn feed(&self, samples: &[i16]) -> Result<()> {
        let want = self.feed_chunksize() * self.feed_channels();
        if samples.len() != want {
            return Err(anyhow!(
                "AFE feed: bad length {} (want {})",
                samples.len(),
                want
            ));
        }
        let f = unsafe { (*self.iface).feed }.ok_or_else(|| anyhow!("iface.feed is null"))?;
        let ret = unsafe { f(self.data, samples.as_ptr()) };
        if ret < 0 {
            return Err(anyhow!("AFE feed returned {}", ret));
        }
        Ok(())
    }

    /// 阻塞 fetch 一帧(默认超时 2s,内部 ringbuf 空时阻塞)。
    /// 返回 None = ret_value 表示 ESP_FAIL。
    pub fn fetch(&self) -> Option<FetchFrame<'_>> {
        let f = unsafe { (*self.iface).fetch }.expect("iface.fetch");
        let r: *mut afe_fetch_result_t = unsafe { f(self.data) };
        if r.is_null() {
            return None;
        }
        let r_ref = unsafe { &*r };
        if r_ref.ret_value < 0 {
            return None;
        }
        // data_size 单位是字节,/2 才是 i16 个数
        let data_n = (r_ref.data_size as usize) / 2;
        let data: &[i16] = if !r_ref.data.is_null() && data_n > 0 {
            unsafe { core::slice::from_raw_parts(r_ref.data, data_n) }
        } else {
            &[]
        };
        let cache_n = (r_ref.vad_cache_size as usize) / 2;
        let vad_cache: &[i16] = if !r_ref.vad_cache.is_null() && cache_n > 0 {
            unsafe { core::slice::from_raw_parts(r_ref.vad_cache, cache_n) }
        } else {
            &[]
        };
        Some(FetchFrame {
            data,
            vad_cache,
            vad_speech: r_ref.vad_state == vad_state_t_VAD_SPEECH,
        })
    }
}

impl Drop for Afe {
    fn drop(&mut self) {
        if let Some(destroy) = unsafe { (*self.iface).destroy } {
            unsafe { destroy(self.data) };
        }
    }
}

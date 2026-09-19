//! 运行时配置
//!
//! NVS 持久化 + `Arc<RwLock<RuntimeConfig>>` 共享。fetcher / 主循环每轮读最新值,
//! 改动即生效(无需重启)。GET/POST /api/config 走这套。

use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

mod model;
pub mod patch;
pub use model::{clamp, RuntimeConfig};

pub type SharedConfig = Arc<RwLock<RuntimeConfig>>;

/// NVS 命名空间 "cfg"。Key 受限 15 字符,故用缩写。
pub struct ConfigStore {
    nvs: EspNvs<NvsDefault>,
}

// NVS key 一览(短名 ≤15 char)
const K_USER: &str = "gh_user";
const K_TOKEN: &str = "gh_token";
const K_GH_OK: &str = "gh_ok";
const K_GH_ERR: &str = "gh_err";
const K_MKT_URL: &str = "mkt_url";
const K_MKT_TOKEN: &str = "mkt_tok";
const K_MKT_SEC: &str = "mkt_sec";
const K_SREFR: &str = "srefr";
const K_ROT_ON: &str = "rot_on";
const K_ROT_SEC: &str = "rot_sec";
const K_T_OFF_CC: &str = "t_off_cc";
const K_H_OFF_CC: &str = "h_off_cc";
const K_TZ_OFF: &str = "tz_off";
const K_SF_N: &str = "sf_n";

impl ConfigStore {
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self> {
        let nvs = EspNvs::new(partition, "cfg", true).context("open NVS namespace 'cfg'")?;
        Ok(Self { nvs })
    }

    /// 把 NVS 里有的字段覆盖到 `base` 上返回。没写过的保留 base 默认。
    pub fn load(&self, mut base: RuntimeConfig) -> RuntimeConfig {
        let mut blob = vec![0u8; 4096];
        if let Ok(Some(bytes)) = self.nvs.get_blob("config_v1", &mut blob) {
            if let Ok(mut saved) = serde_json::from_slice::<RuntimeConfig>(bytes) {
                clamp(&mut saved);
                return saved;
            }
            log::error!("Invalid config_v1; recovering legacy configuration");
        }
        let mut buf = [0u8; 64];
        if let Ok(Some(s)) = self.nvs.get_str(K_USER, &mut buf) {
            if !s.is_empty() {
                base.gh_user = s.into();
            }
        }
        let mut buf = [0u8; 300];
        if let Ok(Some(s)) = self.nvs.get_str(K_TOKEN, &mut buf) {
            if !s.is_empty() {
                base.gh_token = s.into();
            }
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_GH_OK) {
            base.gh_refresh_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_GH_ERR) {
            base.gh_err_s = v;
        }
        let mut buf = [0u8; 192];
        if let Ok(Some(s)) = self.nvs.get_str(K_MKT_URL, &mut buf) {
            base.market_url = s.into();
        }
        let mut buf = [0u8; 128];
        if let Ok(Some(s)) = self.nvs.get_str(K_MKT_TOKEN, &mut buf) {
            base.market_token = s.into();
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_MKT_SEC) {
            base.market_refresh_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_SREFR) {
            base.sensor_refresh_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u8(K_ROT_ON) {
            base.auto_rotate = v != 0;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_ROT_SEC) {
            base.auto_rotate_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_i32(K_T_OFF_CC) {
            base.temp_off_c = v as f32 / 100.0;
        }
        if let Ok(Some(v)) = self.nvs.get_i32(K_H_OFF_CC) {
            base.humid_off_pct = v as f32 / 100.0;
        }
        if let Ok(Some(v)) = self.nvs.get_i32(K_TZ_OFF) {
            base.tz_off_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_SF_N) {
            base.splash_flash = v;
        }
        clamp(&mut base);
        base
    }

    pub fn save(&self, c: &RuntimeConfig) -> Result<()> {
        self.nvs.set_blob("config_v1", &serde_json::to_vec(c)?)?;
        log::info!("config saved to NVS");
        Ok(())
    }
}

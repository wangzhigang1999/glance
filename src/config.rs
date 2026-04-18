//! 运行时配置
//!
//! NVS 持久化 + `Arc<RwLock<RuntimeConfig>>` 共享。fetcher / 主循环每轮读最新值,
//! 改动即生效(无需重启)。GET/POST /api/config 走这套。

use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

pub type SharedConfig = Arc<RwLock<RuntimeConfig>>;

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    // --- GitHub 身份 ---
    pub gh_user: String,
    pub gh_token: String,
    // --- fetcher 周期 (秒) ---
    pub contrib_ok_s: u32,
    pub contrib_err_s: u32,
    pub activity_ok_s: u32,
    pub activity_err_s: u32,
    pub activity_stagger_s: u32,
    pub notif_s: u32,
    // --- 主循环 ---
    pub sensor_refresh_s: u32,
    pub auto_rotate: bool,
    pub auto_rotate_s: u32,
    // --- 传感器校准 ---
    pub temp_off_c: f32,
    pub humid_off_pct: f32,
    // --- 时区 ---
    pub tz_off_s: i32,
    // --- 开机屏幕除斑次数 (仅启动时读) ---
    pub splash_flash: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            gh_user: "wangzhigang1999".into(),
            gh_token: String::new(),
            contrib_ok_s: 300,
            contrib_err_s: 120,
            activity_ok_s: 180,
            activity_err_s: 120,
            activity_stagger_s: 8,
            notif_s: 180,
            sensor_refresh_s: 5,
            auto_rotate: false,
            auto_rotate_s: 15,
            temp_off_c: 0.0,
            humid_off_pct: 0.0,
            tz_off_s: 8 * 3600,
            splash_flash: 8,
        }
    }
}

/// NVS 命名空间 "cfg"。Key 受限 15 字符,故用缩写。
pub struct ConfigStore {
    nvs: EspNvs<NvsDefault>,
}

// NVS key 一览(短名 ≤15 char)
const K_USER: &str = "gh_user";
const K_TOKEN: &str = "gh_token";
const K_CINT_OK: &str = "cint_ok";
const K_CINT_ERR: &str = "cint_err";
const K_AINT_OK: &str = "aint_ok";
const K_AINT_ERR: &str = "aint_err";
const K_AINT_STG: &str = "aint_stg";
const K_NINT: &str = "nint";
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
        if let Ok(Some(v)) = self.nvs.get_u32(K_CINT_OK) {
            base.contrib_ok_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_CINT_ERR) {
            base.contrib_err_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_AINT_OK) {
            base.activity_ok_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_AINT_ERR) {
            base.activity_err_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_AINT_STG) {
            base.activity_stagger_s = v;
        }
        if let Ok(Some(v)) = self.nvs.get_u32(K_NINT) {
            base.notif_s = v;
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
        base
    }

    pub fn save(&self, c: &RuntimeConfig) -> Result<()> {
        let _ = self.nvs.set_str(K_USER, &c.gh_user)?;
        let _ = self.nvs.set_str(K_TOKEN, &c.gh_token)?;
        let _ = self.nvs.set_u32(K_CINT_OK, c.contrib_ok_s)?;
        let _ = self.nvs.set_u32(K_CINT_ERR, c.contrib_err_s)?;
        let _ = self.nvs.set_u32(K_AINT_OK, c.activity_ok_s)?;
        let _ = self.nvs.set_u32(K_AINT_ERR, c.activity_err_s)?;
        let _ = self.nvs.set_u32(K_AINT_STG, c.activity_stagger_s)?;
        let _ = self.nvs.set_u32(K_NINT, c.notif_s)?;
        let _ = self.nvs.set_u32(K_SREFR, c.sensor_refresh_s)?;
        let _ = self.nvs.set_u8(K_ROT_ON, c.auto_rotate as u8)?;
        let _ = self.nvs.set_u32(K_ROT_SEC, c.auto_rotate_s)?;
        let _ = self.nvs.set_i32(K_T_OFF_CC, (c.temp_off_c * 100.0) as i32)?;
        let _ = self.nvs.set_i32(K_H_OFF_CC, (c.humid_off_pct * 100.0) as i32)?;
        let _ = self.nvs.set_i32(K_TZ_OFF, c.tz_off_s)?;
        let _ = self.nvs.set_u32(K_SF_N, c.splash_flash)?;
        log::info!("config saved to NVS");
        Ok(())
    }
}

/// 合法化:夹取到合理区间,防止意外值把设备搞成不可用
pub fn clamp(c: &mut RuntimeConfig) {
    fn clip(v: &mut u32, lo: u32, hi: u32) {
        if *v < lo {
            *v = lo;
        } else if *v > hi {
            *v = hi;
        }
    }
    clip(&mut c.contrib_ok_s, 30, 24 * 3600);
    clip(&mut c.contrib_err_s, 30, 24 * 3600);
    clip(&mut c.activity_ok_s, 30, 24 * 3600);
    clip(&mut c.activity_err_s, 30, 24 * 3600);
    clip(&mut c.activity_stagger_s, 0, 600);
    clip(&mut c.notif_s, 30, 24 * 3600);
    clip(&mut c.sensor_refresh_s, 1, 3600);
    clip(&mut c.auto_rotate_s, 3, 3600);
    clip(&mut c.splash_flash, 0, 64);
    // 温/湿度偏移
    if c.temp_off_c < -20.0 {
        c.temp_off_c = -20.0;
    }
    if c.temp_off_c > 20.0 {
        c.temp_off_c = 20.0;
    }
    if c.humid_off_pct < -50.0 {
        c.humid_off_pct = -50.0;
    }
    if c.humid_off_pct > 50.0 {
        c.humid_off_pct = 50.0;
    }
    if c.tz_off_s < -14 * 3600 {
        c.tz_off_s = -14 * 3600;
    }
    if c.tz_off_s > 14 * 3600 {
        c.tz_off_s = 14 * 3600;
    }
    // 字符串长度保护
    if c.gh_user.len() > 39 {
        c.gh_user.truncate(39);
    }
    if c.gh_token.len() > 255 {
        c.gh_token.truncate(255);
    }
}

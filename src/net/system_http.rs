//! 硬件总览页 + JSON API
//!
//! `GET /system.html` → Tailwind 单页,5s 轮询 `/api/system` 刷新所有卡片
//! `GET /api/system`  → 当前 [`SystemSnapshot`] 的 JSON 序列化
//!
//! Snapshot 由主循环每个刷新 tick 写一次到 [`SharedSystem`] 共享变量,HTTP
//! handler 只读。这样 wifi / 传感器这些只能从主线程访问的状态也能透传出来。

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, RwLock,
};

use anyhow::Result;
use esp_idf_svc::{
    http::{server::EspHttpServer, Method},
    io::Write,
};
use serde::Serialize;

pub type SharedSystem = Arc<RwLock<SystemSnapshot>>;

/// 板子运行时的所有"看一眼就够"指标。新加字段直接往里塞,前端按字段名对应渲染。
#[derive(Default, Clone, Serialize)]
pub struct SystemSnapshot {
    // ---- identity ----
    pub fw: String,
    pub idf: String,
    pub mac: String,
    pub uptime_s: u64,
    pub reset_reason: String,
    pub sample_count: u32,

    // ---- memory ----
    pub heap_free: u32,
    pub heap_total: u32,
    pub heap_min: u32,
    pub psram_free: u32,
    pub psram_total: u32,
    pub stack_hwm: u32,

    // ---- flash ----
    pub flash_total: u32,
    pub app_size: u32,
    pub app_used: u32,
    pub app_part_addr: u32,

    // ---- storage(SD 或 SPIFFS,挂载点统一在 /storage)----
    pub storage_kind: String, // "sd" | "spiffs" | "none"
    pub storage_used: u64,
    pub storage_total: u64,
    pub recordings_count: u32,
    pub recordings_bytes: u64,

    // ---- sensors ----
    pub temp_c: Option<f32>,
    pub humid_pct: Option<f32>,
    pub chip_temp_c: Option<f32>,
    pub temp_off_c: f32,
    pub humid_off_pct: f32,

    // ---- power ----
    /// 仅 USB 拔下时电池数据有意义;USB 在的话 mv/pct 都是 None
    pub battery_mv: Option<u32>,
    pub battery_pct: Option<u8>,
    pub usb_plugged: bool,

    // ---- wifi ----
    pub wifi_connected: bool,
    pub wifi_ssid: String,
    pub wifi_ip: Option<[u8; 4]>,
    pub wifi_rssi: Option<i32>,

    // ---- clock ----
    pub unix_secs: Option<i64>,
    pub clock_source: String, // "rtc" | "sntp" | "—"
}

pub fn new_shared() -> SharedSystem {
    Arc::new(RwLock::new(SystemSnapshot::default()))
}

// ---- 时钟来源:全局原子,主流程在合适时点 set,snapshot 写入时读 ----
pub const CLK_NONE: u8 = 0;
pub const CLK_RTC: u8 = 1;
pub const CLK_SNTP: u8 = 2;
static CLOCK_SOURCE: AtomicU8 = AtomicU8::new(CLK_NONE);

pub fn set_clock_source(kind: u8) {
    CLOCK_SOURCE.store(kind, Ordering::Relaxed);
}

pub fn clock_source_str() -> &'static str {
    match CLOCK_SOURCE.load(Ordering::Relaxed) {
        CLK_RTC => "rtc",
        CLK_SNTP => "sntp",
        _ => "—",
    }
}

const SYSTEM_HTML: &str = include_str!("../../web/system.html");

pub fn register(server: &mut EspHttpServer<'static>, shared: SharedSystem) -> Result<()> {
    server.fn_handler(
        "/system.html",
        Method::Get,
        |req| -> Result<(), anyhow::Error> {
            let mut resp = req.into_ok_response()?;
            resp.write_all(SYSTEM_HTML.as_bytes())?;
            Ok(())
        },
    )?;

    let shared_for_get = shared.clone();
    server.fn_handler(
        "/api/system",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            // 持读锁的时间尽量短:克隆出来再 release,避免 HTTP 慢吞吞写网卡时阻塞主循环 write
            let snap = { shared_for_get.read().unwrap().clone() };
            let json = serde_json::to_string(&snap).unwrap_or_else(|_| "{}".into());
            let len = json.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;
    Ok(())
}

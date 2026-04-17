//! 温湿度计主程序(Phase 1:WiFi 硬编码 + NTP)
//!
//! 启动流程:
//! 1. Display 初始化 + 启屏自检
//! 2. SHTC3 初始化
//! 3. WiFi 连接(硬编码凭据,指数退避直到连上)
//! 4. 启动 SNTP(非阻塞,后台同步)
//! 5. 主循环:5s 读一次传感器 → 更新时钟/IP → 重绘
//!
//! 引脚见 `docs/10-pinout.md`。

mod display;
mod hw;
mod net;
mod ui;

use std::thread::sleep;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::AnyOutputPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::link_patches;

use crate::display::{Display, St7305};
use crate::hw::shtc3::Shtc3;
use crate::net::{format_local_hms, Sntp, WifiCreds, WifiManager};
use crate::ui::AppState;

// Phase 1:凭据硬编码;Phase 2 从 NVS 读;Phase 3 BLE 写 NVS
const WIFI_SSID: &str = "CU_2089";
const WIFI_PASS: &str = "24457k55";

/// 中国时区 UTC+8
const TZ_OFFSET_SECS: i64 = 8 * 3600;

fn main() -> anyhow::Result<()> {
    link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("=== ESP32-S3-RLCD-4.2 温湿度计 (WiFi+SNTP) ===");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ---- Display ----
    log::info!("Init ST7305 display");
    let st7305 = St7305::new(
        peripherals.spi2,
        peripherals.pins.gpio12, // MOSI / SDA
        peripherals.pins.gpio11, // SCK / SCL
        peripherals.pins.gpio40, // CS
        AnyOutputPin::from(peripherals.pins.gpio5),  // DC
        AnyOutputPin::from(peripherals.pins.gpio41), // RST
    )?;
    let mut display = Display::new(st7305);
    display.init()?;
    log::info!("Display ready (400x300 landscape)");
    display.splash_flash(2)?;

    let mut state = AppState::default();
    let _ = ui::render(&mut display, &state);
    display.flush()?;

    // ---- SHTC3 ----
    log::info!("Init SHTC3 sensor (I2C SDA=13 SCL=14)");
    let mut sensor = Shtc3::new(
        peripherals.i2c0,
        peripherals.pins.gpio13,
        peripherals.pins.gpio14,
    )?;

    // ---- WiFi ----
    log::info!("Init WiFi, target SSID={}", WIFI_SSID);
    let creds = WifiCreds::new(WIFI_SSID, WIFI_PASS)?;
    let mut wifi = WifiManager::new(peripherals.modem, sys_loop, nvs)?;
    let ip_info = wifi.connect_with_backoff(&creds);
    state.wifi_connected = true;
    state.ip_octets = Some(ip_info.ip.octets());
    let _ = ui::render(&mut display, &state);
    display.flush()?;

    // ---- SNTP(非阻塞,后台同步) ----
    let sntp = Sntp::start()?;
    // 给 SNTP 最多 10s 首次同步;失败不 panic,后续 tick 里会继续显示"--:--:--"
    if sntp.wait_synced(Duration::from_secs(10)) {
        state.clock_hms = format_local_hms(TZ_OFFSET_SECS);
        log::info!("Time synced: {:?}", state.clock_hms);
    }

    // ---- 主循环 ----
    let boot = Instant::now();
    let mut n: u32 = 0;
    loop {
        n = n.saturating_add(1);
        match sensor.read() {
            Ok((t, rh)) => {
                state.temperature_c = Some(t);
                state.humidity_pct = Some(rh);
                log::info!("#{n} T={t:.2}°C RH={rh:.2}%");
            }
            Err(e) => {
                log::error!("SHTC3 read failed: {e}");
                state.temperature_c = None;
                state.humidity_pct = None;
            }
        }

        state.uptime_secs = boot.elapsed().as_secs();
        state.sample_count = n;
        state.wifi_connected = wifi.is_connected();
        state.ip_octets = wifi.ip_info().map(|i| i.ip.octets());
        state.clock_hms = if sntp.is_synced() {
            format_local_hms(TZ_OFFSET_SECS)
        } else {
            None
        };

        let _ = ui::render(&mut display, &state);
        if let Err(e) = display.flush() {
            log::error!("Display flush failed: {e}");
        }

        sleep(Duration::from_secs(5));
    }
}

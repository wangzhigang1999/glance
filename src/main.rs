//! 温湿度计主程序
//!
//! 启动流程:
//! 1. Display 初始化 + 启屏自检 + SHTC3 初始化
//! 2. NVS 读 WiFi 凭据
//!    - 有凭据:直接连 WiFi(3 次失败回退到 BLE 配网)
//!    - 没凭据:启动 BLE 配网,阻塞等手机写入
//! 3. 配网成功:deinit BLE 释放 ~30KB,启动 SNTP
//! 4. 主循环:5 s 读一次传感器,刷 UI
//!
//! 配网(首次/清 NVS 后):
//! - 手机装 nRF Connect,扫到 "RLCD-Thermo",连接
//! - 写 SSID 特征(524c4344-...-001),写 PASSWORD(...-002),写 COMMIT=0x01(...-003)
//! - 订阅 STATUS(...-004)的 notify 观察连接过程

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
use crate::net::{
    format_local_hms, CredsStore, ProvStatus, Provisioner, Sntp, WifiCreds, WifiManager,
};
use crate::ui::AppState;

/// 中国时区 UTC+8
const TZ_OFFSET_SECS: i64 = 8 * 3600;

/// 首次启动/NVS 空时,BLE 广播名
const BLE_DEVICE_NAME: &str = "RLCD-Thermo";

/// 保存的凭据连接失败多少次后回退到 BLE 配网(凭据可能过期)
const WIFI_RETRY_BUDGET: u32 = 3;

fn main() -> anyhow::Result<()> {
    link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("=== ESP32-S3-RLCD-4.2 温湿度计 ===");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ---- Display ----
    log::info!("Init ST7305 display");
    let st7305 = St7305::new(
        peripherals.spi2,
        peripherals.pins.gpio12,
        peripherals.pins.gpio11,
        peripherals.pins.gpio40,
        AnyOutputPin::from(peripherals.pins.gpio5),
        AnyOutputPin::from(peripherals.pins.gpio41),
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

    // ---- NVS creds ----
    let creds_store = CredsStore::new(nvs.clone())?;
    let stored_creds = creds_store.load()?;

    // ---- WiFi manager ----
    let mut wifi = WifiManager::new(peripherals.modem, sys_loop, nvs)?;

    // ---- 连接策略:优先用 NVS 凭据,失败则回退 BLE 配网 ----
    let creds = obtain_creds(
        &mut display,
        &mut state,
        &mut wifi,
        &creds_store,
        stored_creds,
    )?;

    // 到这里一定连上了 WiFi
    let ip_info = wifi
        .ip_info()
        .ok_or_else(|| anyhow::anyhow!("wifi up but no IP"))?;
    state.wifi_connected = true;
    state.ip_octets = Some(ip_info.ip.octets());
    state.prov_mode = false;
    let _ = ui::render(&mut display, &state);
    display.flush()?;
    log::info!("WiFi fully up, ssid={}, ip={}", creds.ssid, ip_info.ip);

    // ---- SNTP(非阻塞,后台同步) ----
    // _sntp 必须保持在作用域内,否则 drop 会 sntp_stop() 终结对时
    let _sntp = Sntp::start()?;
    // 这里不阻塞,主循环里每次直接看 SystemTime 是否 > 2020 来判"是否已同步"
    // 理由:sntp_get_sync_status 在 poll 周期内会从 COMPLETED 翻回 IN_PROGRESS,不稳定

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
        state.clock_hms = format_local_hms(TZ_OFFSET_SECS);

        let _ = ui::render(&mut display, &state);
        if let Err(e) = display.flush() {
            log::error!("Display flush failed: {e}");
        }

        sleep(Duration::from_secs(5));
    }
}

/// 拿到可连的 WiFi 凭据并完成连接。
/// 优先级:NVS → 失败回退 BLE 配网。成功时 BLE 会被 deinit(如果启动过)。
fn obtain_creds(
    display: &mut Display<'_>,
    state: &mut AppState,
    wifi: &mut WifiManager,
    store: &CredsStore,
    stored: Option<WifiCreds>,
) -> anyhow::Result<WifiCreds> {
    // 尝试 1:NVS 里的凭据
    if let Some(creds) = stored {
        log::info!("found stored creds for ssid={}, trying to connect", creds.ssid);
        state.prov_mode = false;
        state.prov_hint.clear();
        let _ = state.prov_hint.push_str("connecting...");
        let _ = ui::render(display, state);
        let _ = display.flush();

        if try_connect_n(wifi, &creds, WIFI_RETRY_BUDGET) {
            return Ok(creds);
        }
        log::warn!("stored creds failed {WIFI_RETRY_BUDGET}x, falling back to BLE prov");
    }

    // 尝试 2:BLE 配网,阻塞到拿到一组能连上的凭据
    state.prov_mode = true;
    state.prov_hint.clear();
    let _ = state.prov_hint.push_str(BLE_DEVICE_NAME);
    let _ = ui::render(display, state);
    let _ = display.flush();

    let prov = Provisioner::start(BLE_DEVICE_NAME)?;

    loop {
        // 没超时(无限等),手机慢慢来
        let creds = prov
            .wait_for_creds(Duration::from_secs(3600))
            .ok_or_else(|| anyhow::anyhow!("BLE prov idle > 1h, giving up"))?;

        prov.publish_status(ProvStatus::Connecting);
        state.prov_hint.clear();
        let _ = write_owned(&mut state.prov_hint, "connecting ", &creds.ssid);
        let _ = ui::render(display, state);
        let _ = display.flush();

        if try_connect_n(wifi, &creds, WIFI_RETRY_BUDGET) {
            prov.publish_status(ProvStatus::Connected);
            // 先保存 NVS(下次启动就不走 BLE 了),再 deinit BLE
            if let Err(e) = store.save(&creds) {
                log::error!("save creds to NVS failed: {e}");
            }
            // 给 STATUS notify 点时间发出去再关 BLE
            sleep(Duration::from_millis(500));
            if let Err(e) = Provisioner::shutdown() {
                log::warn!("BLE shutdown returned: {e}");
            }
            return Ok(creds);
        }

        log::warn!("connect failed with provisioned creds, wait for new");
        prov.publish_status(ProvStatus::Failed);
        state.prov_hint.clear();
        let _ = state.prov_hint.push_str("bad creds, retry");
        let _ = ui::render(display, state);
        let _ = display.flush();
    }
}

fn try_connect_n(wifi: &mut WifiManager, creds: &WifiCreds, budget: u32) -> bool {
    for attempt in 1..=budget {
        match wifi.connect(creds) {
            Ok(_) => return true,
            Err(e) => {
                log::warn!("connect attempt {attempt}/{budget} failed: {e:#}");
                // 为下一次 set_configuration 留干净状态
                let _ = wifi.force_stop();
                sleep(Duration::from_millis(500 * attempt as u64));
            }
        }
    }
    false
}

fn write_owned(
    out: &mut heapless::String<32>,
    prefix: &str,
    suffix: &heapless::String<32>,
) -> core::fmt::Result {
    use core::fmt::Write;
    write!(out, "{prefix}{suffix}")
}

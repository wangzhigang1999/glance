//! 温湿度计主程序
//!
//! 启动流程:
//! 1. Display 初始化 + 启屏自检 + SHTC3 初始化
//! 2. NVS 读 WiFi 凭据
//!    - 有凭据:直接连 WiFi(3 次失败回退到 SoftAP 配网)
//!    - 没凭据:开 SoftAP + HTTP 门户,阻塞等手机提交
//! 3. 配网成功:停 AP,STA 连家里 WiFi,启动 SNTP
//! 4. 主循环:5 s 读一次传感器,刷 UI
//!
//! 配网(首次/清 NVS 后):
//! - 手机连 "RLCD-Setup"(open,无密码)
//! - 浏览器打开 http://192.168.4.1,填表单
//! - 提交后设备试连,失败会重开 AP 让你再填

mod config;
mod display;
mod hw;
mod net;
mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::AnyOutputPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::link_patches;

use esp_idf_svc::hal::gpio::AnyIOPin;

use crate::display::{Display, St7305};
use crate::hw::battery::{Battery, PowerSource};
use crate::hw::button::Button;
use crate::hw::chip_temp::ChipTemp;
use crate::hw::shtc3::Shtc3;
use crate::hw::system::{mac_suffix, read_flash_stats, read_sys_stats};
use crate::net::activity::{self, Activity};
use crate::net::github::{self, ContribData};
use crate::net::notifications::{self, NotifSummary};
use crate::net::screen_http;
use crate::config::{ConfigStore, RuntimeConfig, SharedConfig};
use crate::net::{
    format_local_date, format_local_hms, CredsStore, Provisioner, Sntp, WifiCreds, WifiManager,
};
use crate::ui::{AppState, Page};

const FW_VERSION: &str = env!("CARGO_PKG_VERSION");
// ESP_IDF_VERSION 从 .cargo/config.toml [env] 注入
const IDF_VERSION: &str = env!("ESP_IDF_VERSION");
// 编译时注入的 GitHub PAT(仅首次 seed 进 NVS);运行时以 NVS 里的 gh_token 为准
const GITHUB_TOKEN_COMPILE: Option<&'static str> = option_env!("GITHUB_TOKEN");

/// 首次启动/NVS 空时,SoftAP 的 SSID(open,无密码)
const AP_SSID: &str = "RLCD-Setup";

/// 保存的凭据连接失败多少次后回退到 SoftAP 配网(凭据可能过期)
const WIFI_RETRY_BUDGET: u32 = 3;

fn main() -> anyhow::Result<()> {
    link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("=== ESP32-S3-RLCD-4.2 温湿度计 ===");

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // ---- Runtime config(NVS + SharedConfig);开机就得到,splash_flash 等要用 ----
    let config_store = Arc::new(ConfigStore::new(nvs.clone())?);
    let mut base = RuntimeConfig::default();
    // 一次性 seed:编译时 env 有 token 就作为初值(会被 NVS 覆盖,空 NVS 时生效)
    if let Some(tok) = GITHUB_TOKEN_COMPILE {
        if !tok.is_empty() {
            base.gh_token = tok.into();
        }
    }
    let loaded = config_store.load(base);
    log::info!(
        "Config loaded: user={} token={} contrib={}/{} act={}/{} notif={} refresh={} tz={}",
        loaded.gh_user,
        if loaded.gh_token.is_empty() { "(none)" } else { "(set)" },
        loaded.contrib_ok_s,
        loaded.contrib_err_s,
        loaded.activity_ok_s,
        loaded.activity_err_s,
        loaded.notif_s,
        loaded.sensor_refresh_s,
        loaded.tz_off_s,
    );
    // NVS 首次写入:把 seeded 值落盘(省得下次 seed 又跑)
    if let Err(e) = config_store.save(&loaded) {
        log::warn!("initial config save failed: {e:#}");
    }
    let splash_n = loaded.splash_flash;
    let config: SharedConfig = Arc::new(RwLock::new(loaded));

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
    // 开机除斑:N 次黑-白翻转,消除反射式 LCD 的液晶分子残影/黑斑
    display.splash_flash(splash_n as u32)?;

    let mut state = AppState::default();
    let _ = ui::render(&mut display, &state, Page::Dashboard);
    display.flush()?;

    // ---- SHTC3 ----
    log::info!("Init SHTC3 sensor (I2C SDA=13 SCL=14)");
    let mut sensor = Shtc3::new(
        peripherals.i2c0,
        peripherals.pins.gpio13,
        peripherals.pins.gpio14,
    )?;

    // ---- Battery ADC(GPIO4)----
    log::info!("Init battery ADC on GPIO4");
    let mut battery = Battery::new(peripherals.adc1, peripherals.pins.gpio4)?;

    // ---- 按钮:BOOT(GPIO0) + KEY(GPIO18),任一按下都切页 ----
    // 板上三键:BOOT / KEY / PWR(独立电源IC,长按关机)
    log::info!("Init buttons: BOOT=GPIO0, KEY=GPIO18");
    let mut btn_boot = Button::new(AnyIOPin::from(peripherals.pins.gpio0))?;
    let mut btn_key = Button::new(AnyIOPin::from(peripherals.pins.gpio18))?;

    // ---- 芯片内置温度传感器 ----
    log::info!("Init chip internal temp sensor");
    let chip_temp = ChipTemp::new().ok();
    if chip_temp.is_none() {
        log::warn!("chip temp sensor init failed, will show N/A");
    }

    // ---- 静态身份信息(一次性算,进 state) ----
    let fw_version: &'static str = FW_VERSION;
    let idf_version: &'static str = IDF_VERSION.strip_prefix('v').unwrap_or(IDF_VERSION);
    let mac = mac_suffix();
    let flash_stats = read_flash_stats();
    log::info!(
        "Flash: chip={}MB part@0x{:x} size={}KB app_img={}KB",
        flash_stats.flash_total / 1024 / 1024,
        flash_stats.app_part_addr,
        flash_stats.app_part_size / 1024,
        flash_stats.app_used / 1024,
    );
    state.flash_total = flash_stats.flash_total;
    state.app_part_size = flash_stats.app_part_size;
    state.app_used = flash_stats.app_used;
    // mac 提前填给 state,配网页(render_prov)要显示
    state.fw_version = fw_version;
    state.idf_version = idf_version;
    state.mac_suffix = mac.clone();

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
    state.wifi_ssid = creds.ssid.clone();
    state.ip_octets = Some(ip_info.ip.octets());
    state.prov_mode = false;
    let _ = ui::render(&mut display, &state, Page::Dashboard);
    display.flush()?;
    log::info!("WiFi fully up, ssid={}, ip={}", creds.ssid, ip_info.ip);

    // ---- SNTP(非阻塞,后台同步) ----
    // _sntp 必须保持在作用域内,否则 drop 会 sntp_stop() 终结对时
    let _sntp = Sntp::start()?;

    // ---- GitHub 贡献数据:GraphQL(每轮从 config 读 user/token/间隔) ----
    let contrib_shared: Arc<Mutex<Option<ContribData>>> = Arc::new(Mutex::new(None));
    let contrib_err_shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    github::spawn_fetcher(
        config.clone(),
        contrib_shared.clone(),
        contrib_err_shared.clone(),
    );

    // ---- 屏幕镜像 HTTP 服务 + 运行时配置 API ----
    let screen_shared = screen_http::new_shared_fb();
    let http_next_flag = Arc::new(AtomicBool::new(false));
    let _screen_server = match screen_http::start(
        screen_shared.clone(),
        http_next_flag.clone(),
        config.clone(),
        config_store.clone(),
    ) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("Screen HTTP server failed to start: {e:#}");
            None
        }
    };

    // ---- GitHub Notifications + Activity(同样走 config) ----
    let notif_shared: Arc<Mutex<Option<NotifSummary>>> = Arc::new(Mutex::new(None));
    let activity_shared: Arc<Mutex<Option<Activity>>> = Arc::new(Mutex::new(None));
    let activity_err_shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    notifications::spawn_fetcher(config.clone(), notif_shared.clone());
    activity::spawn_fetcher(
        config.clone(),
        activity_shared.clone(),
        activity_err_shared.clone(),
    );
    log::info!("GH fetchers: contribution + notifications + activity spawned");
    // 这里不阻塞,主循环里每次直接看 SystemTime 是否 > 2020 来判"是否已同步"
    // 理由:sntp_get_sync_status 在 poll 周期内会从 COMPLETED 翻回 IN_PROGRESS,不稳定

    // ---- 主循环 ----
    // 100ms tick:按钮响应;重活(传感器+重绘)按 config.sensor_refresh_s 节奏跑。
    let boot = Instant::now();
    let mut n: u32 = 0;
    let mut page = Page::Dashboard;
    let mut last_refresh = Instant::now() - Duration::from_secs(60);
    let mut last_rotate = Instant::now();
    let tick = Duration::from_millis(100);

    loop {
        // 每轮从 config 读一次热重载值
        let (refresh_period, auto_rotate, auto_rotate_period, tz_off, t_off, h_off, cfg_user) = {
            let c = config.read().unwrap();
            (
                Duration::from_secs(c.sensor_refresh_s as u64),
                c.auto_rotate,
                Duration::from_secs(c.auto_rotate_s as u64),
                c.tz_off_s as i64,
                c.temp_off_c,
                c.humid_off_pct,
                c.gh_user.clone(),
            )
        };

        // 扫两个按钮 + HTTP /next + 自动翻页,任一触发切页
        let boot_edge = btn_boot.poll_pressed();
        let key_edge = btn_key.poll_pressed();
        let http_next = http_next_flag.swap(false, Ordering::Relaxed);
        let auto_due = auto_rotate && last_rotate.elapsed() >= auto_rotate_period;
        let page_changed = if boot_edge || key_edge || http_next || auto_due {
            page = page.next();
            last_rotate = Instant::now();
            log::info!(
                "Page switch (BOOT={} KEY={} HTTP={} AUTO={}) -> {:?}",
                boot_edge, key_edge, http_next, auto_due, page
            );
            true
        } else {
            false
        };

        let due = last_refresh.elapsed() >= refresh_period;
        if page_changed || due {
            if due {
                n = n.saturating_add(1);
                match sensor.read() {
                    Ok((t_raw, rh_raw)) => {
                        let t = t_raw + t_off;
                        let rh = (rh_raw + h_off).clamp(0.0, 100.0);
                        state.temperature_c = Some(t);
                        state.humidity_pct = Some(rh);
                        state.temp_hist.write(t);
                        state.rh_hist.write(rh);
                        log::info!(
                            "#{n} T={t:.2}°C (raw={t_raw:.2}, off={t_off:+.2}) RH={rh:.2}% (raw={rh_raw:.2}, off={h_off:+.2})"
                        );
                    }
                    Err(e) => {
                        log::error!("SHTC3 read failed: {e}");
                        state.temperature_c = None;
                        state.humidity_pct = None;
                    }
                }
            }

            state.uptime_secs = boot.elapsed().as_secs();
            state.sample_count = n;
            state.wifi_connected = wifi.is_connected();
            state.ip_octets = wifi.ip_info().map(|i| i.ip.octets());
            state.rssi = wifi.rssi();
            state.clock_hm = format_local_hms(tz_off).map(|s| {
                let mut out: heapless::String<8> = heapless::String::new();
                let _ = out.push_str(&s[..5.min(s.len())]);
                out
            });
            state.clock_date = format_local_date(tz_off);

            // 同步 GitHub 用户名到 state(供 UI 渲染)
            state.gh_user.clear();
            for ch in cfg_user.chars().take(40) {
                if state.gh_user.push(ch).is_err() {
                    break;
                }
            }
            state.battery = match battery.read() {
                Ok(PowerSource::Battery { mv, percent }) => Some((mv, percent)),
                Ok(PowerSource::Usb) => None,
                Err(e) => {
                    log::warn!("battery read failed: {e}");
                    None
                }
            };
            let sys = read_sys_stats();
            state.heap_free = sys.heap_free as u32;
            state.heap_total = sys.heap_total as u32;
            state.heap_min_ever = sys.heap_min_ever as u32;
            state.psram_free = sys.psram_free as u32;
            state.psram_total = sys.psram_total as u32;
            state.stack_hwm_bytes = sys.main_stack_hwm_bytes;
            state.reset_reason = sys.reset_reason;
            state.chip_temp_c = chip_temp.as_ref().and_then(|c| c.read_celsius());

            // GitHub 贡献:从共享 Arc 复制到 state
            if let Ok(g) = contrib_shared.lock() {
                if let Some(data) = g.as_ref() {
                    // 取最新的 ≤371 天
                    let take_n = data.levels.len().min(state.contrib.len());
                    let skip = data.levels.len().saturating_sub(take_n);
                    for (i, lvl) in data.levels.iter().skip(skip).take(take_n).enumerate() {
                        state.contrib[i] = *lvl;
                    }
                    // counts 同样对齐:长度应与 levels 一致;若不一致按较短者取
                    let c_take = data.counts.len().min(take_n);
                    let c_skip = data.counts.len().saturating_sub(c_take);
                    for (i, n) in data.counts.iter().skip(c_skip).take(c_take).enumerate() {
                        state.contrib_counts[i] = (*n).min(u16::MAX as u32) as u16;
                    }
                    state.contrib_weeks = ((take_n + 6) / 7) as u16;
                    state.contrib_days = take_n as u16;
                    state.contrib_valid = take_n > 0;
                    state.contrib_total_year = data.total_year;
                }
            }
            // 贡献错误信息(若有)
            if let Ok(e) = contrib_err_shared.lock() {
                state.contrib_error.clear();
                for c in e.chars() {
                    if state.contrib_error.push(c).is_err() {
                        break;
                    }
                }
            }

            // GitHub Notifications:复制到 state(只取最新一条)
            if let Ok(g) = notif_shared.lock() {
                if let Some(s) = g.as_ref() {
                    state.notif_count = s.count as u32;
                    state.notif_top_title.clear();
                    state.notif_top_repo.clear();
                    if let Some(first) = s.items.first() {
                        for c in first.title.chars() {
                            if state.notif_top_title.push(c).is_err() {
                                break;
                            }
                        }
                        for c in first.repo.chars() {
                            if state.notif_top_repo.push(c).is_err() {
                                break;
                            }
                        }
                    }
                    state.notif_valid = true;
                }
            }

            // GitHub Activity:复制到 state
            if let Ok(g) = activity_shared.lock() {
                if let Some(a) = g.as_ref() {
                    state.last_event_line.clear();
                    if let Some(l) = a.last_line.as_deref() {
                        for c in l.chars() {
                            if state.last_event_line.push(c).is_err() {
                                break;
                            }
                        }
                    }
                    state.last_event_detail.clear();
                    if let Some(d) = a.last_detail.as_deref() {
                        for c in d.chars() {
                            if state.last_event_detail.push(c).is_err() {
                                break;
                            }
                        }
                    }
                    state.open_prs = a.open_prs;
                    state.last_event_at_epoch = a.last_at_epoch;
                    state.activity_valid = true;
                }
            }
            if let Ok(e) = activity_err_shared.lock() {
                state.activity_error.clear();
                for c in e.chars() {
                    if state.activity_error.push(c).is_err() {
                        break;
                    }
                }
            }

            let _ = ui::render(&mut display, &state, page);
            if let Err(e) = display.flush() {
                log::error!("Display flush failed: {e}");
            }
            // 镜像当前 fb 到共享 buffer 供 HTTP 镜屏读
            if let Ok(mut guard) = screen_shared.lock() {
                guard.clear();
                guard.extend_from_slice(display.fb_raw());
            }
            last_refresh = Instant::now();
        }

        sleep(tick);
    }
}

/// 拿到可连的 WiFi 凭据并完成连接。
/// 优先级:NVS → 失败回退 SoftAP + HTTP 门户。
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
        let _ = ui::render(display, state, Page::Dashboard);
        let _ = display.flush();

        if try_connect_n(wifi, &creds, WIFI_RETRY_BUDGET) {
            return Ok(creds);
        }
        log::warn!("stored creds failed {WIFI_RETRY_BUDGET}x, falling back to SoftAP prov");
        let _ = wifi.force_stop();
    }

    // 尝试 2:SoftAP + HTTP 门户,阻塞到拿到一组能连上的凭据
    loop {
        state.prov_mode = true;
        state.prov_hint.clear();
        let _ = state.prov_hint.push_str(AP_SSID);
        state.prov_ap_ip = None;
        let _ = ui::render(display, state, Page::Dashboard);
        let _ = display.flush();

        wifi.start_ap(AP_SSID)?;
        // AP 启好再读 IP:DHCP server 把 192.168.x.1 赋给 ap netif 后才能查到
        state.prov_ap_ip = wifi.ap_ip().map(|ip| ip.octets());
        let _ = ui::render(display, state, Page::Dashboard);
        let _ = display.flush();
        let prov = Provisioner::start(AP_SSID, &state.mac_suffix)?;

        // 无限等(手机慢慢来);空表单/非法字符会被 wait_for_creds 滤掉返回 None,继续等
        let creds = loop {
            match prov.wait_for_creds(Duration::from_secs(3600)) {
                Some(c) => break c,
                None => continue,
            }
        };

        // 拆 AP + HTTP 才能切 STA,prov 也一并 drop
        drop(prov);
        let _ = wifi.force_stop();

        state.prov_hint.clear();
        let _ = write_owned(&mut state.prov_hint, "connecting ", &creds.ssid);
        let _ = ui::render(display, state, Page::Dashboard);
        let _ = display.flush();

        if try_connect_n(wifi, &creds, WIFI_RETRY_BUDGET) {
            if let Err(e) = store.save(&creds) {
                log::error!("save creds to NVS failed: {e}");
            }
            return Ok(creds);
        }

        log::warn!("connect failed with submitted creds, restart AP for retry");
        let _ = wifi.force_stop();
        // 循环顶端会重开 AP 和 Provisioner
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

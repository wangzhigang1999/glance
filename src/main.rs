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
//! - 手机连 "CuriosityLab-Setup"(open,无密码)
//! - 浏览器打开 http://192.168.4.1,填表单
//! - 提交后设备试连,失败会重开 AP 让你再填

mod config;
mod display;
mod hw;
mod net;
mod ui;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread::sleep,
    time::{Duration, Instant},
};

use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        gpio::{AnyIOPin, AnyOutputPin},
        i2c::{I2cConfig, I2cDriver},
        peripherals::Peripherals,
        units::Hertz,
    },
    nvs::EspDefaultNvsPartition,
    sys::link_patches,
};

use crate::{
    config::{ConfigStore, RuntimeConfig, SharedConfig},
    display::{Display, St7305},
    hw::{
        battery::{Battery, PowerSource},
        button::Button,
        chip_temp::ChipTemp,
        es7210::Es7210,
        mic::Mic,
        shtc3::Shtc3,
        system::{mac_suffix, read_flash_stats, read_sys_stats},
        I2cBus,
    },
    net::{
        activity::Activity, format_local_date, format_local_hms, gh_worker, github::ContribData,
        log_sink, notifications::NotifSummary, screen_http, CredsStore, Provisioner, Sntp,
        WifiCreds, WifiManager,
    },
    ui::{AppState, Page},
};

const FW_VERSION: &str = env!("CARGO_PKG_VERSION");
// ESP_IDF_VERSION 从 .cargo/config.toml [env] 注入
const IDF_VERSION: &str = env!("ESP_IDF_VERSION");
// 编译时注入的 GitHub PAT(仅首次 seed 进 NVS);运行时以 NVS 里的 gh_token 为准
const GITHUB_TOKEN_COMPILE: Option<&'static str> = option_env!("GITHUB_TOKEN");

/// 首次启动/NVS 空时,SoftAP 的 SSID(open,无密码)
const AP_SSID: &str = "CuriosityLab-Setup";

/// 保存的凭据连接失败多少次后回退到 SoftAP 配网(凭据可能过期)
const WIFI_RETRY_BUDGET: u32 = 3;

fn main() -> anyhow::Result<()> {
    link_patches();
    let log_hub = log_sink::install();

    log::info!("=== ESP32-S3-RLCD-4.2 ===");

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
        "Config loaded: user={} token={} gh={}/{} refresh={} tz={}",
        loaded.gh_user,
        if loaded.gh_token.is_empty() {
            "(none)"
        } else {
            "(set)"
        },
        loaded.gh_refresh_s,
        loaded.gh_err_s,
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

    // ---- 共享 I2C0(SDA13/SCL14):SHTC3 / ES7210 / ES8311 / PCF85063 都挂这条 ----
    log::info!("Init shared I2C0 bus (SDA=13 SCL=14 @100kHz)");
    let i2c_cfg = I2cConfig::new().baudrate(Hertz(100_000));
    let i2c_drv = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio13,
        peripherals.pins.gpio14,
        &i2c_cfg,
    )?;
    let i2c_bus: I2cBus = Arc::new(Mutex::new(i2c_drv));

    // ---- SHTC3 ----
    log::info!("Init SHTC3 sensor (addr 0x70)");
    let mut sensor = Shtc3::new(i2c_bus.clone());

    // ---- ES7210 + I2S MIC(骨架:MIC1 / 16kHz / 16-bit / mono)----
    // 等 A3V3 稳了再发 I2C,免得首次写寄存器 NACK
    sleep(Duration::from_millis(50));
    let mut mic_codec = Es7210::new(i2c_bus.clone());
    if let Err(e) = mic_codec.open_mic1() {
        log::warn!("ES7210 open failed (mic skeleton disabled): {e:#}");
    } else {
        match Mic::new(
            peripherals.i2s0,
            peripherals.pins.gpio16,
            peripherals.pins.gpio9,
            peripherals.pins.gpio45,
            peripherals.pins.gpio10,
        ) {
            Ok(mut mic) => match mic.start() {
                Err(e) => log::warn!("Mic start failed: {e:#}"),
                Ok(()) => {
                    // 给 I2S MCLK 几个 ms 稳定后再让 ES7210 上电模拟通路
                    sleep(Duration::from_millis(10));
                    if let Err(e) = mic_codec.enable() {
                        log::warn!("ES7210 enable failed (analog path off): {e:#}");
                    }
                    std::thread::Builder::new()
                        .name("mic_rms".into())
                        // 8K:loop 里 log::info!("MIC RMS=...") 会过 log_sink 的 format!,
                        // fmt 机器一把吃 1-2KB,4K 顶不住会爆栈
                        .stack_size(8192)
                        .spawn(move || mic_rms_loop(mic))
                        .map(|_| ())
                        .unwrap_or_else(|e| log::warn!("spawn mic_rms thread failed: {e:#}"));
                }
            },
            Err(e) => log::warn!("Mic init failed: {e:#}"),
        }
    }

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

    // ---- NVS creds(多 slot,最多 4 个) ----
    let creds_store = Arc::new(CredsStore::new(nvs.clone())?);
    let stored_creds = creds_store.load_all()?;
    log::info!("loaded {} stored wifi cred(s)", stored_creds.len());

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

    // ---- GitHub 共享状态(contrib / notif / activity) ----
    let contrib_shared: Arc<Mutex<Option<ContribData>>> = Arc::new(Mutex::new(None));
    let contrib_err_shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let notif_shared: Arc<Mutex<Option<NotifSummary>>> = Arc::new(Mutex::new(None));
    let activity_shared: Arc<Mutex<Option<Activity>>> = Arc::new(Mutex::new(None));
    let activity_err_shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    // ---- 屏幕镜像 HTTP 服务 + 运行时配置 API ----
    let screen_shared = screen_http::new_shared_fb();
    let http_next_flag = Arc::new(AtomicBool::new(false));
    let _screen_server = match screen_http::start(
        screen_shared.clone(),
        http_next_flag.clone(),
        config.clone(),
        config_store.clone(),
        creds_store.clone(),
        log_hub.clone(),
    ) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("Screen HTTP server failed to start: {e:#}");
            None
        }
    };

    // ---- 单线程 GH worker(三家串行轮询,省 ~24KB SRAM vs 三线程各 12KB 栈) ----
    gh_worker::spawn(
        config.clone(),
        contrib_shared.clone(),
        contrib_err_shared.clone(),
        notif_shared.clone(),
        activity_shared.clone(),
        activity_err_shared.clone(),
    );
    log::info!("GH worker: contribution + notifications + activity (merged)");
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
        // 每 tick 一次性读出本轮要用的所有配置字段
        let (refresh_period, auto_rotate, auto_rotate_period, tz_off, t_off, h_off) = {
            let c = config.read().unwrap();
            copy_truncated(&mut state.gh_user, &c.gh_user);
            state.gh_token_set = !c.gh_token.is_empty();
            (
                Duration::from_secs(c.sensor_refresh_s as u64),
                c.auto_rotate,
                Duration::from_secs(c.auto_rotate_s as u64),
                c.tz_off_s as i64,
                c.temp_off_c,
                c.humid_off_pct,
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
                boot_edge,
                key_edge,
                http_next,
                auto_due,
                page
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
                let _ = out.push_str(s.get(..5).unwrap_or(s.as_str()));
                out
            });
            state.clock_date = format_local_date(tz_off);

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
                copy_truncated(&mut state.contrib_error, &e);
            }

            // GitHub Notifications:复制到 state(只取最新一条)
            if let Ok(g) = notif_shared.lock() {
                if let Some(s) = g.as_ref() {
                    state.notif_count = s.count as u32;
                    state.notif_top_title.clear();
                    state.notif_top_repo.clear();
                    if let Some(first) = s.items.first() {
                        copy_truncated(&mut state.notif_top_title, &first.title);
                        copy_truncated(&mut state.notif_top_repo, &first.repo);
                    }
                    state.notif_valid = true;
                }
            }

            // GitHub Activity:复制到 state
            if let Ok(g) = activity_shared.lock() {
                if let Some(a) = g.as_ref() {
                    state.last_event_line.clear();
                    if let Some(l) = a.last_line.as_deref() {
                        copy_truncated(&mut state.last_event_line, l);
                    }
                    state.last_event_detail.clear();
                    if let Some(d) = a.last_detail.as_deref() {
                        copy_truncated(&mut state.last_event_detail, d);
                    }
                    state.open_prs = a.open_prs;
                    state.last_event_at_epoch = a.last_at_epoch;
                    state.activity_valid = true;
                }
            }
            if let Ok(e) = activity_err_shared.lock() {
                copy_truncated(&mut state.activity_error, &e);
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
/// 优先级:NVS 多 slot(按 slot 序逐个试)→ 全败回退 SoftAP + HTTP 门户。
fn obtain_creds(
    display: &mut Display<'_>,
    state: &mut AppState,
    wifi: &mut WifiManager,
    store: &CredsStore,
    stored: Vec<WifiCreds>,
) -> anyhow::Result<WifiCreds> {
    // 多凭据场景先扫一次,按 RSSI 强弱筛出"此时能连上"的候选;
    // 单凭据跳过扫描避免 2s 无意义开销。
    let ordered: Vec<WifiCreds> = match stored.len() {
        0 => Vec::new(),
        1 => stored,
        _ => match wifi.scan_and_sort(&stored) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("scan failed, fallback to slot order: {e:#}");
                stored
            }
        },
    };
    let had_stored = !ordered.is_empty();

    // 尝试 1:按排序后的顺序逐个试连,第一个连上就走
    for (i, creds) in ordered.iter().enumerate() {
        log::info!(
            "[cand {}/{}] trying stored creds ssid={}",
            i + 1,
            ordered.len(),
            creds.ssid
        );
        state.prov_mode = false;
        state.prov_hint.clear();
        let _ = state.prov_hint.push_str("connecting...");
        let _ = ui::render(display, state, Page::Dashboard);
        let _ = display.flush();

        if try_connect_n(wifi, creds, WIFI_RETRY_BUDGET) {
            // 成功:把这个 slot promote 到 slot 0(下次开机优先试)
            if let Err(e) = store.save(creds) {
                log::warn!("promote creds to slot 0 failed: {e:#}");
            }
            return Ok(creds.clone());
        }
        log::warn!(
            "stored ssid={} failed {WIFI_RETRY_BUDGET}x, trying next slot",
            creds.ssid
        );
        let _ = wifi.force_stop();
    }
    if had_stored {
        log::warn!(
            "all {} stored cred(s) failed, falling back to SoftAP prov",
            ordered.len()
        );
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
        {
            use core::fmt::Write;
            let _ = write!(state.prov_hint, "connecting {}", creds.ssid);
        }
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

/// 麦克风骨架验证 loop:每帧 1024 个 i16(stereo,L=R=MIC1),约 32ms 一帧。
/// 算 left 声道 RMS,每秒打一行日志——对着麦克风讲话能看到数字明显涨。
fn mic_rms_loop(mut mic: Mic) -> ! {
    let mut buf = [0i16; 1024];
    let mut frame_n: u32 = 0;
    let mut acc_sq: u64 = 0;
    let mut acc_n: u64 = 0;
    loop {
        match mic.read(&mut buf, 1000) {
            Ok(n) => {
                // stereo,左声道在偶数 idx
                for s in buf[..n].iter().step_by(2) {
                    let v = *s as i32;
                    acc_sq += (v * v) as u64;
                    acc_n += 1;
                }
                frame_n += 1;
                // 16kHz / (1024/2) ≈ 31 帧/秒
                if frame_n % 31 == 0 && acc_n > 0 {
                    let rms = ((acc_sq / acc_n) as f64).sqrt() as u32;
                    log::info!("MIC RMS={rms} (n_frames={frame_n})");
                    acc_sq = 0;
                    acc_n = 0;
                }
            }
            Err(e) => {
                log::error!("mic read error: {e:#}");
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// 把 `src` 尽量多地拷入 `dst`(不超 capacity),先 clear。装不下的字符直接丢。
fn copy_truncated<const N: usize>(dst: &mut heapless::String<N>, src: &str) {
    dst.clear();
    for c in src.chars() {
        if dst.push(c).is_err() {
            break;
        }
    }
}

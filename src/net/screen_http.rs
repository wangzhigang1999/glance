//! 屏幕镜像 HTTP 服务 + 运行时配置 API
//!
//! `GET  /`            → 自刷新 HTML 页(内嵌 `<img src=/screen.bmp>`,1 秒一轮)
//! `GET  /screen.bmp`  → 当前 framebuffer 编码为 1-bit BMP
//! `POST /next`        → 翻页触发
//! `GET  /settings`    → 配置表单页(Tailwind)
//! `GET  /api/config`  → 返回当前 RuntimeConfig(JSON;token 已脱敏)
//! `POST /api/config`  → JSON body,更新字段,保存 NVS
//! `GET  /api/wifi`    → 已保存 WiFi 凭据列表(只回 ssid,password 永不外泄)
//! `POST /api/wifi`    → body `{ssid,password}`,追加/提升到 slot 0
//! `POST /api/wifi/remove` → body `{ssid}`,按 ssid 删一个 slot
//! `POST /api/wifi_forget` → 清 wifi NVS 凭据 + 重启(下次开机回 SoftAP 配网)
//! `POST /api/reboot`  → esp_restart()

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use anyhow::{anyhow, Result};
use esp_idf_svc::{
    http::{
        client::{Configuration as HttpClientConfig, EspHttpConnection},
        server::{Configuration, EspHttpServer},
        Method,
    },
    io::Write,
};

use crate::{
    config::{clamp, ConfigStore, SharedConfig},
    display::framebuffer::{BUF_LEN, HEIGHT, WIDTH},
    net::{
        creds::{CredsStore, MAX_SLOTS as WIFI_MAX_SLOTS},
        wifi::WifiCreds,
    },
};

pub type SharedFb = Arc<Mutex<Vec<u8>>>;

pub fn new_shared_fb() -> SharedFb {
    Arc::new(Mutex::new(vec![0xFFu8; BUF_LEN]))
}

/// 统一读 body 到 `$buf`,返回切片到有效字节。
/// 之前四处 handler 各写一遍同一套 `loop match req.read(...)`。
macro_rules! read_body {
    ($req:expr, $buf:expr) => {{
        let buf_ref = &mut $buf;
        let mut total = 0usize;
        loop {
            match $req.read(&mut buf_ref[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if total >= buf_ref.len() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        &buf_ref[..total]
    }};
}

const HTML: &str = include_str!("../../web/live.html");

pub fn start(
    shared: SharedFb,
    next_flag: Arc<AtomicBool>,
    cfg: SharedConfig,
    store: Arc<ConfigStore>,
    creds: Arc<CredsStore>,
) -> Result<EspHttpServer<'static>> {
    let srv_cfg = Configuration {
        stack_size: 10 * 1024,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&srv_cfg)?;

    server.fn_handler("/", Method::Get, |req| -> Result<(), anyhow::Error> {
        let mut resp = req.into_ok_response()?;
        resp.write_all(HTML.as_bytes())?;
        Ok(())
    })?;

    // ---- GET /api/sys: 实时系统快照,调试用 ----
    server.fn_handler(
        "/api/sys",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let sys = crate::hw::system::read_sys_stats();
            let flash = crate::hw::system::read_flash_stats();
            #[derive(serde::Serialize)]
            struct View {
                heap_free: u32,
                heap_total: u32,
                heap_min_ever: u32,
                psram_free: u32,
                psram_total: u32,
                main_stack_hwm_bytes: u32,
                reset_reason: &'static str,
                flash_total: u32,
                app_part_addr: u32,
                app_part_size: u32,
                app_used: u32,
            }
            let v = View {
                heap_free: sys.heap_free as u32,
                heap_total: sys.heap_total as u32,
                heap_min_ever: sys.heap_min_ever as u32,
                psram_free: sys.psram_free as u32,
                psram_total: sys.psram_total as u32,
                main_stack_hwm_bytes: sys.main_stack_hwm_bytes,
                reset_reason: sys.reset_reason,
                flash_total: flash.flash_total,
                app_part_addr: flash.app_part_addr,
                app_part_size: flash.app_part_size,
                app_used: flash.app_used,
            };
            let s = serde_json::to_string(&v).unwrap_or_else(|_| "{}".into());
            let len = s.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(s.as_bytes())?;
            Ok(())
        },
    )?;

    server.fn_handler(
        "/settings",
        Method::Get,
        |req| -> Result<(), anyhow::Error> {
            let mut resp = req.into_ok_response()?;
            resp.write_all(SETTINGS_HTML.as_bytes())?;
            Ok(())
        },
    )?;

    let shared_for_handler = shared.clone();
    server.fn_handler(
        "/screen.bmp",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let fb_copy: Vec<u8> = {
                let guard = shared_for_handler.lock().unwrap();
                guard.clone()
            };
            let bmp = encode_bmp(&fb_copy);
            let len_str = bmp.len().to_string();
            let headers = [
                ("content-type", "image/bmp"),
                ("cache-control", "no-store"),
                ("content-length", len_str.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(&bmp)?;
            Ok(())
        },
    )?;

    let next_for_handler = next_flag.clone();
    server.fn_handler(
        "/next",
        Method::Post,
        move |req| -> Result<(), anyhow::Error> {
            next_for_handler.store(true, Ordering::Relaxed);
            let mut resp = req.into_ok_response()?;
            resp.write_all(b"ok")?;
            Ok(())
        },
    )?;

    // ---- GET /api/config ----
    let cfg_for_get = cfg.clone();
    server.fn_handler(
        "/api/config",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let json = {
                let c = cfg_for_get.read().unwrap();
                emit_config_json(&c, true)
            };
            let len_str = json.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len_str.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- POST /api/config ----
    let cfg_for_post = cfg.clone();
    let store_for_post = store.clone();
    server.fn_handler(
        "/api/config",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            // 读 body,限制 4KB 防溢出
            let mut buf = [0u8; 4096];
            let body_bytes = read_body!(req, buf);
            let body = std::str::from_utf8(body_bytes).unwrap_or("");
            let mut updated = {
                let c = cfg_for_post.read().unwrap();
                c.clone()
            };
            apply_json_patch(&mut updated, body);
            clamp(&mut updated);

            // 持久化
            if let Err(e) = store_for_post.save(&updated) {
                log::warn!("config save failed: {e:#}");
                let mut resp = req.into_status_response(500)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"save failed\"}")?;
                return Ok(());
            }
            // 写回共享
            {
                let mut g = cfg_for_post.write().unwrap();
                *g = updated;
            }
            let json = {
                let c = cfg_for_post.read().unwrap();
                emit_config_json(&c, true)
            };
            let len_str = json.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len_str.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- POST /api/whoami ----
    // body: {"token":"..."}  token 缺省或以 "***" 开头则用当前 config 里的
    let cfg_for_whoami = cfg.clone();
    server.fn_handler(
        "/api/whoami",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let mut buf = [0u8; 512];
            let body_bytes = read_body!(req, buf);
            let body = std::str::from_utf8(body_bytes).unwrap_or("");
            #[derive(serde::Deserialize, Default)]
            struct WhoamiReq {
                #[serde(default)]
                token: String,
            }
            let mut token = serde_json::from_str::<WhoamiReq>(body)
                .unwrap_or_default()
                .token;
            if token.is_empty() || token.starts_with("***") {
                token = cfg_for_whoami.read().unwrap().gh_token.clone();
            }
            if token.is_empty() {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"no token\"}")?;
                return Ok(());
            }
            #[derive(serde::Serialize)]
            struct Ok_ {
                ok: bool,
                login: String,
            }
            #[derive(serde::Serialize)]
            struct Err_ {
                ok: bool,
                error: String,
            }
            match github_whoami(&token) {
                Ok(login) => {
                    let s = serde_json::to_string(&Ok_ { ok: true, login }).unwrap();
                    let len = s.len().to_string();
                    let headers = [
                        ("content-type", "application/json; charset=utf-8"),
                        ("cache-control", "no-store"),
                        ("content-length", len.as_str()),
                    ];
                    let mut resp = req.into_response(200, Some("OK"), &headers)?;
                    resp.write_all(s.as_bytes())?;
                    Ok(())
                }
                Err(e) => {
                    let s = serde_json::to_string(&Err_ {
                        ok: false,
                        error: format!("{e:#}"),
                    })
                    .unwrap();
                    let mut resp = req.into_status_response(502)?;
                    resp.write_all(s.as_bytes())?;
                    Ok(())
                }
            }
        },
    )?;

    // ---- GET /api/wifi:列出已保存的凭据(只返回 ssid + 容量上限) ----
    let creds_for_list = creds.clone();
    server.fn_handler(
        "/api/wifi",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            #[derive(serde::Serialize)]
            struct Slot<'a> {
                ssid: &'a str,
            }
            #[derive(serde::Serialize)]
            struct View<'a> {
                slots: Vec<Slot<'a>>,
                max: usize,
            }
            let list = creds_for_list.load_all().unwrap_or_default();
            let slots: Vec<Slot> = list
                .iter()
                .map(|c| Slot {
                    ssid: c.ssid.as_str(),
                })
                .collect();
            let v = View {
                slots,
                max: WIFI_MAX_SLOTS,
            };
            let json = serde_json::to_string(&v).unwrap_or_else(|_| "{}".into());
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

    // ---- POST /api/wifi:追加一组凭据(命中 ssid 会提升到 slot 0) ----
    let creds_for_add = creds.clone();
    server.fn_handler(
        "/api/wifi",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let mut buf = [0u8; 512];
            let body_bytes = read_body!(req, buf);
            let body = std::str::from_utf8(body_bytes).unwrap_or("");
            #[derive(serde::Deserialize, Default)]
            struct Add {
                #[serde(default)]
                ssid: String,
                #[serde(default)]
                password: String,
            }
            let a: Add = serde_json::from_str(body).unwrap_or_default();
            if a.ssid.is_empty() {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"ssid required\"}")?;
                return Ok(());
            }
            match WifiCreds::new(&a.ssid, &a.password) {
                Ok(c) => {
                    if let Err(e) = creds_for_add.save(&c) {
                        log::warn!("wifi add: save failed: {e:#}");
                        let mut resp = req.into_status_response(500)?;
                        resp.write_all(b"{\"ok\":false,\"error\":\"save failed\"}")?;
                        return Ok(());
                    }
                    log::info!("wifi add: ssid={} promoted to slot 0", a.ssid);
                    let mut resp = req.into_ok_response()?;
                    resp.write_all(b"{\"ok\":true}")?;
                    Ok(())
                }
                Err(e) => {
                    log::warn!("wifi add: invalid: {e:#}");
                    let mut resp = req.into_status_response(400)?;
                    resp.write_all(
                        b"{\"ok\":false,\"error\":\"ssid/password length exceeds 32/64\"}",
                    )?;
                    Ok(())
                }
            }
        },
    )?;

    // ---- POST /api/wifi/remove:按 ssid 删除 slot ----
    let creds_for_rm = creds.clone();
    server.fn_handler(
        "/api/wifi/remove",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let mut buf = [0u8; 256];
            let body_bytes = read_body!(req, buf);
            let body = std::str::from_utf8(body_bytes).unwrap_or("");
            #[derive(serde::Deserialize, Default)]
            struct Rm {
                #[serde(default)]
                ssid: String,
            }
            let r: Rm = serde_json::from_str(body).unwrap_or_default();
            if r.ssid.is_empty() {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"ssid required\"}")?;
                return Ok(());
            }
            if let Err(e) = creds_for_rm.remove(&r.ssid) {
                log::warn!("wifi remove ssid={}: {e:#}", r.ssid);
                let mut resp = req.into_status_response(500)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"remove failed\"}")?;
                return Ok(());
            }
            log::info!("wifi remove: ssid={}", r.ssid);
            let mut resp = req.into_ok_response()?;
            resp.write_all(b"{\"ok\":true}")?;
            Ok(())
        },
    )?;

    // ---- POST /api/wifi_forget:清全部 slot + 重启,下次开机回 SoftAP 门户 ----
    let creds_for_forget = creds.clone();
    server.fn_handler(
        "/api/wifi_forget",
        Method::Post,
        move |req| -> Result<(), anyhow::Error> {
            if let Err(e) = creds_for_forget.clear() {
                log::warn!("wifi_forget: clear NVS failed: {e:#}");
                let mut resp = req.into_status_response(500)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"clear failed\"}")?;
                return Ok(());
            }
            let mut resp = req.into_ok_response()?;
            resp.write_all(b"{\"ok\":true}")?;
            drop(resp);
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            });
            Ok(())
        },
    )?;

    // ---- POST /api/reboot ----
    server.fn_handler(
        "/api/reboot",
        Method::Post,
        |req| -> Result<(), anyhow::Error> {
            let mut resp = req.into_ok_response()?;
            resp.write_all(b"{\"ok\":true}")?;
            drop(resp);
            // 给 TCP flush 一点时间
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            });
            Ok(())
        },
    )?;

    log::info!(
        "Screen HTTP server up on http://<ip>/  (/, /settings, /screen.bmp, /next, /api/config, /api/wifi{{,/remove}}, /api/wifi_forget, /api/reboot)"
    );
    Ok(server)
}

/// 用 token 调用 `GET https://api.github.com/user`,取 `login`
fn github_whoami(token: &str) -> Result<String> {
    let cfg = HttpClientConfig {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(15)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(1024),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&cfg)?;

    let auth = format!("Bearer {}", token);
    let headers = [
        ("user-agent", "clab/0.1"),
        ("accept", "application/vnd.github+json"),
        ("x-github-api-version", "2022-11-28"),
        ("authorization", auth.as_str()),
    ];
    conn.initiate_request(Method::Get, "https://api.github.com/user", &headers)?;
    conn.initiate_response()?;
    let status = conn.status();
    if status != 200 {
        return Err(anyhow!("HTTP {}", status));
    }

    let mut body: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("read: {e:?}")),
        }
    }
    #[derive(serde::Deserialize)]
    struct User {
        login: String,
    }
    let u: User = serde_json::from_slice(&body).map_err(|e| anyhow!("parse /user: {e}"))?;
    Ok(u.login)
}

/// 出站视图:token 脱敏为 `***<后4位>`,额外暴露 `gh_token_set` 给 UI 判断
#[derive(serde::Serialize)]
struct ConfigView<'a> {
    gh_user: &'a str,
    gh_token: String,
    gh_token_set: bool,
    gh_refresh_s: u32,
    gh_err_s: u32,
    sensor_refresh_s: u32,
    auto_rotate: bool,
    auto_rotate_s: u32,
    temp_off_c: f32,
    humid_off_pct: f32,
    tz_off_s: i32,
    splash_flash: u32,
}

fn emit_config_json(c: &crate::config::RuntimeConfig, mask_token: bool) -> String {
    let token_display = if mask_token && !c.gh_token.is_empty() {
        let tail: String = c.gh_token.chars().rev().take(4).collect();
        let tail: String = tail.chars().rev().collect();
        format!("***{tail}")
    } else {
        c.gh_token.clone()
    };
    let view = ConfigView {
        gh_user: &c.gh_user,
        gh_token: token_display,
        gh_token_set: !c.gh_token.is_empty(),
        gh_refresh_s: c.gh_refresh_s,
        gh_err_s: c.gh_err_s,
        sensor_refresh_s: c.sensor_refresh_s,
        auto_rotate: c.auto_rotate,
        auto_rotate_s: c.auto_rotate_s,
        temp_off_c: c.temp_off_c,
        humid_off_pct: c.humid_off_pct,
        tz_off_s: c.tz_off_s,
        splash_flash: c.splash_flash,
    };
    serde_json::to_string(&view).unwrap_or_else(|_| "{}".to_string())
}

/// 入站 patch:所有字段 Option,缺省表示不更新。脱敏 token(`***...`)视为不更新。
#[derive(serde::Deserialize, Default)]
struct ConfigPatch {
    gh_user: Option<String>,
    gh_token: Option<String>,
    gh_refresh_s: Option<u32>,
    gh_err_s: Option<u32>,
    sensor_refresh_s: Option<u32>,
    auto_rotate: Option<bool>,
    auto_rotate_s: Option<u32>,
    temp_off_c: Option<f32>,
    humid_off_pct: Option<f32>,
    tz_off_s: Option<i32>,
    splash_flash: Option<u32>,
}

fn apply_json_patch(c: &mut crate::config::RuntimeConfig, body: &str) {
    let p: ConfigPatch = serde_json::from_str(body).unwrap_or_default();
    if let Some(v) = p.gh_user {
        c.gh_user = v;
    }
    if let Some(v) = p.gh_token {
        if v.is_empty() {
            c.gh_token.clear();
        } else if !v.starts_with("***") {
            c.gh_token = v;
        }
    }
    if let Some(v) = p.gh_refresh_s {
        c.gh_refresh_s = v;
    }
    if let Some(v) = p.gh_err_s {
        c.gh_err_s = v;
    }
    if let Some(v) = p.sensor_refresh_s {
        c.sensor_refresh_s = v;
    }
    if let Some(v) = p.auto_rotate {
        c.auto_rotate = v;
    }
    if let Some(v) = p.auto_rotate_s {
        c.auto_rotate_s = v;
    }
    if let Some(v) = p.temp_off_c {
        c.temp_off_c = v;
    }
    if let Some(v) = p.humid_off_pct {
        c.humid_off_pct = v;
    }
    if let Some(v) = p.tz_off_s {
        c.tz_off_s = v;
    }
    if let Some(v) = p.splash_flash {
        c.splash_flash = v;
    }
}

const SETTINGS_HTML: &str = include_str!("../../web/settings.html");

/// 把 ST7305 本地 fb 编码成标准 1-bit BMP。
fn encode_bmp(fb: &[u8]) -> Vec<u8> {
    // BMP 1-bit:每行字节 = ((W*1 + 31)/32) * 4。W=400 → 52.
    const W: usize = WIDTH as usize;
    const H: usize = HEIGHT as usize;
    const ROW: usize = ((W + 31) / 32) * 4; // 52
    const PIXEL_DATA_LEN: usize = ROW * H; // 15600

    const FILE_HDR: usize = 14;
    const DIB_HDR: usize = 40;
    const PALETTE: usize = 8; // 2 entries × 4 bytes
    const OFFSET: usize = FILE_HDR + DIB_HDR + PALETTE; // 62
    const FILE_SIZE: usize = OFFSET + PIXEL_DATA_LEN; // 15662

    let mut out: Vec<u8> = Vec::with_capacity(FILE_SIZE);

    // ---- BITMAPFILEHEADER ----
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(FILE_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&(OFFSET as u32).to_le_bytes());

    // ---- BITMAPINFOHEADER ----
    out.extend_from_slice(&(DIB_HDR as u32).to_le_bytes());
    out.extend_from_slice(&(W as i32).to_le_bytes());
    // 负 height => top-down
    out.extend_from_slice(&(-(H as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&1u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(PIXEL_DATA_LEN as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes()); // x ppm
    out.extend_from_slice(&2835u32.to_le_bytes()); // y ppm
    out.extend_from_slice(&2u32.to_le_bytes()); // palette entries
    out.extend_from_slice(&0u32.to_le_bytes()); // important

    // ---- palette: [0]=white, [1]=black ----
    out.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    // ---- pixel data ----
    // 每行 ROW 字节,BMP MSB-first。bit=1 表示黑(palette[1])。
    // fb bit=0 表示前景(黑)。我们要把 "黑" 映射到 bit=1。
    //
    // 优化:原写法 300*400=12 万次 `pixel_index_mask` 函数调用 + 分支。
    // 在固定 y 下 `inv_y / blk_y / local_y / shift` 全部常量,只有 byte_x 扫;
    // fb 里一个 byte_x 字节同时包含"偶数 x(local_x=0)"和"奇数 x(local_x=1)"两像素,
    // 一趟就消化 2 个 BMP 像素位,`pixel_index_mask` 完全不必入热循环。
    const H4: usize = HEIGHT as usize / 4; // fb 步长常量
    let row_start = out.len();
    out.resize(row_start + PIXEL_DATA_LEN, 0);
    for y in 0..H {
        let inv_y = H - 1 - y;
        let blk_y = inv_y >> 2;
        let local_y = (inv_y & 3) as u32;
        // 同一 fb 字节里,local_x=0 的位是 (7-2*local_y),local_x=1 的位是 (6-2*local_y)
        let mask_e: u8 = 1 << (7 - 2 * local_y);
        let mask_o: u8 = 1 << (6 - 2 * local_y);

        let out_row = &mut out[row_start + y * ROW..row_start + y * ROW + ROW];
        for x_byte in 0..(W / 8) {
            // BMP byte x_byte 覆盖 8 个像素(x = 8*x_byte..8*x_byte+7),
            // 对应 4 个 fb byte_x(每 fb 字节拿 2 个像素)
            let base = 4 * x_byte;
            let mut b: u8 = 0;
            for k in 0..4u32 {
                let fb_byte = fb[(base + k as usize) * H4 + blk_y];
                // fg bit=0 → 显示黑 → BMP bit=1
                if fb_byte & mask_e == 0 {
                    b |= 1 << (7 - 2 * k);
                }
                if fb_byte & mask_o == 0 {
                    b |= 1 << (6 - 2 * k);
                }
            }
            out_row[x_byte] = b;
        }
        // 剩余 pad 字节保持 0
    }

    out
}

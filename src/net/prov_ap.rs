//! SoftAP + HTTP 门户配网
//!
//! 开 open AP(默认 "RLCD-Setup"),手机连上后访问 http://192.168.4.1
//! 看到表单,填 SSID/密码 → POST /save → 主线程拿回凭据去连 WiFi。
//!
//! 设计:
//! - `Provisioner::start` 起 `EspHttpServer`,挂 `GET /` 表单、`POST /save` 存凭据、
//!   iOS/Android captive portal 探测 URL 都返回表单触发弹窗
//! - HTTP handler 只做"解析 + 发送到 mpsc",不直接碰 WiFi/NVS(WiFi 切模式要主线程来)
//! - 主线程 `wait_for_creds` 阻塞拿凭据,试连失败就重开 AP 等下一次

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;

use super::wifi::WifiCreds;

pub struct Provisioner {
    _server: EspHttpServer<'static>,
    rx: Receiver<(String, String)>,
}

impl Provisioner {
    /// 启动 HTTP 门户。要求 WiFi 已经处于 AP 模式(由调用方 `WifiManager::start_ap` 负责)。
    pub fn start(ap_ssid: &str, mac: &str) -> Result<Self> {
        let srv_cfg = Configuration {
            stack_size: 10 * 1024,
            ..Default::default()
        };
        let mut server = EspHttpServer::new(&srv_cfg)?;

        let form_html = render_form(ap_ssid, mac);
        // form 和 done 页都 'static 化给各 handler 闭包
        let form_html: &'static str = Box::leak(form_html.into_boxed_str());

        // `/` + captive-portal 探测 URL 全部返回表单
        for path in ["/", "/hotspot-detect.html", "/library/test/success.html", "/generate_204"] {
            server.fn_handler(path, Method::Get, move |req| -> Result<(), anyhow::Error> {
                let mut resp = req.into_ok_response()?;
                resp.write_all(form_html.as_bytes())?;
                Ok(())
            })?;
        }

        let (tx, rx): (Sender<(String, String)>, Receiver<(String, String)>) = mpsc::channel();
        // 多个 POST 同时到(理论上不会),Mutex 防互相顶掉
        let tx = Mutex::new(tx);

        server.fn_handler(
            "/save",
            Method::Post,
            move |mut req| -> Result<(), anyhow::Error> {
                // x-www-form-urlencoded,最长 256B 就够(ssid 32 + pwd 64 + 编码膨胀)
                let mut buf = [0u8; 512];
                let mut total = 0usize;
                loop {
                    match req.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => {
                            total += n;
                            if total >= buf.len() {
                                break;
                            }
                        }
                        Err(e) => {
                            log::warn!("prov_ap: read body err: {e:?}");
                            break;
                        }
                    }
                }
                let body = core::str::from_utf8(&buf[..total]).unwrap_or("");
                let (ssid, password) = parse_form(body);
                log::info!(
                    "prov_ap: received ssid={} pwd_len={}",
                    ssid,
                    password.len()
                );

                let done_html = render_done(&ssid);
                let len = done_html.len().to_string();
                let headers = [
                    ("content-type", "text/html; charset=utf-8"),
                    ("cache-control", "no-store"),
                    ("content-length", len.as_str()),
                ];
                let mut resp = req.into_response(200, Some("OK"), &headers)?;
                resp.write_all(done_html.as_bytes())?;
                drop(resp);

                if let Ok(g) = tx.lock() {
                    let _ = g.send((ssid, password));
                }
                Ok(())
            },
        )?;

        log::info!("prov_ap: HTTP portal up on http://192.168.4.1");
        Ok(Self { _server: server, rx })
    }

    /// 阻塞拿一组表单提交。解析 + 长度校验在这里,失败返回 None(调用方重试)。
    pub fn wait_for_creds(&self, timeout: Duration) -> Option<WifiCreds> {
        let (ssid, password) = self.rx.recv_timeout(timeout).ok()?;
        if ssid.is_empty() {
            log::warn!("prov_ap: empty ssid, ignoring");
            return None;
        }
        match WifiCreds::new(&ssid, &password) {
            Ok(c) => Some(c),
            Err(e) => {
                log::warn!("prov_ap: creds invalid: {e}");
                None
            }
        }
    }
}

fn parse_form(body: &str) -> (String, String) {
    let mut ssid = String::new();
    let mut password = String::new();
    for pair in body.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next().unwrap_or("");
        let val = it.next().unwrap_or("");
        let decoded = urldecode(val);
        match key {
            "ssid" => ssid = decoded,
            "password" => password = decoded,
            _ => {}
        }
    }
    (ssid, password)
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push(h * 16 + l);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn render_form(ap_ssid: &str, mac: &str) -> String {
    FORM_HTML
        .replace("{AP}", &html_escape(ap_ssid))
        .replace("{MAC}", &html_escape(mac))
}

fn render_done(ssid: &str) -> String {
    DONE_HTML.replace("{SSID}", &html_escape(ssid))
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const FORM_HTML: &str = r##"<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>RLCD Setup</title>
<style>
  *{box-sizing:border-box;margin:0;padding:0;font-family:-apple-system,system-ui,"Segoe UI",Roboto,sans-serif}
  body{background:#f8f7f4;color:#252119;min-height:100vh;display:grid;place-items:center;padding:20px}
  .card{background:#fff;border:1px solid #dcd9cf;border-radius:14px;padding:28px;max-width:420px;width:100%;box-shadow:0 6px 24px rgba(0,0,0,0.05)}
  .brand{font-size:11px;letter-spacing:.12em;text-transform:uppercase;color:#bc3908;font-weight:700;margin-bottom:10px}
  h1{font-size:22px;margin-bottom:6px;font-weight:700}
  .sub{color:#6b655c;font-size:13.5px;margin-bottom:22px;line-height:1.5}
  label{font-size:12.5px;font-weight:600;display:block;margin-bottom:6px;color:#3a342a}
  input{width:100%;padding:11px 13px;border:1px solid #dcd9cf;border-radius:9px;font-size:15px;margin-bottom:16px;background:#fff;color:#252119}
  input:focus{border-color:#bc3908;outline:none;box-shadow:0 0 0 3px rgba(188,57,8,0.12)}
  button{background:#bc3908;color:#fff;border:none;padding:12px 18px;border-radius:9px;font-size:15px;font-weight:600;cursor:pointer;width:100%;transition:filter .15s}
  button:hover{filter:brightness(1.08)}
  button:active{transform:translateY(1px)}
  .foot{margin-top:18px;font-size:12px;color:#9a9389;text-align:center;font-family:ui-monospace,Consolas,monospace}
</style>
</head>
<body>
<form class=card method=POST action="/save">
  <div class=brand>RLCD Setup</div>
  <h1>Connect to WiFi</h1>
  <p class=sub>Enter your home WiFi credentials. The device will save them and connect automatically.</p>
  <label for=ssid>SSID</label>
  <input id=ssid name=ssid required autocomplete=off autocapitalize=off autocorrect=off spellcheck=false maxlength=32>
  <label for=password>Password</label>
  <input id=password name=password type=password autocomplete=off maxlength=64>
  <button type=submit>Save &amp; Connect</button>
  <p class=foot>AP {AP} &middot; {MAC}</p>
</form>
</body>
</html>"##;

const DONE_HTML: &str = r##"<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>RLCD Setup</title>
<style>
  *{box-sizing:border-box;margin:0;padding:0;font-family:-apple-system,system-ui,"Segoe UI",Roboto,sans-serif}
  body{background:#f8f7f4;color:#252119;min-height:100vh;display:grid;place-items:center;padding:20px}
  .card{background:#fff;border:1px solid #dcd9cf;border-radius:14px;padding:28px;max-width:420px;width:100%;box-shadow:0 6px 24px rgba(0,0,0,0.05);text-align:center}
  .brand{font-size:11px;letter-spacing:.12em;text-transform:uppercase;color:#bc3908;font-weight:700;margin-bottom:10px}
  h1{font-size:22px;margin-bottom:10px;font-weight:700}
  .sub{color:#6b655c;font-size:13.5px;line-height:1.6}
  code{font-family:ui-monospace,Consolas,monospace;background:#f1efe9;padding:2px 6px;border-radius:4px;font-size:13px}
</style>
</head>
<body>
<div class=card>
  <div class=brand>RLCD Setup</div>
  <h1>Trying {SSID}…</h1>
  <p class=sub>Check the device screen. On success it will switch to your WiFi.<br>If it fails, reconnect to <code>RLCD-Setup</code> and try again.</p>
</div>
</body>
</html>"##;

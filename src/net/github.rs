//! 拉取 GitHub 贡献活动(绿点网格)
//!
//! 走 jogruber 的公共代理 API(避免 GraphQL 鉴权):
//!   https://github-contributions-api.jogruber.de/v4/<user>?y=last
//!
//! 返回 JSON:
//! ```json
//! {
//!   "total": {"lastYear": 1234, "2025": 888, ...},
//!   "contributions": [
//!     {"date": "2024-04-18", "count": 3, "level": 2},
//!     ...
//!   ]
//! }
//! ```
//!
//! 我们只要每天的 `level` 和 `total.lastYear`。手写扫描,不拉 serde_json。

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;

#[derive(Debug, Clone, Default)]
pub struct ContribData {
    pub levels: Vec<u8>,
    pub total_year: u32,
}

pub fn fetch(user: &str) -> Result<ContribData> {
    let url = format!(
        "https://github-contributions-api.jogruber.de/v4/{}?y=last",
        user
    );

    let config = Configuration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(20)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(1024),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&config)?;

    let headers = [("accept", "application/json"), ("user-agent", "rlcd-thermo/0.1")];
    log::info!("GitHub: GET {}", url);
    conn.initiate_request(Method::Get, &url, &headers)?;
    conn.initiate_response()?;
    let status = conn.status();
    log::info!("GitHub: {} responded {}", user, status);
    if status != 200 {
        return Err(anyhow!("GitHub API HTTP {}", status));
    }

    // 读完整 body
    let mut body: Vec<u8> = Vec::with_capacity(32 * 1024);
    let mut chunk = [0u8; 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("read body: {e:?}")),
        }
    }
    log::info!("GitHub: read {} bytes body", body.len());

    parse_response(&body).context("parse GitHub contributions JSON")
}

fn parse_response(body: &[u8]) -> Result<ContribData> {
    let s = std::str::from_utf8(body).context("body not utf-8")?;
    let total_year = find_json_int(s, "\"lastYear\"").unwrap_or(0).max(0) as u32;

    let mut levels: Vec<u8> = Vec::with_capacity(400);
    let pattern = "\"level\":";
    let mut cursor = 0usize;
    while let Some(rel) = s[cursor..].find(pattern) {
        let start = cursor + rel + pattern.len();
        let tail = &s[start..];
        let end = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        if end > 0 {
            if let Ok(n) = tail[..end].parse::<u8>() {
                levels.push(n.min(4));
            }
        }
        cursor = start + end;
    }

    log::info!(
        "GitHub: parsed total={}  levels.len()={}",
        total_year,
        levels.len()
    );
    Ok(ContribData { levels, total_year })
}

/// 在 JSON 文本里找到 `key:` 后的第一个整数。简陋但够用。
fn find_json_int(s: &str, key: &str) -> Option<i64> {
    let p = s.find(key)?;
    let tail = &s[p + key.len()..];
    let colon = tail.find(':')?;
    let num = tail[colon + 1..].trim_start();
    let end = num
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(num.len());
    if end == 0 {
        None
    } else {
        num[..end].parse().ok()
    }
}

/// 后台线程:启动时拉一次,之后 6h 一次。失败不退出,下一轮重试。
pub fn spawn_fetcher(
    user: &'static str,
    shared: Arc<Mutex<Option<ContribData>>>,
) {
    thread::Builder::new()
        .name("gh-fetch".into())
        .stack_size(12 * 1024)
        .spawn(move || loop {
            match fetch(user) {
                Ok(data) => {
                    log::info!(
                        "GitHub OK: {} days, {} contribs last year",
                        data.levels.len(),
                        data.total_year
                    );
                    if let Ok(mut g) = shared.lock() {
                        *g = Some(data);
                    }
                }
                Err(e) => {
                    log::warn!("GitHub fetch failed: {e:#}");
                }
            }
            thread::sleep(Duration::from_secs(6 * 3600));
        })
        .expect("spawn github fetcher");
}

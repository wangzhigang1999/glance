//! 拉取 GitHub 贡献活动(绿点 + 当日 commit 数)
//!
//! 改走官方 **GraphQL API**(`POST https://api.github.com/graphql`),避免 jogruber.de
//! 代理从国内网络走国际线路被墙/丢包(原先 `ESP_ERR_HTTP_CONNECT` 就是此)。
//! api.github.com 在国内有 CDN 可达,复用已有 GITHUB_TOKEN。

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;

#[derive(Debug, Clone, Default)]
pub struct ContribData {
    pub levels: Vec<u8>,
    pub counts: Vec<u32>, // 与 levels 一一对应:当日 commit 数
    pub total_year: u32,
}

pub fn fetch(user: &str, token: &str) -> Result<ContribData> {
    let url = "https://api.github.com/graphql";
    // GraphQL 单行 query,避免多余空白占包体
    let body = format!(
        r#"{{"query":"query{{user(login:\"{}\"){{contributionsCollection{{contributionCalendar{{totalContributions weeks{{contributionDays{{contributionCount contributionLevel}}}}}}}}}}}}"}}"#,
        user
    );

    let config = Configuration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(20)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(2048),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&config)?;

    let auth = format!("Bearer {}", token);
    let clen = body.len().to_string();
    let headers = [
        ("user-agent", "rlcd-thermo/0.1"),
        ("accept", "application/vnd.github+json"),
        ("content-type", "application/json"),
        ("authorization", auth.as_str()),
        ("content-length", clen.as_str()),
    ];
    log::info!("GitHub: POST /graphql (contributionCalendar for {})", user);
    conn.initiate_request(Method::Post, url, &headers)?;
    conn.write_all(body.as_bytes())?;
    conn.flush()?;
    conn.initiate_response()?;
    let status = conn.status();
    log::info!("GitHub: graphql responded {}", status);
    if status != 200 {
        return Err(anyhow!("GitHub GraphQL HTTP {}", status));
    }

    let mut body_buf: Vec<u8> = Vec::with_capacity(48 * 1024);
    let mut chunk = [0u8; 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body_buf.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("read body: {e:?}")),
        }
    }
    log::info!("GitHub: read {} bytes body", body_buf.len());

    parse_response(&body_buf).context("parse GitHub GraphQL JSON")
}

fn parse_response(body: &[u8]) -> Result<ContribData> {
    let s = std::str::from_utf8(body).context("body not utf-8")?;

    // 检测 GraphQL 错误
    if s.contains("\"errors\"") {
        return Err(anyhow!(
            "GitHub GraphQL error: {}",
            &s[..s.len().min(200)]
        ));
    }

    let total_year = find_json_int(s, "\"totalContributions\"")
        .unwrap_or(0)
        .max(0) as u32;

    let counts: Vec<u32> = scan_ints(s, "\"contributionCount\":")
        .into_iter()
        .map(|n| n.max(0) as u32)
        .collect();

    let levels: Vec<u8> = scan_strings(s, "\"contributionLevel\":\"")
        .into_iter()
        .map(|l| level_from_str(&l))
        .collect();

    log::info!(
        "GitHub: parsed total={} levels={} counts={}",
        total_year,
        levels.len(),
        counts.len()
    );
    Ok(ContribData {
        levels,
        counts,
        total_year,
    })
}

fn level_from_str(s: &str) -> u8 {
    match s {
        "NONE" => 0,
        "FIRST_QUARTILE" => 1,
        "SECOND_QUARTILE" => 2,
        "THIRD_QUARTILE" => 3,
        "FOURTH_QUARTILE" => 4,
        _ => 0,
    }
}

fn scan_ints(s: &str, key: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = s[cursor..].find(key) {
        let start = cursor + rel + key.len();
        let tail = &s[start..];
        let trimmed = tail.trim_start();
        let skipped = tail.len() - trimmed.len();
        let end = trimmed
            .find(|c: char| !c.is_ascii_digit() && c != '-')
            .unwrap_or(trimmed.len());
        if end > 0 {
            if let Ok(n) = trimmed[..end].parse::<i64>() {
                out.push(n);
            }
        }
        cursor = start + skipped + end;
    }
    out
}

fn scan_strings(s: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = s[cursor..].find(key) {
        let start = cursor + rel + key.len();
        let tail = &s[start..];
        let mut end = 0usize;
        for (i, c) in tail.char_indices() {
            if c == '"' {
                end = i;
                break;
            }
        }
        if end == 0 {
            break;
        }
        out.push(tail[..end].to_string());
        cursor = start + end + 1;
    }
    out
}

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

pub fn spawn_fetcher(
    config: crate::config::SharedConfig,
    shared: Arc<Mutex<Option<ContribData>>>,
    error_shared: Arc<Mutex<String>>,
) {
    thread::Builder::new()
        .name("gh-fetch".into())
        .stack_size(12 * 1024)
        .spawn(move || loop {
            let (user, token, ok_s, err_s) = {
                let c = config.read().unwrap();
                (
                    c.gh_user.clone(),
                    c.gh_token.clone(),
                    c.contrib_ok_s as u64,
                    c.contrib_err_s as u64,
                )
            };
            if user.is_empty() || token.is_empty() {
                if let Ok(mut e) = error_shared.lock() {
                    *e = "no user/token".into();
                }
                thread::sleep(Duration::from_secs(30));
                continue;
            }
            let interval = match fetch(&user, &token) {
                Ok(data) => {
                    log::info!(
                        "GitHub OK: {} days, {} contribs last year",
                        data.levels.len(),
                        data.total_year
                    );
                    if let Ok(mut g) = shared.lock() {
                        *g = Some(data);
                    }
                    if let Ok(mut e) = error_shared.lock() {
                        e.clear();
                    }
                    Duration::from_secs(ok_s)
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    log::warn!("GitHub fetch failed: {msg}");
                    if let Ok(mut es) = error_shared.lock() {
                        *es = msg;
                    }
                    Duration::from_secs(err_s)
                }
            };
            thread::sleep(interval);
        })
        .expect("spawn github fetcher");
}

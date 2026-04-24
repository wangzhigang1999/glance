//! 拉取 GitHub 贡献活动(绿点 + 当日 commit 数)
//!
//! 改走官方 **GraphQL API**(`POST https://api.github.com/graphql`),避免 jogruber.de
//! 代理从国内网络走国际线路被墙/丢包(原先 `ESP_ERR_HTTP_CONNECT` 就是此)。
//! api.github.com 在国内有 CDN 可达,复用已有 GITHUB_TOKEN。

use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::{
    http::{
        client::{Configuration, EspHttpConnection},
        Method,
    },
    io::Write,
};

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

// ---- GraphQL 响应 deserialize ----
#[derive(serde::Deserialize)]
struct GqlResp {
    #[serde(default)]
    data: Option<GqlData>,
    #[serde(default)]
    errors: Option<serde_json::Value>,
}
#[derive(serde::Deserialize)]
struct GqlData {
    // user 可能为 null(用户名不存在),此时 errors 会带原因
    user: Option<GqlUser>,
}
#[derive(serde::Deserialize)]
struct GqlUser {
    #[serde(rename = "contributionsCollection")]
    cc: Cc,
}
#[derive(serde::Deserialize)]
struct Cc {
    #[serde(rename = "contributionCalendar")]
    cal: Cal,
}
#[derive(serde::Deserialize)]
struct Cal {
    #[serde(rename = "totalContributions")]
    total: u32,
    weeks: Vec<Week>,
}
#[derive(serde::Deserialize)]
struct Week {
    #[serde(rename = "contributionDays")]
    days: Vec<Day>,
}
#[derive(serde::Deserialize)]
struct Day {
    #[serde(rename = "contributionCount")]
    count: u32,
    #[serde(rename = "contributionLevel")]
    level: String,
}

fn parse_response(body: &[u8]) -> Result<ContribData> {
    let resp: GqlResp = serde_json::from_slice(body).context("graphql JSON")?;
    if let Some(errs) = resp.errors {
        // 尝试提取第一个 errors[].message 展示给人看
        let msg = errs
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                let s = serde_json::to_string(&errs).unwrap_or_default();
                s[..s.len().min(200)].to_string()
            });
        return Err(anyhow!("GitHub: {}", msg));
    }
    let user = resp.data.and_then(|d| d.user).context("user not found")?;
    let cal = user.cc.cal;

    let mut counts: Vec<u32> = Vec::with_capacity(400);
    let mut levels: Vec<u8> = Vec::with_capacity(400);
    for w in cal.weeks {
        for d in w.days {
            counts.push(d.count);
            levels.push(level_from_str(&d.level));
        }
    }

    log::info!(
        "GitHub: parsed total={} levels={} counts={}",
        cal.total,
        levels.len(),
        counts.len()
    );
    Ok(ContribData {
        levels,
        counts,
        total_year: cal.total,
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

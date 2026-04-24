//! 拉 GitHub Notifications(需 PAT:scope=notifications)
//!
//! API: GET https://api.github.com/notifications?all=false
//! 响应: 数组,每条含 `subject.title` 和 `reason`。这里只取 title + count。

use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::http::{
    client::{Configuration, EspHttpConnection},
    Method,
};

#[derive(Debug, Clone, Default)]
pub struct NotifItem {
    pub title: String,
    pub repo: String,
}

#[derive(Debug, Clone, Default)]
pub struct NotifSummary {
    pub count: usize,
    pub items: Vec<NotifItem>,
}

pub fn fetch(token: &str) -> Result<NotifSummary> {
    let url = "https://api.github.com/notifications?all=false&per_page=30";

    let config = Configuration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(20)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(1024),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&config)?;

    let auth = format!("Bearer {}", token);
    let headers = [
        ("user-agent", "rlcd-thermo/0.1"),
        ("accept", "application/vnd.github+json"),
        ("authorization", auth.as_str()),
        ("x-github-api-version", "2022-11-28"),
    ];
    log::info!("GH Notif: GET {}", url);
    conn.initiate_request(Method::Get, url, &headers)?;
    conn.initiate_response()?;
    let status = conn.status();
    log::info!("GH Notif: HTTP {}", status);
    if status == 401 {
        return Err(anyhow!("GitHub notifications: 401 — token invalid/expired"));
    }
    if status != 200 {
        return Err(anyhow!("GitHub notifications: HTTP {}", status));
    }

    let mut body: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("read body: {e:?}")),
        }
    }

    parse(&body).context("parse notifications JSON")
}

#[derive(serde::Deserialize)]
struct RawNotif {
    subject: Subject,
    repository: Repository,
}
#[derive(serde::Deserialize)]
struct Subject {
    title: String,
}
#[derive(serde::Deserialize)]
struct Repository {
    full_name: String,
}

fn parse(body: &[u8]) -> Result<NotifSummary> {
    let raw: Vec<RawNotif> = serde_json::from_slice(body).context("notifications JSON")?;
    let items: Vec<NotifItem> = raw
        .into_iter()
        .map(|n| NotifItem {
            title: n.subject.title,
            repo: n.repository.full_name,
        })
        .collect();
    log::info!("GH Notif: parsed {} unread", items.len());
    Ok(NotifSummary {
        count: items.len(),
        items,
    })
}

/// 后台线程:周期从 config.notif_s 读,每轮 re-read
pub fn spawn_fetcher(
    config: crate::config::SharedConfig,
    shared: Arc<Mutex<Option<NotifSummary>>>,
) {
    thread::Builder::new()
        .name("gh-notif".into())
        .stack_size(12 * 1024)
        .spawn(move || loop {
            let (token, period_s) = {
                let c = config.read().unwrap();
                (c.gh_token.clone(), c.notif_s as u64)
            };
            if token.is_empty() {
                thread::sleep(Duration::from_secs(30));
                continue;
            }
            match fetch(&token) {
                Ok(s) => {
                    log::info!("GH Notif OK: {} unread", s.count);
                    if let Ok(mut g) = shared.lock() {
                        *g = Some(s);
                    }
                }
                Err(e) => {
                    log::warn!("GH Notif fetch failed: {e:#}");
                }
            }
            thread::sleep(Duration::from_secs(period_s));
        })
        .expect("spawn gh notif fetcher");
}

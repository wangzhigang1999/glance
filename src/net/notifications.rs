//! 拉 GitHub Notifications(需 PAT:scope=notifications)
//!
//! API: GET https://api.github.com/notifications?all=false
//! 响应: 数组,每条含 `subject.title` 和 `reason`。这里只取 title + count。

use anyhow::{Context, Result};

use super::gh_http::gh_request;

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
    log::info!("GH Notif: GET {}", url);
    let body = gh_request(url, Some(token), None)?;
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

//! GitHub 活动数据:最近一条 event(任意类型) + open PR 数
//!
//! `/users/{user}/events` 带 token 可见 private 事件,按时间倒序;
//! 第一条就是最新。不同 type 从 payload 不同子对象取动作/目标。

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;

#[derive(Debug, Clone, Default)]
pub struct Activity {
    /// 一行摘要,例如 `"reviewed rust-lang/rust#1234"` 或 `"pushed to esp32-s3-rlcd"`
    pub last_line: Option<String>,
    /// 详情文本(commit msg / comment body / PR title 首行),可能为空
    pub last_detail: Option<String>,
    pub open_prs: u32,
}

pub fn fetch(user: &str, token: &str) -> Result<Activity> {
    let mut act = Activity::default();

    // ---- 1. events(带 token 可见 private) ----
    let ev_url = format!("https://api.github.com/users/{}/events?per_page=20", user);
    let body = http_get(&ev_url, Some(token))?;
    if let Some((line, detail)) = parse_last_event(&body) {
        act.last_line = Some(line);
        act.last_detail = if detail.is_empty() { None } else { Some(detail) };
    }

    // ---- 2. open PRs ----
    let search = format!(
        "https://api.github.com/search/issues?q=is:open+is:pr+author:{}&per_page=1",
        user
    );
    if let Ok(body) = http_get(&search, Some(token)) {
        #[derive(serde::Deserialize)]
        struct Search {
            total_count: u32,
        }
        if let Ok(s) = serde_json::from_str::<Search>(&body) {
            act.open_prs = s.total_count;
        }
    } else {
        log::warn!("activity: PR search failed (ignored)");
    }

    Ok(act)
}

fn http_get(url: &str, token: Option<&str>) -> Result<String> {
    let config = Configuration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(20)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(1024),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&config)?;

    let auth = token.map(|t| format!("Bearer {}", t));
    let mut headers: Vec<(&str, &str)> = vec![
        ("user-agent", "rlcd-thermo/0.1"),
        ("accept", "application/vnd.github+json"),
        ("x-github-api-version", "2022-11-28"),
    ];
    if let Some(a) = auth.as_deref() {
        headers.push(("authorization", a));
    }

    log::info!("GH Activity: GET {}", url);
    conn.initiate_request(Method::Get, url, &headers)?;
    conn.initiate_response()?;
    let status = conn.status();
    if status != 200 {
        return Err(anyhow!("HTTP {} for {}", status, url));
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
    Ok(String::from_utf8(body).context("body not utf-8")?)
}

// ---- Event deserialize;payload 因 type 而异,所有字段都 Option ----
#[derive(serde::Deserialize)]
struct Event {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    repo: Option<Repo>,
    #[serde(default)]
    payload: Option<Payload>,
}
#[derive(serde::Deserialize)]
struct Repo {
    name: String,
}
#[derive(serde::Deserialize, Default)]
struct Payload {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    commits: Option<Vec<Commit>>,
    #[serde(default)]
    pull_request: Option<Issue>,
    #[serde(default)]
    issue: Option<Issue>,
    #[serde(default)]
    comment: Option<Body>,
    #[serde(default)]
    review: Option<Body>,
    #[serde(default)]
    release: Option<Named>,
    #[serde(default)]
    ref_type: Option<String>,
}
#[derive(serde::Deserialize)]
struct Commit {
    message: String,
}
#[derive(serde::Deserialize)]
struct Issue {
    number: u64,
    #[serde(default)]
    title: Option<String>,
}
#[derive(serde::Deserialize)]
struct Body {
    #[serde(default)]
    body: Option<String>,
}
#[derive(serde::Deserialize)]
struct Named {
    #[serde(default)]
    name: Option<String>,
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// 返回 (摘要行, 详情)。摘要形如 "reviewed repo#123"、"pushed to repo"。
fn parse_last_event(body: &str) -> Option<(String, String)> {
    // events 接口返回数组,取第一条
    let events: Vec<Event> = serde_json::from_str(body).ok()?;
    let ev = events.into_iter().next()?;
    let repo = ev.repo.map(|r| r.name).unwrap_or_default();
    let pl = ev.payload.unwrap_or_default();

    let (line, detail) = match ev.ty.as_str() {
        "PushEvent" => {
            // 最后一条 commit 的 message 首行
            let detail = pl
                .commits
                .and_then(|cs| cs.into_iter().last())
                .map(|c| first_line(&c.message))
                .unwrap_or_default();
            (format!("pushed to {}", repo), detail)
        }
        "PullRequestEvent" => {
            let action = pl.action.unwrap_or_else(|| "updated".into());
            let pr = pl.pull_request.unwrap_or(Issue {
                number: 0,
                title: None,
            });
            let detail = pr.title.unwrap_or_default();
            (format!("{} PR {}#{}", action, repo, pr.number), detail)
        }
        "PullRequestReviewEvent" => {
            let num = pl.pull_request.map(|p| p.number).unwrap_or(0);
            let detail = pl.review.and_then(|r| r.body).map(|s| first_line(&s)).unwrap_or_default();
            (format!("reviewed {}#{}", repo, num), detail)
        }
        "PullRequestReviewCommentEvent" => {
            let num = pl.pull_request.map(|p| p.number).unwrap_or(0);
            let detail = pl.comment.and_then(|c| c.body).map(|s| first_line(&s)).unwrap_or_default();
            (format!("review comment on {}#{}", repo, num), detail)
        }
        "IssueCommentEvent" => {
            let num = pl.issue.as_ref().map(|i| i.number).unwrap_or(0);
            let detail = pl.comment.and_then(|c| c.body).map(|s| first_line(&s)).unwrap_or_default();
            (format!("commented on {}#{}", repo, num), detail)
        }
        "IssuesEvent" => {
            let action = pl.action.unwrap_or_else(|| "updated".into());
            let issue = pl.issue.unwrap_or(Issue {
                number: 0,
                title: None,
            });
            let detail = issue.title.unwrap_or_default();
            (format!("{} issue {}#{}", action, repo, issue.number), detail)
        }
        "CreateEvent" => {
            let rt = pl.ref_type.unwrap_or_default();
            (format!("created {} in {}", rt, repo), String::new())
        }
        "ForkEvent" => (format!("forked {}", repo), String::new()),
        "WatchEvent" => (format!("starred {}", repo), String::new()),
        "ReleaseEvent" => {
            let name = pl.release.and_then(|r| r.name).unwrap_or_default();
            (format!("released {} in {}", name, repo), String::new())
        }
        other => (
            format!("{} on {}", other.trim_end_matches("Event"), repo),
            String::new(),
        ),
    };
    Some((line, detail))
}

pub fn spawn_fetcher(
    config: crate::config::SharedConfig,
    shared: Arc<Mutex<Option<Activity>>>,
    error_shared: Arc<Mutex<String>>,
) {
    thread::Builder::new()
        .name("gh-activity".into())
        .stack_size(12 * 1024)
        .spawn(move || {
            // stagger:避开启动瞬间 github + notifications 同时握 TLS 挤 lwip socket
            let stagger = config.read().unwrap().activity_stagger_s as u64;
            thread::sleep(Duration::from_secs(stagger));
            loop {
                let (user, token, ok_s, err_s) = {
                    let c = config.read().unwrap();
                    (
                        c.gh_user.clone(),
                        c.gh_token.clone(),
                        c.activity_ok_s as u64,
                        c.activity_err_s as u64,
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
                    Ok(a) => {
                        log::info!(
                            "GH Activity: last={:?} open_prs={}",
                            a.last_line,
                            a.open_prs,
                        );
                        if let Ok(mut g) = shared.lock() {
                            *g = Some(a);
                        }
                        if let Ok(mut e) = error_shared.lock() {
                            e.clear();
                        }
                        Duration::from_secs(ok_s)
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        log::warn!("GH Activity fetch failed: {msg}");
                        if let Ok(mut es) = error_shared.lock() {
                            *es = msg;
                        }
                        Duration::from_secs(err_s)
                    }
                };
                thread::sleep(interval);
            }
        })
        .expect("spawn gh activity fetcher");
}

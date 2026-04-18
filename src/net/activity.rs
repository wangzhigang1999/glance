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
        if let Some(tc) = find_total_count(&body) {
            act.open_prs = tc as u32;
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

/// 返回 (摘要行, 详情)。摘要形如 "reviewed repo#123"、"pushed to repo"。
fn parse_last_event(body: &str) -> Option<(String, String)> {
    // 最新一条 event 的 type:第一次出现的 "type":
    let ev_type = find_str(body, "\"type\":\"")?;
    let repo = find_str_after(body, "\"repo\":{", "\"name\":\"").unwrap_or_default();

    let line;
    let detail;
    match ev_type.as_str() {
        "PushEvent" => {
            // payload.commits[-1].message
            detail = find_last_str_after(body, "\"commits\":", "\"message\":\"")
                .map(first_line)
                .unwrap_or_default();
            line = format!("pushed to {}", repo);
        }
        "PullRequestEvent" => {
            let action = find_str(body, "\"action\":\"").unwrap_or_else(|| "updated".into());
            let num = find_int_after(body, "\"pull_request\":{", "\"number\":").unwrap_or(0);
            detail = find_str_after(body, "\"pull_request\":{", "\"title\":\"").unwrap_or_default();
            line = format!("{} PR {}#{}", action, repo, num);
        }
        "PullRequestReviewEvent" => {
            let num = find_int_after(body, "\"pull_request\":{", "\"number\":").unwrap_or(0);
            let review_body =
                find_str_after(body, "\"review\":{", "\"body\":\"").unwrap_or_default();
            detail = first_line(review_body);
            line = format!("reviewed {}#{}", repo, num);
        }
        "PullRequestReviewCommentEvent" => {
            let num = find_int_after(body, "\"pull_request\":{", "\"number\":").unwrap_or(0);
            let c = find_str_after(body, "\"comment\":{", "\"body\":\"").unwrap_or_default();
            detail = first_line(c);
            line = format!("review comment on {}#{}", repo, num);
        }
        "IssueCommentEvent" => {
            let num = find_int_after(body, "\"issue\":{", "\"number\":").unwrap_or(0);
            let c = find_str_after(body, "\"comment\":{", "\"body\":\"").unwrap_or_default();
            detail = first_line(c);
            line = format!("commented on {}#{}", repo, num);
        }
        "IssuesEvent" => {
            let action = find_str(body, "\"action\":\"").unwrap_or_else(|| "updated".into());
            let num = find_int_after(body, "\"issue\":{", "\"number\":").unwrap_or(0);
            detail = find_str_after(body, "\"issue\":{", "\"title\":\"").unwrap_or_default();
            line = format!("{} issue {}#{}", action, repo, num);
        }
        "CreateEvent" => {
            let ref_type = find_str(body, "\"ref_type\":\"").unwrap_or_default();
            detail = String::new();
            line = format!("created {} in {}", ref_type, repo);
        }
        "ForkEvent" => {
            detail = String::new();
            line = format!("forked {}", repo);
        }
        "WatchEvent" => {
            detail = String::new();
            line = format!("starred {}", repo);
        }
        "ReleaseEvent" => {
            let name = find_str_after(body, "\"release\":{", "\"name\":\"").unwrap_or_default();
            detail = String::new();
            line = format!("released {} in {}", name, repo);
        }
        _ => {
            detail = String::new();
            line = format!("{} on {}", ev_type.trim_end_matches("Event"), repo);
        }
    }
    Some((line, detail))
}

fn first_line(s: String) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// 找 `key` 后的 JSON 字符串(处理 \" \\ \n 转义)。key 需包含起始引号,如 `"\"name\":\""`。
fn find_str(body: &str, key: &str) -> Option<String> {
    let pos = body.find(key)?;
    read_json_string(&body[pos + key.len()..])
}

/// 先定位 anchor,再在其之后找 key 的字符串值。
fn find_str_after(body: &str, anchor: &str, key: &str) -> Option<String> {
    let a = body.find(anchor)?;
    let sub = &body[a..];
    find_str(sub, key)
}

/// 找 anchor 之后的 key,但返回**最后一次**命中(对 commits[-1] 用)。
fn find_last_str_after(body: &str, anchor: &str, key: &str) -> Option<String> {
    let a = body.find(anchor)?;
    let sub = &body[a..];
    let mut cursor = 0usize;
    let mut last: Option<String> = None;
    while let Some(rel) = sub[cursor..].find(key) {
        let start = cursor + rel + key.len();
        if let Some(s) = read_json_string(&sub[start..]) {
            last = Some(s);
        }
        cursor = start;
    }
    last
}

fn find_int_after(body: &str, anchor: &str, key: &str) -> Option<i64> {
    let a = body.find(anchor)?;
    let sub = &body[a..];
    let p = sub.find(key)?;
    let tail = sub[p + key.len()..].trim_start();
    let end = tail.find(|c: char| !c.is_ascii_digit() && c != '-')?;
    tail[..end].parse().ok()
}

fn read_json_string(after_opening_quote: &str) -> Option<String> {
    let mut out = String::with_capacity(64);
    let mut chars = after_opening_quote.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push(' '),
                'u' => {
                    // 跳过 \uXXXX,不展开
                    for _ in 0..4 {
                        chars.next();
                    }
                    out.push('?');
                }
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

fn find_total_count(body: &str) -> Option<i64> {
    let p = body.find("\"total_count\":")?;
    let tail = body[p + "\"total_count\":".len()..].trim_start();
    let end = tail.find(|c: char| !c.is_ascii_digit() && c != '-')?;
    tail[..end].parse().ok()
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

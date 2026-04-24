//! 拉取 GitHub 贡献活动(绿点 + 当日 commit 数)
//!
//! 走官方 **GraphQL API**(`POST https://api.github.com/graphql`);HTTP 底层由
//! `gh_http::gh_request` 统一处理。
//!
//! 调度线程合并到 `net::gh_worker`,本文件只提供 `fetch` + 类型。

use anyhow::{anyhow, Context, Result};

use super::gh_http::gh_request;

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

    log::info!("GitHub: POST /graphql (contributionCalendar for {})", user);
    let body_buf = gh_request(url, Some(token), Some(body.as_bytes()))?;
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

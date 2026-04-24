//! 合并后的 GitHub 后台 fetcher
//!
//! 原先 contrib / notif / activity 各起一个 12KB 栈的线程共 36KB;
//! 这里合成一个 `gh-worker`,按同一周期串行轮询三个端点。
//! 串行 TLS 握手天然避开"同时抢 lwip socket",原先的 `activity_stagger_s` 作废。

use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use super::{activity::Activity, github::ContribData, notifications::NotifSummary};

/// 启动一个后台线程,按 `gh_refresh_s` 周期轮询 contrib / notif / activity;
/// 任何一次失败就退避到 `gh_err_s` 再试整轮。
pub fn spawn(
    config: crate::config::SharedConfig,
    contrib: Arc<Mutex<Option<ContribData>>>,
    contrib_err: Arc<Mutex<String>>,
    notif: Arc<Mutex<Option<NotifSummary>>>,
    activity: Arc<Mutex<Option<Activity>>>,
    activity_err: Arc<Mutex<String>>,
) {
    thread::Builder::new()
        .name("gh-worker".into())
        .stack_size(16 * 1024)
        .spawn(move || loop {
            let (user, token, ok_s, err_s) = {
                let c = config.read().unwrap();
                (
                    c.gh_user.clone(),
                    c.gh_token.clone(),
                    c.gh_refresh_s as u64,
                    c.gh_err_s as u64,
                )
            };

            // 没 user/token:三家全标错,歇 30s 再看配置
            if user.is_empty() || token.is_empty() {
                if let Ok(mut e) = contrib_err.lock() {
                    *e = "no user/token".into();
                }
                if let Ok(mut e) = activity_err.lock() {
                    *e = "no user/token".into();
                }
                thread::sleep(Duration::from_secs(30));
                continue;
            }

            // 任一失败 = 这轮整体记错 + 短退避重来
            let mut had_err = false;

            match super::github::fetch(&user, &token) {
                Ok(data) => {
                    log::info!(
                        "GitHub OK: {} days, {} contribs last year",
                        data.levels.len(),
                        data.total_year,
                    );
                    if let Ok(mut g) = contrib.lock() {
                        *g = Some(data);
                    }
                    if let Ok(mut e) = contrib_err.lock() {
                        e.clear();
                    }
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    log::warn!("GitHub fetch failed: {msg}");
                    if let Ok(mut es) = contrib_err.lock() {
                        *es = msg;
                    }
                    had_err = true;
                }
            }

            match super::notifications::fetch(&token) {
                Ok(s) => {
                    log::info!("GH Notif OK: {} unread", s.count);
                    if let Ok(mut g) = notif.lock() {
                        *g = Some(s);
                    }
                }
                Err(e) => {
                    log::warn!("GH Notif fetch failed: {e:#}");
                    had_err = true;
                }
            }

            match super::activity::fetch(&user, &token) {
                Ok(a) => {
                    log::info!(
                        "GH Activity: last={:?} open_prs={}",
                        a.last_line,
                        a.open_prs,
                    );
                    if let Ok(mut g) = activity.lock() {
                        *g = Some(a);
                    }
                    if let Ok(mut e) = activity_err.lock() {
                        e.clear();
                    }
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    log::warn!("GH Activity fetch failed: {msg}");
                    if let Ok(mut es) = activity_err.lock() {
                        *es = msg;
                    }
                    had_err = true;
                }
            }

            let sleep_s = if had_err { err_s } else { ok_s };
            thread::sleep(Duration::from_secs(sleep_s));
        })
        .expect("spawn gh-worker");
}

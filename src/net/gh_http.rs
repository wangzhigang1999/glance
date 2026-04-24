//! GitHub API 公共 HTTP 帮手
//!
//! 三个 fetcher(github / notifications / activity)原本各写一遍
//! `EspHttpConnection::new → headers → initiate → read chunks`,
//! 这里统一:`gh_request` 处理 GET/POST,自动挂 UA/Accept/API 版本/Bearer。
//!
//! 返回 body 原始字节,parser 自己 `serde_json::from_slice` 或 `String::from_utf8`。

use std::time::Duration;

use anyhow::{anyhow, Result};
use esp_idf_svc::{
    http::{
        client::{Configuration, EspHttpConnection},
        Method,
    },
    io::Write,
};

const UA: &str = "clab/0.1";
const API_VERSION: &str = "2022-11-28";
const ACCEPT: &str = "application/vnd.github+json";

/// 统一 GitHub HTTP 调用。
/// - `token` 为 `Some(..)` 则挂 `Authorization: Bearer ..`
/// - `body` 为 `Some(..)` 则走 POST + `Content-Type: application/json`,否则 GET
pub fn gh_request(url: &str, token: Option<&str>, body: Option<&[u8]>) -> Result<Vec<u8>> {
    let config = Configuration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(20)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(if body.is_some() { 2048 } else { 1024 }),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&config)?;

    // Header 拥有者要在 initiate_request 期间存活,分配在栈上
    let auth_owned = token.map(|t| format!("Bearer {}", t));
    let clen_owned = body.map(|b| b.len().to_string());

    let mut headers: heapless::Vec<(&str, &str), 6> = heapless::Vec::new();
    let _ = headers.push(("user-agent", UA));
    let _ = headers.push(("accept", ACCEPT));
    let _ = headers.push(("x-github-api-version", API_VERSION));
    if let Some(a) = auth_owned.as_deref() {
        let _ = headers.push(("authorization", a));
    }
    if body.is_some() {
        let _ = headers.push(("content-type", "application/json"));
    }
    if let Some(len) = clen_owned.as_deref() {
        let _ = headers.push(("content-length", len));
    }

    let method = if body.is_some() {
        Method::Post
    } else {
        Method::Get
    };
    conn.initiate_request(method, url, &headers)?;
    if let Some(b) = body {
        conn.write_all(b)?;
        conn.flush()?;
    }
    conn.initiate_response()?;
    let status = conn.status();
    if status != 200 {
        return Err(anyhow!("HTTP {} for {}", status, url));
    }

    let mut out: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(anyhow!("read body: {e:?}")),
        }
    }
    Ok(out)
}

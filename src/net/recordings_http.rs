//! 录音 HTTP 接口:列表 / 流式下载(带 Range)/ 删除。
//!
//! - `GET /recordings.html`           → 浏览页(列表 + 直接 audio 控件)
//! - `GET /api/recordings`            → JSON 列表 `{files:[{name,size},...]}`
//! - `GET /api/recording?name=foo.wav` → 流式 WAV;支持 `Range: bytes=...` → 206
//! - `DELETE /api/recording?name=foo.wav` → 删除
//!
//! 文件全部存在 `/storage/` 根(SD 或 SPIFFS,挂载点统一),平面命名空间;
//! `is_safe_name` 拒绝任何带斜杠 / 点开头 / 不以 .wav 结尾的请求,防止越权。

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
};

use anyhow::{anyhow, Result};
use esp_idf_svc::{
    http::{server::EspHttpServer, Method},
    io::Write,
};

use crate::recorder;

const REC_DIR: &str = "/storage";
const READ_CHUNK: usize = 4096;

const REC_HTML: &str = include_str!("../../web/recordings.html");

pub fn register(server: &mut EspHttpServer<'static>) -> Result<()> {
    // ---- 浏览页 ----
    server.fn_handler(
        "/recordings.html",
        Method::Get,
        |req| -> Result<(), anyhow::Error> {
            let mut resp = req.into_ok_response()?;
            resp.write_all(REC_HTML.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- 重新扫盘灌索引(脱钩时人肉触发) ----
    server.fn_handler(
        "/api/recordings/resync",
        Method::Post,
        |req| -> Result<(), anyhow::Error> {
            recorder::index_scan_storage();
            let (_, total, total_size) = recorder::index_list_paged(0, 0);
            let body = format!("{{\"ok\":true,\"total\":{total},\"total_size\":{total_size}}}");
            let mut resp = req.into_ok_response()?;
            resp.write_all(body.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- 批量删全部:迭代 index 逐个 remove(SD/FATFS 和 SPIFFS 都走这条) ----
    server.fn_handler(
        "/api/recordings",
        Method::Delete,
        |req| -> Result<(), anyhow::Error> {
            let (entries, total, _) = recorder::index_list_paged(0, 0);
            // 内存 index 立刻清 → list 立刻显示空 → 视觉响应即时
            recorder::index_clear();
            let mut deleted = 0u32;
            for e in &entries {
                let path = format!("{REC_DIR}/{}", e.name);
                if fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
            }
            log::info!("bulk-deleted {deleted}/{total} recordings");
            let body = format!("{{\"ok\":true,\"deleted\":{deleted},\"total\":{total}}}");
            let mut resp = req.into_ok_response()?;
            resp.write_all(body.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- JSON 列表(从内存 index 读,不走 SPIFFS read_dir) ----
    // ?limit=N(默认 30,0 = 不限);?offset=M
    server.fn_handler(
        "/api/recordings",
        Method::Get,
        |req| -> Result<(), anyhow::Error> {
            let q = req.uri().split_once('?').map(|(_, q)| q).unwrap_or("");
            let limit = parse_query_uint(q, "limit").unwrap_or(30);
            let offset = parse_query_uint(q, "offset").unwrap_or(0);
            let (entries, total, total_size) =
                recorder::index_list_paged(offset as usize, limit as usize);
            let mut json = String::with_capacity(80 + entries.len() * 64);
            json.push_str("{\"files\":[");
            for (i, e) in entries.iter().enumerate() {
                if i > 0 {
                    json.push(',');
                }
                json.push_str(&format!("{{\"name\":\"{}\",\"size\":{}}}", e.name, e.size));
            }
            json.push_str(&format!(
                "],\"total\":{total},\"total_size\":{total_size}}}"
            ));
            let len = json.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- 流式下载(支持 Range) ----
    server.fn_handler(
        "/api/recording",
        Method::Get,
        |req| -> Result<(), anyhow::Error> {
            let Some(name) = parse_query_name(req.uri()) else {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"missing name")?;
                return Ok(());
            };
            if !is_safe_name(&name) {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"bad name")?;
                return Ok(());
            }
            let path = format!("{REC_DIR}/{name}");
            let meta = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => {
                    let mut resp = req.into_status_response(404)?;
                    resp.write_all(b"not found")?;
                    return Ok(());
                }
            };
            let size = meta.len();
            let range = req.header("Range").and_then(parse_range);

            // Range:bytes=START-END (END 可空,表示到末尾)。<audio>/<video> 跳进度时会发。
            if let Some((start, end_opt)) = range {
                if start >= size {
                    // 416 Range Not Satisfiable
                    let mut resp = req.into_status_response(416)?;
                    resp.write_all(b"range out of bounds")?;
                    return Ok(());
                }
                let end = end_opt.unwrap_or(size - 1).min(size - 1);
                let len = end - start + 1;
                let cr = format!("bytes {start}-{end}/{size}");
                let len_str = len.to_string();
                let headers = [
                    ("content-type", "audio/wav"),
                    ("accept-ranges", "bytes"),
                    ("content-range", cr.as_str()),
                    ("content-length", len_str.as_str()),
                    ("cache-control", "no-store"),
                ];
                let mut resp = req.into_response(206, Some("Partial Content"), &headers)?;
                stream_file_range(&mut resp, &path, start, len)?;
            } else {
                let len_str = size.to_string();
                let headers = [
                    ("content-type", "audio/wav"),
                    ("accept-ranges", "bytes"),
                    ("content-length", len_str.as_str()),
                    ("cache-control", "no-store"),
                ];
                let mut resp = req.into_response(200, Some("OK"), &headers)?;
                stream_file_range(&mut resp, &path, 0, size)?;
            }
            Ok(())
        },
    )?;

    // ---- 删除 ----
    server.fn_handler(
        "/api/recording",
        Method::Delete,
        |req| -> Result<(), anyhow::Error> {
            let Some(name) = parse_query_name(req.uri()) else {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"missing name")?;
                return Ok(());
            };
            if !is_safe_name(&name) {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"bad name")?;
                return Ok(());
            }
            let path = format!("{REC_DIR}/{name}");
            // 先清 index(让 list 立刻反映),再去删盘文件;盘删失败也只是孤儿文件,后续可 prune
            recorder::index_remove(&name);
            match fs::remove_file(&path) {
                Ok(()) => {
                    log::info!("deleted recording: {path}");
                    let mut resp = req.into_ok_response()?;
                    resp.write_all(b"{\"ok\":true}")?;
                }
                Err(e) => {
                    log::warn!("delete failed: {path}: {e}");
                    let mut resp = req.into_status_response(500)?;
                    let body = format!("{{\"ok\":false,\"error\":\"{e}\"}}");
                    resp.write_all(body.as_bytes())?;
                }
            }
            Ok(())
        },
    )?;

    Ok(())
}

// ============================================================================
// helpers
// ============================================================================

fn parse_query_uint(q: &str, key: &str) -> Option<u32> {
    for kv in q.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            if k == key {
                return v.parse::<u32>().ok();
            }
        }
    }
    None
}

/// `/api/recording?name=20260425-...wav` → `Some("20260425-...wav")`
fn parse_query_name(uri: &str) -> Option<String> {
    let q = uri.split_once('?')?.1;
    for kv in q.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            if k == "name" {
                return Some(url_decode(v));
            }
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((hi * 16 + lo) as char);
                i += 3;
                continue;
            }
        }
        if b == b'+' {
            out.push(' ');
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 防越权 / 防 ../foo:仅允许扁平 ASCII 文件名,以 .wav 结尾。
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.starts_with('.')
        && name.ends_with(".wav")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `Range: bytes=START-END` → `(start, Some(end))` / `(start, None)`
fn parse_range(h: &str) -> Option<(u64, Option<u64>)> {
    let r = h.strip_prefix("bytes=")?;
    let (s, e) = r.split_once('-')?;
    let start: u64 = s.parse().ok()?;
    let end = if e.is_empty() {
        None
    } else {
        e.parse::<u64>().ok()
    };
    Some((start, end))
}

fn stream_file_range<W>(resp: &mut W, path: &str, start: u64, len: u64) -> Result<()>
where
    W: Write,
    W::Error: core::fmt::Debug,
{
    let mut file = fs::File::open(path)?;
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }
    let mut buf = [0u8; READ_CHUNK];
    let mut remaining = len;
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        resp.write_all(&buf[..n])
            .map_err(|e| anyhow!("response write: {e:?}"))?;
        remaining -= n as u64;
    }
    Ok(())
}

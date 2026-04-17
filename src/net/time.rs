//! 时间格式化(不拉 chrono,省一个依赖)
//!
//! ESP-IDF 的 `time()` 在 SNTP 完成前返回 0(1970-01-01 00:00:00 UTC),
//! 我们用阈值判断"是否已同步"——2020-01-01 前的时间视为未同步。

use core::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// 按固定秒偏移格式化 HH:MM:SS(中国 = +8*3600)。
/// 未同步(Unix epoch < 2020-01-01)返回 None。
pub fn format_local_hms(tz_offset_secs: i64) -> Option<heapless::String<8>> {
    let now = SystemTime::now();
    let unix = now.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    if unix < 1_577_836_800 {
        // 2020-01-01 00:00 UTC;之前一律视为未同步
        return None;
    }
    let local = unix + tz_offset_secs;
    let h = (local / 3600) % 24;
    let m = (local / 60) % 60;
    let s = local % 60;

    let mut out: heapless::String<8> = heapless::String::new();
    let _ = write!(out, "{:02}:{:02}:{:02}", h, m, s);
    Some(out)
}

/// 当前 Unix 秒(未同步返回 None)
pub fn unix_secs() -> Option<u64> {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if unix < 1_577_836_800 {
        None
    } else {
        Some(unix)
    }
}

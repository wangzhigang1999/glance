//! 时间 / 日期格式化(不拉 chrono/time,自己算)
//!
//! ESP-IDF 的 `time()` 在 SNTP 完成前返回 0,阈值 2020-01-01 判"是否已同步"。
//!
//! 日期算法:Howard Hinnant 的 `civil_from_days`(以 1970-01-01 为 0),
//! 见 <https://howardhinnant.github.io/date_algorithms.html>,
//! 对 1970..9999 区间精确,无浮点。

use core::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// 2020-01-01 00:00:00 UTC
const SYNCED_THRESHOLD: i64 = 1_577_836_800;

/// 当前 Unix 秒(未同步返回 None)
pub fn unix_secs() -> Option<i64> {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    if unix < SYNCED_THRESHOLD {
        None
    } else {
        Some(unix)
    }
}

/// HH:MM:SS(本地时区,未同步返回 None)
pub fn format_local_hms(tz_offset_secs: i64) -> Option<heapless::String<8>> {
    let unix = unix_secs()?;
    let local = unix + tz_offset_secs;
    let sec_of_day = local.rem_euclid(86400);
    let h = sec_of_day / 3600;
    let m = (sec_of_day / 60) % 60;
    let s = sec_of_day % 60;

    let mut out: heapless::String<8> = heapless::String::new();
    let _ = write!(out, "{:02}:{:02}:{:02}", h, m, s);
    Some(out)
}

/// YYYY-MM-DD Weekday(本地时区,未同步返回 None)
/// 例:`2026-04-18 SAT`
pub fn format_local_date(tz_offset_secs: i64) -> Option<heapless::String<16>> {
    let unix = unix_secs()?;
    let local = unix + tz_offset_secs;
    let days = local.div_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let wday = weekday_name(days);

    let mut out: heapless::String<16> = heapless::String::new();
    let _ = write!(out, "{:04}-{:02}-{:02} {}", y, m, d, wday);
    Some(out)
}

/// days-since-1970 → (year, month 1..12, day 1..31)
/// 参考:Howard Hinnant, days_from_civil 的反函数
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719468; // 0000-03-01 为第 0 天的偏移
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // 0..146096
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // 0..399
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0..365
    let mp = (5 * doy + 2) / 153; // 0..11(以 3 月为起点)
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

fn weekday_name(days: i64) -> &'static str {
    // 1970-01-01 是星期四(index=4),0=SUN
    let wd = ((days + 4).rem_euclid(7)) as usize;
    ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"][wd]
}

/// (year, month 1..12, day 1..31) → days-since-1970(Hinnant 正向算法)。
/// 对 1970..9999 区间精确,无浮点。RTC 那边互相 round-trip 用。
pub fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 } as i64) / 400;
    let yoe = (y as i64 - era * 400) as u64; // 0..399
    let mp = if m > 2 {
        (m - 3) as u64
    } else {
        (m + 9) as u64
    }; // 0..11
    let doy = (153 * mp + 2) / 5 + (d as u64 - 1); // 0..365
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // 0..146096
    era * 146097 + doe as i64 - 719468
}

/// (Y/M/D h:m:s UTC) → Unix 秒。RTC 写时间用。
pub fn unix_from_utc(y: i32, m: u8, d: u8, h: u8, mi: u8, s: u8) -> i64 {
    days_from_civil(y, m, d) * 86400 + (h as i64) * 3600 + (mi as i64) * 60 + s as i64
}

/// Unix 秒 → (year, month, day, hour, minute, second) UTC。RTC 读出系统时间往回写时用。
pub fn utc_from_unix(unix: i64) -> (i32, u8, u8, u8, u8, u8) {
    let days = unix.div_euclid(86400);
    let sod = unix.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let h = (sod / 3600) as u8;
    let mi = ((sod / 60) % 60) as u8;
    let s = (sod % 60) as u8;
    (y, m, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_thursday_1970() {
        // days=0 → 1970-01-01 THU
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday_name(0), "THU");
    }

    #[test]
    fn known_date_2026_04_18() {
        // 2026-04-18 是星期六(验证过)
        // 距 1970-01-01 的天数:
        //   1970..2025 共 56 年,14 闰年 → 56*365+14 = 20454
        //   2026: 1+31+28+31+17 = 108 天(1-indexed 4-18 的第 107 天后 +1 = 108)
        //   总 20454 + 107 = 20561? 让函数自己算回来
        let (y, m, d) = civil_from_days(20561);
        assert_eq!((y, m, d), (2026, 4, 18));
        assert_eq!(weekday_name(20561), "SAT");
    }
}

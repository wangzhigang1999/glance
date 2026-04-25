//! PCF85063A/T RTC,挂在共享 I2C0 总线 @ 0x51。
//!
//! 寄存器(BCD 编码,**注意 sec 的 bit 7 是 OS 振荡器停止标志**):
//! - 0x00 Control_1(bit 5 = STOP,写时间前置 1 暂停计数)
//! - 0x04 Seconds(bit 7 = OS,断电过/没接备份电池则上电后这位 = 1,时间不可信)
//! - 0x05 Minutes / 0x06 Hours(24h)/ 0x07 Day / 0x08 Weekday(我们不用)
//! - 0x09 Month / 0x0A Year(00-99,以 2000 为基)
//!
//! 板上 ML1220 钮扣电池才能跨电源关闭保活。没装的话每次上电 OS=1 → 等 SNTP。
//!
//! 系统时间集成:
//! - 启动:`sync_to_system()` 读 RTC,合法就 settimeofday()
//! - SNTP 完成后:`sync_from_system()` 读 std::time::SystemTime 写回 RTC

use anyhow::{Context, Result};
use esp_idf_svc::sys::{settimeofday, timeval};

use super::I2cBus;
use crate::net::time::{unix_from_utc, utc_from_unix};

const ADDR: u8 = 0x51;
const REG_CONTROL_1: u8 = 0x00;
const REG_SECONDS: u8 = 0x04;

#[derive(Debug, Clone, Copy)]
pub struct DateTime {
    pub year: i32,  // 完整年(2026 而不是 26)
    pub month: u8,  // 1..12
    pub day: u8,    // 1..31
    pub hour: u8,   // 0..23
    pub minute: u8, // 0..59
    pub second: u8, // 0..59
}

pub struct Rtc {
    bus: I2cBus,
}

impl Rtc {
    pub fn new(bus: I2cBus) -> Self {
        Self { bus }
    }

    /// 读 RTC。返回 None 表示 OS=1(芯片断过电、没备份电池),时间不可信。
    pub fn read(&self) -> Result<Option<DateTime>> {
        let mut buf = [0u8; 7];
        let mut drv = self.bus.lock().expect("i2c bus poisoned");
        drv.write_read(ADDR, &[REG_SECONDS], &mut buf, 100)
            .context("PCF85063 read time")?;
        if buf[0] & 0x80 != 0 {
            return Ok(None);
        }
        Ok(Some(DateTime {
            second: bcd_to_bin(buf[0] & 0x7F),
            minute: bcd_to_bin(buf[1] & 0x7F),
            hour: bcd_to_bin(buf[2] & 0x3F),
            day: bcd_to_bin(buf[3] & 0x3F),
            // buf[4] 是 weekday,我们不用
            month: bcd_to_bin(buf[5] & 0x1F),
            year: 2000 + bcd_to_bin(buf[6]) as i32,
        }))
    }

    /// 写 RTC。STOP → burst 写 7 字节(顺带清 OS bit)→ RUN。
    pub fn set(&self, dt: &DateTime) -> Result<()> {
        let mut drv = self.bus.lock().expect("i2c bus poisoned");
        // STOP 计数,免得写到一半被时钟翻一秒
        drv.write(ADDR, &[REG_CONTROL_1, 0x20], 100)
            .context("PCF85063 STOP")?;
        let yr = ((dt.year - 2000).clamp(0, 99)) as u8;
        let buf = [
            REG_SECONDS,
            bin_to_bcd(dt.second), // bit 7 = 0 → 同时清 OS
            bin_to_bcd(dt.minute),
            bin_to_bcd(dt.hour),
            bin_to_bcd(dt.day),
            0, // weekday,不算
            bin_to_bcd(dt.month),
            bin_to_bcd(yr),
        ];
        drv.write(ADDR, &buf, 100)
            .context("PCF85063 burst write time")?;
        drv.write(ADDR, &[REG_CONTROL_1, 0x00], 100)
            .context("PCF85063 RUN")?;
        Ok(())
    }

    /// 启动期调用:RTC 合法就把它灌进系统时间(settimeofday)。
    /// 返回 true = 灌成功,false = OS bit 置位需要等 SNTP。
    pub fn sync_to_system(&self) -> Result<bool> {
        let Some(dt) = self.read()? else {
            return Ok(false);
        };
        let unix = unix_from_utc(dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second);
        // 健壮性:RTC 偶尔会返回 oxff/全 0,过滤明显跳的
        if dt.year < 2025 || dt.year > 2099 {
            log::warn!(
                "RTC value looks bogus: {}-{:02}-{:02} {:02}:{:02}:{:02}, ignoring",
                dt.year,
                dt.month,
                dt.day,
                dt.hour,
                dt.minute,
                dt.second
            );
            return Ok(false);
        }
        let tv = timeval {
            tv_sec: unix as _,
            tv_usec: 0,
        };
        let ret = unsafe { settimeofday(&tv, core::ptr::null()) };
        if ret != 0 {
            anyhow::bail!("settimeofday failed: {ret}");
        }
        log::info!(
            "RTC → system: {}-{:02}-{:02} {:02}:{:02}:{:02} UTC (unix={})",
            dt.year,
            dt.month,
            dt.day,
            dt.hour,
            dt.minute,
            dt.second,
            unix
        );
        Ok(true)
    }

    /// SNTP 同步成功后调用:把当前系统时间写回 RTC,下次开机不用再等 SNTP。
    pub fn sync_from_system(&self) -> Result<()> {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if unix < 1_577_836_800 {
            anyhow::bail!("system time not synced yet (unix={unix})");
        }
        let (y, m, d, h, mi, s) = utc_from_unix(unix);
        let dt = DateTime {
            year: y,
            month: m,
            day: d,
            hour: h,
            minute: mi,
            second: s,
        };
        self.set(&dt)?;
        log::info!(
            "system → RTC: {}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            y,
            m,
            d,
            h,
            mi,
            s
        );
        Ok(())
    }
}

#[inline]
fn bcd_to_bin(b: u8) -> u8 {
    (b >> 4) * 10 + (b & 0x0F)
}

#[inline]
fn bin_to_bcd(b: u8) -> u8 {
    ((b / 10) << 4) | (b % 10)
}

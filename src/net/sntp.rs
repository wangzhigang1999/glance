//! SNTP 对时(ESP-IDF 官方 sntp 组件,非阻塞)
//!
//! 行为:
//! - `Sntp::start` 立刻返回,后台 poll 多个时间源,第一个响应的生效
//! - 使用 `Immediate` 同步模式:拿到时间直接硬切,UI 可能跳秒但不留负值
//! - 本地时区交给调用方(Linux 风格 TZ env + `time::format_local_hms`)
//!
//! 常用国内 NTP:
//! - ntp.aliyun.com       阿里云
//! - ntp.tencent.com      腾讯云
//! - cn.pool.ntp.org      NTP Pool 中国镜像

use anyhow::Result;
use esp_idf_svc::sntp::{EspSntp, OperatingMode, SntpConf, SyncMode};

pub struct Sntp {
    _inner: EspSntp<'static>,
}

impl Sntp {
    pub fn start() -> Result<Self> {
        let mut servers = ["", "", ""];
        servers[0] = "ntp.aliyun.com";
        servers[1] = "ntp.tencent.com";
        servers[2] = "cn.pool.ntp.org";

        let conf = SntpConf {
            servers,
            operating_mode: OperatingMode::Poll,
            sync_mode: SyncMode::Immediate,
        };
        let inner = EspSntp::new(&conf)?;
        log::info!("SNTP started with {:?}", conf.servers);
        Ok(Self { _inner: inner })
    }

}

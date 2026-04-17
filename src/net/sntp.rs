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
use esp_idf_svc::sntp::{EspSntp, OperatingMode, SntpConf, SyncMode, SyncStatus};
use std::time::{Duration, Instant};

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

    pub fn status(&self) -> SyncStatus {
        self._inner.get_sync_status()
    }

    pub fn is_synced(&self) -> bool {
        matches!(self.status(), SyncStatus::Completed)
    }

    /// 阻塞等待首次对时(用于启动期"UI 显示时间"前兜底)。
    /// 超时返回 false,不 panic——WiFi 若断开也不会卡死主程序。
    pub fn wait_synced(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        while !self.is_synced() {
            if start.elapsed() > timeout {
                log::warn!("SNTP first sync timeout after {:?}", timeout);
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        log::info!(
            "SNTP first sync OK in {} ms",
            start.elapsed().as_millis()
        );
        true
    }
}

//! ESP32-S3 WiFi STA 连接管理
//!
//! 职责:
//! - 持有 `EspWifi`(modem 被它拿走,一旦丢就再也没有 WiFi)
//! - 以 WPA2-Personal 连接固定 AP
//! - 暴露:连接、检查状态、拿 IP、断线重连
//!
//! 注:lifetime 必须 'static——`BlockingWifi<EspWifi<'static>>` 里的事件订阅、
//! DMA 缓冲都活到程序结束,不能绑给 main 的栈帧。

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    ipv4::IpInfo,
    nvs::EspDefaultNvsPartition,
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct WifiCreds {
    pub ssid: heapless::String<32>,
    pub password: heapless::String<64>,
}

impl WifiCreds {
    pub fn new(ssid: &str, password: &str) -> Result<Self> {
        Ok(Self {
            ssid: heapless::String::try_from(ssid).map_err(|_| anyhow!("ssid > 32 bytes"))?,
            password: heapless::String::try_from(password)
                .map_err(|_| anyhow!("password > 64 bytes"))?,
        })
    }
}

pub struct WifiManager {
    wifi: BlockingWifi<EspWifi<'static>>,
}

impl WifiManager {
    pub fn new(
        modem: Modem<'static>,
        sys_loop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
    ) -> Result<Self> {
        let esp_wifi = EspWifi::new(modem, sys_loop.clone(), Some(nvs))
            .context("EspWifi::new failed (modem + nvs)")?;
        let wifi = BlockingWifi::wrap(esp_wifi, sys_loop).context("BlockingWifi::wrap failed")?;
        Ok(Self { wifi })
    }

    /// 一次性连接(成功才返回)。失败会抛错,由调用方决定是否重试。
    pub fn connect(&mut self, creds: &WifiCreds) -> Result<IpInfo> {
        let cfg = Configuration::Client(ClientConfiguration {
            ssid: creds
                .ssid
                .as_str()
                .try_into()
                .map_err(|_| anyhow!("ssid encoding"))?,
            password: creds
                .password
                .as_str()
                .try_into()
                .map_err(|_| anyhow!("password encoding"))?,
            auth_method: AuthMethod::WPA2Personal,
            ..Default::default()
        });
        self.wifi.set_configuration(&cfg)?;

        self.wifi.start().context("wifi.start")?;
        log::info!("wifi started, associating with {}", creds.ssid);

        self.wifi.connect().context("wifi.connect")?;
        log::info!("wifi associated");

        self.wifi
            .wait_netif_up()
            .context("wait_netif_up (DHCP timeout?)")?;

        let ip_info = self.wifi.wifi().sta_netif().get_ip_info()?;
        log::info!(
            "wifi netif up, ip={} gw={} mask={:?}",
            ip_info.ip,
            ip_info.subnet.gateway,
            ip_info.subnet.mask
        );
        Ok(ip_info)
    }

    /// 带指数退避的阻塞式连接:失败后 1s,2s,4s... 上限 30s,一直到成功。
    /// 仅在启动期调用;运行时的重连用 `tick_reconnect`。
    pub fn connect_with_backoff(&mut self, creds: &WifiCreds) -> IpInfo {
        let mut backoff_ms = 1000u64;
        loop {
            match self.connect(creds) {
                Ok(info) => return info,
                Err(e) => {
                    log::warn!("wifi connect failed: {e:#}; retry in {backoff_ms}ms");
                    // 失败后要 stop,不然 set_configuration 会报 ESP_ERR_INVALID_STATE
                    let _ = self.wifi.stop();
                    std::thread::sleep(Duration::from_millis(backoff_ms));
                    backoff_ms = (backoff_ms * 2).min(30_000);
                }
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.wifi.is_connected().unwrap_or(false)
    }

    pub fn ip_info(&self) -> Option<IpInfo> {
        self.wifi.wifi().sta_netif().get_ip_info().ok()
    }
}

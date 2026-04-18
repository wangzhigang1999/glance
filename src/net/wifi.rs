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

    pub fn is_connected(&self) -> bool {
        self.wifi.is_connected().unwrap_or(false)
    }

    pub fn ip_info(&self) -> Option<IpInfo> {
        self.wifi.wifi().sta_netif().get_ip_info().ok()
    }

    /// 当前连接的 AP 信号强度(dBm,负数,越接近 0 越强)
    pub fn rssi(&self) -> Option<i32> {
        self.wifi.wifi().get_rssi().ok()
    }

    /// 失败重试前把栈重置干净。`set_configuration` 二次调用要求 STA 不在 STARTED 态。
    pub fn force_stop(&mut self) -> Result<()> {
        let _ = self.wifi.disconnect();
        let _ = self.wifi.stop();
        Ok(())
    }
}

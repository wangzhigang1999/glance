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
    sys,
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

    /// 扫描周围 AP,把 `stored` 按扫到的 RSSI 强弱排序(扫不到的放尾部兜底)。
    ///
    /// 适用场景:设备离开了"上次连的 AP"所在网络,需要在多个已保存凭据里
    /// 选一个"此时此地能连上"的。扫描耗时约 1-2s,1-slot 情况下调用方应跳过。
    pub fn scan_and_sort(&mut self, stored: &[WifiCreds]) -> Result<Vec<WifiCreds>> {
        // 给个空 STA 配置让 start() 能成功(scan 需要 wifi 在 STARTED 态)
        let cfg = Configuration::Client(ClientConfiguration::default());
        self.wifi
            .set_configuration(&cfg)
            .context("set_configuration (scan)")?;
        self.wifi.start().context("wifi.start (scan)")?;
        let results = self.wifi.scan().context("wifi.scan")?;
        let _ = self.wifi.stop();

        log::info!("scan: {} APs visible", results.len());

        let mut seen: Vec<(WifiCreds, i8)> = Vec::new();
        let mut unseen: Vec<WifiCreds> = Vec::new();
        for c in stored {
            match results
                .iter()
                .find(|ap| ap.ssid.as_str() == c.ssid.as_str())
            {
                Some(ap) => {
                    log::info!("  candidate ssid={} rssi={}dBm", c.ssid, ap.signal_strength);
                    seen.push((c.clone(), ap.signal_strength));
                }
                None => {
                    log::info!("  candidate ssid={} (not in scan, fallback order)", c.ssid);
                    unseen.push(c.clone());
                }
            }
        }
        // RSSI 越大越强(-40 > -80),倒序排 = 强的在前
        seen.sort_by_key(|(_, rssi)| -(*rssi as i32));
        let mut out: Vec<WifiCreds> = seen.into_iter().map(|(c, _)| c).collect();
        out.append(&mut unseen);
        Ok(out)
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

    /// 开 AccessPoint(open, 无密码)供手机连上来配网。
    /// 默认 IP 由 esp-netif 分配(通常 192.168.71.1),DHCP server 自动起。
    ///
    /// 用 AP-only 模式 + 最小化 wifi_ap_config_t —— esp-idf-svc 的
    /// AccessPointConfiguration 填的冗余字段在 IDF 5.5 会被 esp_wifi_set_config 拒 INVALID_ARG。
    pub fn start_ap(&mut self, ssid: &str) -> Result<()> {
        use sys::{
            esp, esp_wifi_set_config, esp_wifi_set_mode, wifi_auth_mode_t_WIFI_AUTH_OPEN,
            wifi_config_t, wifi_interface_t_WIFI_IF_AP, wifi_mode_t_WIFI_MODE_AP,
        };

        log::info!("start_ap: ssid='{ssid}' (open, AP-only)");
        if ssid.len() > 32 {
            return Err(anyhow!("ap ssid > 32 bytes"));
        }

        unsafe {
            esp!(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_AP)).context("esp_wifi_set_mode(AP)")?;
        }

        let mut cfg: wifi_config_t = unsafe { core::mem::zeroed() };
        unsafe {
            let bytes = ssid.as_bytes();
            cfg.ap.ssid[..bytes.len()].copy_from_slice(bytes);
            cfg.ap.ssid_len = bytes.len() as u8;
            cfg.ap.authmode = wifi_auth_mode_t_WIFI_AUTH_OPEN;
            cfg.ap.max_connection = 1;
            esp!(esp_wifi_set_config(wifi_interface_t_WIFI_IF_AP, &mut cfg))
                .context("esp_wifi_set_config(AP)")?;
        }

        self.wifi.start().context("wifi.start (AP)")?;
        // AP 模式不用 wait_netif_up:BlockingWifi.is_up() 要 driver.is_connected(),
        // AP-only 下只要 driver 状态为 Started 就够了,start() 已保证这点。
        log::info!("wifi AP '{ssid}' up");
        Ok(())
    }

    /// 查 AP netif 当前 IP(用 192.168.x.1 这种作为 captive portal 入口地址显示)
    pub fn ap_ip(&self) -> Option<core::net::Ipv4Addr> {
        self.wifi.wifi().ap_netif().get_ip_info().ok().map(|i| i.ip)
    }
}

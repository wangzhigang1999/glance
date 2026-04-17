//! WiFi 凭据 NVS 持久化
//!
//! Namespace:`wifi`
//! Keys:    `ssid`(UTF-8,≤32B)、`password`(UTF-8,≤64B)
//!
//! 失败语义:read 出 None 表示"还没配过",上层应走 BLE 配网;
//!          read 出 Some 但 ssid 空串同样视为 None。

use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

use super::wifi::WifiCreds;

const NAMESPACE: &str = "wifi";
const KEY_SSID: &str = "ssid";
const KEY_PASSWORD: &str = "password";

pub struct CredsStore {
    nvs: EspNvs<NvsDefault>,
}

impl CredsStore {
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self> {
        let nvs = EspNvs::new(partition, NAMESPACE, true)
            .context("open NVS namespace 'wifi'")?;
        Ok(Self { nvs })
    }

    /// 读凭据。Ok(None) = 还没配过
    pub fn load(&self) -> Result<Option<WifiCreds>> {
        let mut ssid_buf = [0u8; 64];
        let mut pwd_buf = [0u8; 128];

        let ssid = self.nvs.get_str(KEY_SSID, &mut ssid_buf)?;
        let pwd = self.nvs.get_str(KEY_PASSWORD, &mut pwd_buf)?;

        match (ssid, pwd) {
            (Some(s), Some(p)) if !s.is_empty() => {
                log::info!("creds loaded from NVS: ssid={s}");
                Ok(Some(WifiCreds::new(s, p)?))
            }
            _ => {
                log::info!("NVS has no wifi creds");
                Ok(None)
            }
        }
    }

    pub fn save(&self, creds: &WifiCreds) -> Result<()> {
        self.nvs.set_str(KEY_SSID, creds.ssid.as_str())?;
        self.nvs.set_str(KEY_PASSWORD, creds.password.as_str())?;
        log::info!("creds saved to NVS (ssid={})", creds.ssid);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn clear(&self) -> Result<()> {
        let _ = self.nvs.remove(KEY_SSID)?;
        let _ = self.nvs.remove(KEY_PASSWORD)?;
        log::warn!("creds cleared from NVS");
        Ok(())
    }
}

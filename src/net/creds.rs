//! WiFi 凭据 NVS 持久化(多凭据版)
//!
//! Namespace:`wifi`
//! Keys:`ssid0..ssid3` + `pwd0..pwd3`(UTF-8,≤32/64B)
//! 容量:[`MAX_SLOTS`] = 4(家/公司/手机热点/旅行)
//!
//! Slot 0 = 最后一次成功连接的凭据;`save` 命中 ssid 则提到 slot 0,
//! 未命中则插入 slot 0 并把最老的挤出 MAX_SLOTS 淘汰。
//!
//! 兼容迁移:旧版单凭据 key(`ssid`/`password`)在首次 `load_all` 时自动搬到
//! slot 0 并删除,保证老设备升级后 WiFi 不掉。

use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

use super::wifi::WifiCreds;

const NAMESPACE: &str = "wifi";
pub const MAX_SLOTS: usize = 4;

const SSID_KEYS: [&str; MAX_SLOTS] = ["ssid0", "ssid1", "ssid2", "ssid3"];
const PWD_KEYS: [&str; MAX_SLOTS] = ["pwd0", "pwd1", "pwd2", "pwd3"];

// 老版单凭据 key(仅迁移用)
const K_LEGACY_SSID: &str = "ssid";
const K_LEGACY_PWD: &str = "password";

pub struct CredsStore {
    nvs: EspNvs<NvsDefault>,
}

impl CredsStore {
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self> {
        let nvs = EspNvs::new(partition, NAMESPACE, true).context("open NVS namespace 'wifi'")?;
        Ok(Self { nvs })
    }

    /// 读所有 slot 里的有效凭据,按 slot 序返回(最新在前)。
    ///
    /// 首次调用若发现老单凭据,会搬到 slot 0 并清旧 key(副作用一次性)。
    pub fn load_all(&self) -> Result<Vec<WifiCreds>> {
        let mut out: Vec<WifiCreds> = Vec::with_capacity(MAX_SLOTS);
        for slot in 0..MAX_SLOTS {
            if let Some(c) = self.read_slot(slot)? {
                out.push(c);
            }
        }
        if out.is_empty() {
            if let Some(legacy) = self.read_legacy()? {
                log::info!("migrating legacy creds (ssid/password) -> slot 0");
                let _ = self.nvs.remove(K_LEGACY_SSID);
                let _ = self.nvs.remove(K_LEGACY_PWD);
                self.write_slot(0, &legacy)?;
                out.push(legacy);
            }
        }
        Ok(out)
    }

    /// 兼容旧接口:等价 `load_all` 的第一个。
    #[allow(dead_code)]
    pub fn load(&self) -> Result<Option<WifiCreds>> {
        Ok(self.load_all()?.into_iter().next())
    }

    /// 保存/升位:命中 ssid → 提到 slot 0,未命中 → 插到 slot 0 并淘汰尾部。
    pub fn save(&self, creds: &WifiCreds) -> Result<()> {
        let mut list = self.load_all()?;
        list.retain(|c| c.ssid.as_str() != creds.ssid.as_str());
        list.insert(0, creds.clone());
        if list.len() > MAX_SLOTS {
            list.truncate(MAX_SLOTS);
        }
        self.write_all(&list)?;
        log::info!("creds saved: ssid={} slots={}", creds.ssid, list.len());
        Ok(())
    }

    /// 按 ssid 删除某个 slot(给 /settings 的移除按钮用)。
    pub fn remove(&self, ssid: &str) -> Result<()> {
        let mut list = self.load_all()?;
        let before = list.len();
        list.retain(|c| c.ssid.as_str() != ssid);
        if list.len() == before {
            return Ok(());
        }
        self.write_all(&list)?;
        log::info!("creds removed: ssid={} remaining={}", ssid, list.len());
        Ok(())
    }

    /// 清全部 slot(Switch WiFi 用)。
    pub fn clear(&self) -> Result<()> {
        for slot in 0..MAX_SLOTS {
            let _ = self.nvs.remove(SSID_KEYS[slot]);
            let _ = self.nvs.remove(PWD_KEYS[slot]);
        }
        let _ = self.nvs.remove(K_LEGACY_SSID);
        let _ = self.nvs.remove(K_LEGACY_PWD);
        log::warn!("all wifi creds cleared from NVS");
        Ok(())
    }

    // ---- 内部 ----

    fn read_slot(&self, slot: usize) -> Result<Option<WifiCreds>> {
        let mut ssid_buf = [0u8; 64];
        let mut pwd_buf = [0u8; 128];
        let ssid = self.nvs.get_str(SSID_KEYS[slot], &mut ssid_buf)?;
        let pwd = self.nvs.get_str(PWD_KEYS[slot], &mut pwd_buf)?;
        match (ssid, pwd) {
            (Some(s), Some(p)) if !s.is_empty() => Ok(Some(WifiCreds::new(s, p)?)),
            _ => Ok(None),
        }
    }

    fn read_legacy(&self) -> Result<Option<WifiCreds>> {
        let mut ssid_buf = [0u8; 64];
        let mut pwd_buf = [0u8; 128];
        let ssid = self.nvs.get_str(K_LEGACY_SSID, &mut ssid_buf)?;
        let pwd = self.nvs.get_str(K_LEGACY_PWD, &mut pwd_buf)?;
        match (ssid, pwd) {
            (Some(s), Some(p)) if !s.is_empty() => Ok(Some(WifiCreds::new(s, p)?)),
            _ => Ok(None),
        }
    }

    fn write_slot(&self, slot: usize, c: &WifiCreds) -> Result<()> {
        self.nvs.set_str(SSID_KEYS[slot], c.ssid.as_str())?;
        self.nvs.set_str(PWD_KEYS[slot], c.password.as_str())?;
        Ok(())
    }

    /// 按 list 顺序写 slot 0..N,尾部多余 slot 清掉防残留。
    fn write_all(&self, list: &[WifiCreds]) -> Result<()> {
        for (i, c) in list.iter().enumerate() {
            self.write_slot(i, c)?;
        }
        for slot in list.len()..MAX_SLOTS {
            let _ = self.nvs.remove(SSID_KEYS[slot]);
            let _ = self.nvs.remove(PWD_KEYS[slot]);
        }
        Ok(())
    }
}

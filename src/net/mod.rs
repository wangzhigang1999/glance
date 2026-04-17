//! 网络栈:WiFi 连接 + NTP 对时
//!
//! Phase 1:SSID/密码硬编码,验证链路
//! Phase 2:凭据从 NVS 读
//! Phase 3:BLE 配网写 NVS

pub mod creds;
pub mod prov;
pub mod sntp;
pub mod time;
pub mod wifi;

pub use creds::CredsStore;
pub use prov::{ProvStatus, Provisioner};
pub use sntp::Sntp;
pub use time::{format_local_date, format_local_hms};
pub use wifi::{WifiCreds, WifiManager};

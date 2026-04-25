//! 网络栈:WiFi 连接 + NTP 对时
//!
//! Phase 1:SSID/密码硬编码,验证链路
//! Phase 2:凭据从 NVS 读
//! Phase 3:SoftAP + HTTP 门户配网写 NVS

pub mod activity;
pub mod creds;
mod gh_http;
pub mod gh_worker;
pub mod github;
pub mod log_sink;
pub mod notifications;
pub mod prov_ap;
pub mod screen_http;
pub mod sntp;
pub mod time;
pub mod wifi;

pub use creds::CredsStore;
pub use prov_ap::Provisioner;
pub use sntp::Sntp;
pub use time::{format_local_date, format_local_hms};
pub use wifi::{WifiCreds, WifiManager};

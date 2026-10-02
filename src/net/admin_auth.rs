//! Management authentication for device enrollment, independent of firmware updates.
use super::admin_policy as policy;
use anyhow::Result;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs};
use std::sync::OnceLock;
static KEY: OnceLock<String> = OnceLock::new();
pub fn initialize(partition: EspDefaultNvsPartition) -> Result<()> {
    let nvs = EspNvs::new(
        partition, "ota_auth",
        true, // Legacy namespace: preserve the installed management credential.
    )?;
    let mut buf = [0u8; 64];
    let saved = nvs.get_str("key", &mut buf)?.map(str::to_owned);
    let key = match saved {
        Some(k) => k,
        None => {
            let seed: serde_json::Value =
                serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/admin_auth.json")))?;
            let k = seed["password"].as_str().unwrap_or("").to_owned();
            if policy::valid_key(&k) {
                nvs.set_str("key", &k)?;
            }
            k
        }
    };
    let _ = KEY.set(key);
    Ok(())
}

pub fn authorized(candidate: &str) -> bool {
    KEY.get()
        .is_some_and(|key| policy::authorized(key, candidate))
}

//! Secret configuration is validated as a whole before one atomic NVS write.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::Ipv4Addr};

pub const MAX_BYTES: usize = 12_000;
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub ip: Ipv4Addr,
    pub did: u32,
    pub token: String,
    pub properties: Vec<Property>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub key: String,
    pub siid: u32,
    pub piid: u32,
    #[serde(default)]
    pub legacy: String,
}
pub fn validate(devices: &[Device]) -> Result<()> {
    ensure!(devices.len() <= 16, "At most 16 devices");
    let mut ids = BTreeSet::new();
    for d in devices {
        ensure!(
            d.did != 0 && d.id == format!("mi_{}", d.did) && ids.insert(d.did),
            "Invalid identity"
        );
        ensure!(
            d.ip.is_private() && !d.ip.is_broadcast(),
            "Private address required"
        );
        ensure!(
            !d.name.is_empty() && d.name.len() <= 96 && d.model.len() <= 96,
            "Invalid name"
        );
        ensure!(
            d.token.len() == 32 && d.token.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid key"
        );
        ensure!(d.properties.len() <= 10, "Too many properties");
        let mut keys = BTreeSet::new();
        for p in &d.properties {
            ensure!(
                !p.key.is_empty()
                    && p.key.len() <= 48
                    && p.key
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    && keys.insert(&p.key),
                "Invalid property key"
            );
            ensure!(
                p.siid <= 255
                    && p.piid <= 255
                    && (p.legacy.is_empty()
                        || (p.legacy.len() <= 32
                            && p.legacy
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'_'))),
                "Invalid read property"
            );
            ensure!(
                p.legacy.is_empty() == d.properties[0].legacy.is_empty(),
                "Mixed protocols"
            );
        }
    }
    ensure!(
        serde_json::to_vec(devices)?.len() <= MAX_BYTES,
        "Configuration too large"
    );
    Ok(())
}
/// Update by durable device ID; never drop an existing device on partial import.
pub fn merged(current: &[Device], incoming: Vec<Device>) -> Result<Vec<Device>> {
    ensure!(!incoming.is_empty(), "Select at least one device");
    validate(&incoming)?;
    let mut next = current.to_vec();
    for device in incoming {
        if let Some(existing) = next.iter_mut().find(|d| d.did == device.did) {
            *existing = device;
        } else {
            next.push(device);
        }
    }
    validate(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(did: u32) -> Device {
        Device {
            id: format!("mi_{did}"),
            did,
            name: "test".into(),
            model: "test.model".into(),
            ip: "192.168.1.24".parse().unwrap(),
            token: "a".repeat(32),
            properties: vec![],
        }
    }
    #[test]
    fn merge_preserves_devices_and_replaces_credentials_without_duplicates() {
        let before = vec![device(1), device(2)];
        let mut moved = device(1);
        moved.ip = "10.0.0.8".parse().unwrap();
        let after = merged(&before, vec![moved, device(3)]).unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(after[0].ip.to_string(), "10.0.0.8");
        assert_eq!(before[0].ip.to_string(), "192.168.1.24");
    }
    #[test]
    fn rejects_identity_collision_public_address_and_capacity_overflow() {
        assert!(validate(&[device(1), device(1)]).is_err());
        let mut d = device(1);
        d.ip = "8.8.8.8".parse().unwrap();
        assert!(validate(&[d]).is_err());
        let full: Vec<_> = (1..=16).map(device).collect();
        assert!(merged(&full, vec![device(17)]).is_err());
    }
}

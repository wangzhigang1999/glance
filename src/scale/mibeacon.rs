//! Authenticated MiBeacon v4/v5 receiver for the owner's clock only.
use std::{collections::VecDeque, time::Instant};

use esp_idf_svc::sys;
use serde::Serialize;

pub const ADDRESS: &str = "A4:C1:38:67:37:31";
pub const MAC: [u8; 6] = [0xa4, 0xc1, 0x38, 0x67, 0x37, 0x31];
const KEYS: &str = include_str!(concat!(env!("OUT_DIR"), "/clock_keys.json"));

#[derive(Clone, Debug, Serialize)]
pub struct Reading {
    pub value: f32,
    pub unix_secs: Option<i64>,
    #[serde(skip)]
    pub received: Instant,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub configured: bool,
    pub authenticated: u64,
    pub rejected: u64,
    pub temperature: Option<Reading>,
    pub humidity: Option<Reading>,
    pub battery: Option<Reading>,
}
pub struct Decoder {
    key: Option<[u8; 16]>,
    seen: VecDeque<([u8; 12], Instant)>,
}
impl Decoder {
    pub fn new() -> Self {
        if !self_test() {
            log::error!("Clock AES-CCM self-test failed; receiver disabled");
            return Self {
                key: None,
                seen: VecDeque::new(),
            };
        }
        log::info!("Clock AES-CCM known-vector and tamper checks passed");
        let key = serde_json::from_str::<serde_json::Value>(KEYS)
            .ok()
            .and_then(|v| v.get(ADDRESS)?.as_str().map(str::to_owned))
            .and_then(|s| {
                if s.len() != 32 || !s.is_ascii() {
                    return None;
                }
                let mut key = [0; 16];
                for (i, byte) in key.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
                }
                Some(key)
            });
        Self {
            key,
            seen: VecDeque::new(),
        }
    }
    pub fn configured(&self) -> bool {
        self.key.is_some()
    }
    pub fn receive(&mut self, adv: &[u8], state: &mut Snapshot) {
        let Some(key) = self.key else { return };
        let Some(data) = service_data(adv) else {
            return;
        };
        match decrypt(data, &key) {
            Some((nonce, values)) => {
                while self
                    .seen
                    .front()
                    .is_some_and(|(_, t)| t.elapsed().as_secs() > 3600)
                {
                    self.seen.pop_front();
                }
                if self.seen.iter().any(|(n, _)| *n == nonce) {
                    return;
                }
                if self.seen.len() == 256 {
                    self.seen.pop_front();
                }
                self.seen.push_back((nonce, Instant::now()));
                state.authenticated += 1;
                let unix_secs = crate::net::time::unix_secs();
                for (kind, value) in values {
                    let reading = Some(Reading {
                        value,
                        unix_secs,
                        received: Instant::now(),
                    });
                    match kind {
                        0 => state.temperature = reading,
                        1 => state.humidity = reading,
                        2 => state.battery = reading,
                        _ => {}
                    }
                }
            }
            None => state.rejected += 1,
        }
    }
}
fn self_test() -> bool {
    // Synthetic public test vector, never a household key or measurement.
    let key = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut frame = [
        88, 80, 66, 37, 7, 49, 55, 103, 56, 193, 164, 24, 12, 62, 132, 162, 178, 106, 175, 241,
        151, 226, 1, 2, 3, 212, 12, 94, 80,
    ];
    let valid = decrypt(&frame, &key)
        .is_some_and(|(_, v)| v.len() == 2 && (v[0].1 - 25.3).abs() < 0.001 && v[1] == (1, 61.0));
    frame[28] ^= 1;
    valid && decrypt(&frame, &key).is_none() && parse_objects(&[1, 76, 4, 0]).is_none()
}
fn service_data(mut adv: &[u8]) -> Option<&[u8]> {
    while let Some(&len) = adv.first() {
        let end = len as usize + 1;
        if len == 0 || end > adv.len() {
            return None;
        }
        let field = &adv[1..end];
        if field.starts_with(&[0x16, 0x95, 0xfe]) {
            return Some(&field[3..]);
        }
        adv = &adv[end..];
    }
    None
}
fn decrypt(data: &[u8], key: &[u8; 16]) -> Option<([u8; 12], Vec<(u8, f32)>)> {
    if data.len() < 14 {
        return None;
    }
    let control = u16::from_le_bytes([data[0], data[1]]);
    if ![4, 5].contains(&(control >> 12)) || control & 8 == 0 {
        return None;
    }
    let reversed = [MAC[5], MAC[4], MAC[3], MAC[2], MAC[1], MAC[0]];
    let mut pos = 5;
    if control & 16 != 0 {
        if data.get(pos..pos + 6)? != reversed {
            return None;
        }
        pos += 6;
    }
    if control & 32 != 0 {
        pos += 1 + if *data.get(pos)? & 32 != 0 { 2 } else { 0 };
    }
    if data.len() < pos + 9 {
        return None;
    }
    let mut nonce = [0; 12];
    nonce[..6].copy_from_slice(&reversed);
    nonce[6..9].copy_from_slice(&data[2..5]);
    nonce[9..].copy_from_slice(&data[data.len() - 7..data.len() - 4]);
    let cipher = &data[pos..data.len() - 7];
    let mut plain = vec![0u8; cipher.len()];
    let result = unsafe {
        let mut ctx = std::mem::zeroed::<sys::mbedtls_ccm_context>();
        sys::mbedtls_ccm_init(&mut ctx);
        let mut result = sys::mbedtls_ccm_setkey(
            &mut ctx,
            sys::mbedtls_cipher_id_t_MBEDTLS_CIPHER_ID_AES,
            key.as_ptr(),
            128,
        );
        if result == 0 {
            result = sys::mbedtls_ccm_auth_decrypt(
                &mut ctx,
                cipher.len(),
                nonce.as_ptr(),
                nonce.len(),
                [0x11u8].as_ptr(),
                1,
                cipher.as_ptr(),
                plain.as_mut_ptr(),
                data[data.len() - 4..].as_ptr(),
                4,
            );
        }
        sys::mbedtls_ccm_free(&mut ctx);
        result
    };
    if result != 0 {
        return None;
    }
    Some((nonce, parse_objects(&plain)?))
}
fn parse_objects(plain: &[u8]) -> Option<Vec<(u8, f32)>> {
    let mut pos = 0;
    let mut result = Vec::new();
    while pos < plain.len() {
        let header = plain.get(pos..pos + 3)?;
        let kind = u16::from_le_bytes([header[0], header[1]]);
        let size = header[2] as usize;
        pos += 3;
        let value = plain.get(pos..pos + size)?;
        pos += size;
        let decoded = match (kind, size) {
            (0x4c01, 4) => Some((0, f32::from_le_bytes(value.try_into().ok()?))),
            (0x4c02, 1) => Some((1, value[0] as f32)),
            (0x4803 | 0x4c03, 1) => Some((2, value[0] as f32)),
            _ => None,
        };
        if let Some((kind, v)) = decoded {
            if !v.is_finite()
                || !(if kind == 0 {
                    -80.0..=150.0
                } else {
                    0.0..=100.0
                })
                .contains(&v)
            {
                return None;
            }
            result.push((kind, v));
        }
    }
    Some(result)
}

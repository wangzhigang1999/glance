//! Compact local history with a seven-calendar-day retention window (UTC+8).
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub const CAPACITY: usize = 128;
const RECORD_BYTES: usize = 21;
pub const MAX_BYTES: usize = 6 + CAPACITY * RECORD_BYTES;
const MAGIC: &[u8; 4] = b"WH01";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(default)]
    pub event_id: String,
    pub kg: f32,
    pub unix_secs: Option<i64>,
}

/// Keep today and the previous six local dates; unknown timestamps remain bounded.
pub fn retain_week(history: &mut Vec<Measurement>, now: Option<i64>) -> bool {
    let before = history.len();
    let cutoff = now.map(|t| ((t + 8 * 3600).div_euclid(86400) - 6) * 86400 - 8 * 3600);
    history.retain(|r| {
        r.kg.is_finite()
            && (1.0..=200.0).contains(&r.kg)
            && cutoff
                .zip(r.unix_secs)
                .is_none_or(|(cutoff, t)| t >= cutoff)
    });
    if history.len() > CAPACITY {
        history.drain(..history.len() - CAPACITY);
    }
    before != history.len()
}

pub fn encode(history: &[Measurement]) -> Result<Vec<u8>> {
    ensure!(history.len() <= CAPACITY, "history capacity exceeded");
    let mut bytes = Vec::with_capacity(6 + history.len() * RECORD_BYTES);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(history.len() as u16).to_le_bytes());
    for reading in history {
        let id = if reading.event_id.is_empty() {
            0
        } else {
            ensure!(reading.event_id.len() == 16, "invalid history event id");
            u64::from_str_radix(&reading.event_id, 16)?
        };
        bytes.push(u8::from(!reading.event_id.is_empty()));
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&reading.kg.to_le_bytes());
        bytes.extend_from_slice(&reading.unix_secs.unwrap_or(i64::MIN).to_le_bytes());
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Measurement>> {
    if !bytes.starts_with(MAGIC) {
        // Migrate the installed JSON records without changing event identities or values.
        return Ok(serde_json::from_slice(bytes)?);
    }
    ensure!(bytes.len() >= 6, "truncated history header");
    let count = u16::from_le_bytes(bytes[4..6].try_into()?) as usize;
    ensure!(
        count <= CAPACITY && bytes.len() == 6 + count * RECORD_BYTES,
        "invalid history length"
    );
    bytes[6..]
        .chunks_exact(RECORD_BYTES)
        .map(|row| {
            ensure!(row[0] <= 1, "invalid history flags");
            let id = u64::from_le_bytes(row[1..9].try_into()?);
            let timestamp = i64::from_le_bytes(row[13..21].try_into()?);
            Ok(Measurement {
                event_id: if row[0] == 0 {
                    String::new()
                } else {
                    format!("{id:016x}")
                },
                kg: f32::from_le_bytes(row[9..13].try_into()?),
                unix_secs: (timestamp != i64::MIN).then_some(timestamp),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(timestamp: Option<i64>) -> Measurement {
        Measurement {
            event_id: "000000000000abcd".into(),
            kg: 56.75,
            unix_secs: timestamp,
        }
    }
    #[test]
    fn migrates_json_and_preserves_exact_binary_values() {
        let mut records = vec![record(Some(1790820047)), record(None)];
        records[1].event_id.clear();
        let legacy = serde_json::to_vec(&records).unwrap();
        let migrated = decode(&legacy).unwrap();
        let encoded = encode(&migrated).unwrap();
        assert_eq!(encoded.len(), 6 + 2 * RECORD_BYTES);
        assert_eq!(
            serde_json::to_value(decode(&encoded).unwrap()).unwrap(),
            serde_json::to_value(records).unwrap()
        );
        assert!(decode(&encoded[..encoded.len() - 1]).is_err());
    }
    #[test]
    fn seven_local_dates_and_capacity_are_bounded() {
        let today = 100 * 86400 - 8 * 3600;
        let oldest = today - 6 * 86400;
        let mut records = vec![
            record(Some(oldest - 1)),
            record(Some(oldest)),
            record(Some(today)),
            record(None),
        ];
        assert!(retain_week(&mut records, Some(today)));
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].unix_secs, Some(oldest));
        let mut many = vec![record(Some(today)); CAPACITY + 1];
        assert!(retain_week(&mut many, None));
        assert_eq!(many.len(), CAPACITY);
        assert_eq!(encode(&many).unwrap().len(), MAX_BYTES);
    }
}

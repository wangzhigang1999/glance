//! Version-compatible durable weight queue. Never evict an unacknowledged item.
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

pub const QUEUE_LIMIT: usize = 24;
#[derive(Clone, Serialize, Deserialize)]
pub struct Pending {
    pub key: String,
    pub payload: String,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Durable {
    pub seen: Vec<String>,
    pub pending: Option<Pending>,
    #[serde(default)]
    pub queue: VecDeque<Pending>,
}
impl Durable {
    pub fn contains(&self, key: &str) -> bool {
        self.seen.iter().any(|k| k == key)
            || self.pending.as_ref().is_some_and(|p| p.key == key)
            || self.queue.iter().any(|p| p.key == key)
    }
    pub fn enqueue(&mut self, record: Pending) -> bool {
        if self.contains(&record.key) || self.queue.len() >= QUEUE_LIMIT {
            return false;
        }
        self.queue.push_back(record);
        true
    }
    pub fn promote(&mut self) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = self.queue.pop_front();
        self.pending.is_some()
    }
    pub fn acknowledge(&mut self) {
        if let Some(p) = self.pending.take() {
            self.seen.push(p.key);
            if self.seen.len() > 32 {
                self.seen.remove(0);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn item(i: usize) -> Pending {
        Pending {
            key: i.to_string(),
            payload: format!("{{\"boot_id\":\"original\",\"seq\":{i}}}"),
        }
    }
    #[test]
    fn migrates_existing_pending_and_retries_exact_payload() {
        let old = r#"{"seen":["old"],"pending":{"key":"1","payload":"original bytes"}}"#;
        let state: Durable = serde_json::from_str(old).unwrap();
        let restored: Durable =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(restored.pending.unwrap().payload, "original bytes");
        assert!(restored.queue.is_empty());
    }
    #[test]
    fn offline_queue_does_not_evict_and_deduplicates() {
        let mut state = Durable::default();
        for n in 0..24 {
            assert!(state.enqueue(item(n)));
        }
        assert!(!state.enqueue(item(24)));
        assert!(!state.enqueue(item(2)));
        assert!(state.promote());
        assert!(!state.promote());
        assert_eq!(state.pending.as_ref().unwrap().key, "0");
        assert!(state.enqueue(item(24)));
        let mut restored: Durable =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        for n in 0..25 {
            restored.promote();
            assert_eq!(restored.pending.as_ref().unwrap().payload, item(n).payload);
            restored.acknowledge();
            assert!(restored.contains(&n.to_string()));
        }
        assert!(!restored.promote());
    }
}

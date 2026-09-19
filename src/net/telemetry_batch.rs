//! MQTT telemetry envelope limits; samples remain byte-for-byte unchanged.
use std::collections::VecDeque;
pub const MAX_RECORDS: usize = 60;
pub const MAX_BYTES: usize = 16 * 1024;
pub fn take_batch(queue: &mut VecDeque<String>) -> Result<Option<String>, &'static str> {
    if queue.front().is_some_and(|s| s.len() + 2 > MAX_BYTES) {
        return Err("Telemetry sample exceeds batch limit");
    }
    let mut payload = String::from("[");
    let mut count = 0;
    while let Some(item) = queue.front() {
        let separator = usize::from(count > 0);
        if count == MAX_RECORDS || payload.len() + separator + item.len() + 1 > MAX_BYTES {
            break;
        }
        if separator > 0 {
            payload.push(',');
        }
        payload.push_str(&queue.pop_front().unwrap());
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    payload.push(']');
    Ok(Some(payload))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_original_samples_and_order() {
        let mut q = VecDeque::from([
            r#"{"seq":1,"ts_ms":123}"#.into(),
            r#"{"seq":2,"ts_ms":456}"#.into(),
        ]);
        assert_eq!(
            take_batch(&mut q).unwrap().unwrap(),
            r#"[{"seq":1,"ts_ms":123},{"seq":2,"ts_ms":456}]"#
        );
        assert!(take_batch(&mut q).unwrap().is_none());
    }
    #[test]
    fn limits_record_count() {
        let mut q = VecDeque::from(vec!["{}".into(); 61]);
        let b = take_batch(&mut q).unwrap().unwrap();
        assert_eq!(b.matches("{}").count(), 60);
        assert_eq!(q.len(), 1);
    }
    #[test]
    fn limits_encoded_bytes_without_splitting_samples() {
        let item = format!("{{\"x\":\"{}\"}}", "汉".repeat(3000));
        let mut q = VecDeque::from([item.clone(), item.clone()]);
        assert_eq!(take_batch(&mut q).unwrap().unwrap(), format!("[{item}]"));
        assert_eq!(q.len(), 1);
        let mut q = VecDeque::from(["x".repeat(MAX_BYTES - 2)]);
        assert_eq!(take_batch(&mut q).unwrap().unwrap().len(), MAX_BYTES);
        q.push_back("x".repeat(MAX_BYTES - 1));
        assert!(take_batch(&mut q).is_err());
        assert_eq!(q.len(), 1);
    }
}

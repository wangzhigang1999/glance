//! Pure validation helpers shared by device services.
pub fn truncate_utf8(s: &mut String, max: usize) {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0xffu8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_limits() {
        for max in 0..12 {
            let mut s = "ab中文🙂z".to_owned();
            truncate_utf8(&mut s, max);
            assert!(s.len() <= max);
        }
        let mut s = format!("{}你", "a".repeat(38));
        truncate_utf8(&mut s, 39);
        assert_eq!(s.len(), 38);
    }
    #[test]
    fn sensor_crc_vector() {
        assert_eq!(crc8(&[0xbe, 0xef]), 0x92);
        assert_ne!(crc8(&[0xbe, 0xee]), 0x92);
    }
}

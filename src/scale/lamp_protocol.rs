//! CBL1600B module-one packets, validated against physical-device captures.
pub const BATTERY_QUERY: [u8; 6] = [0xde, 6, 0xa4, 0, 0xa2, 0xed];

/// The device reports coarse battery levels, not a precise fuel-gauge percentage.
pub fn battery(data: &[u8]) -> Option<u8> {
    if data.len() != 19 || data[0..4] != [0xde, 19, 0xb4, 0] || data[18] != 0xed {
        return None;
    }
    let checksum = data[1..17].iter().fold(0, |acc, byte| acc ^ byte);
    (checksum == data[17] && data[8] <= 100).then_some(data[8])
}

pub fn packet(level: u8) -> Option<[u8; 20]> {
    if level > 100 {
        return None;
    }
    let mut result = [
        0xde,
        20,
        0xa2,
        1,
        1,
        1,
        1,
        level,
        0,
        if level > 0 { 1 } else { 0 },
        if level > 0 { 0x50 } else { 0 },
        0,
        0,
        0,
        0,
        0,
        0,
        0xbb,
        0,
        0xed,
    ];
    result[18] = result[1..18].iter().fold(0, |acc, byte| acc ^ byte);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn battery_accepts_capture_and_rejects_corruption() {
        let mut data = [
            0xde, 19, 0xb4, 0, 0, 0, 0, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0xc3, 0xed,
        ];
        assert_eq!(battery(&data), Some(100));
        data[8] = 50;
        assert_eq!(battery(&data), None);
        data[17] = data[1..17].iter().fold(0, |a, b| a ^ b);
        assert_eq!(battery(&data), Some(50));
        data[8] = 101;
        data[17] = data[1..17].iter().fold(0, |a, b| a ^ b);
        assert_eq!(battery(&data), None);
        assert_eq!(battery(&data[..10]), None);
        assert_eq!(battery(&[0xde, 7, 0xb6, 0, 1, 0xb0, 0xed]), None);
    }
    #[test]
    fn matches_physically_verified_captures() {
        for (level, expected) in [
            (0, "de14a20101010100000000000000000000bb0ded"),
            (10, "de14a2010101010a000150000000000000bb56ed"),
            (25, "de14a20101010119000150000000000000bb45ed"),
        ] {
            assert_eq!(
                packet(level)
                    .unwrap()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
        }
        assert!(packet(101).is_none());
    }
}

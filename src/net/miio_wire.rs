//! Bounds and padding checks for untrusted miIO UDP datagrams.
pub const MAX_PACKET: usize = 2048;
pub fn valid_packet(packet: &[u8], did: u32, encrypted: bool) -> bool {
    packet.len() >= 32
        && packet.len() <= MAX_PACKET
        && packet[..2] == [0x21, 0x31]
        && u16::from_be_bytes([packet[2], packet[3]]) as usize == packet.len()
        && u32::from_be_bytes(packet[8..12].try_into().unwrap()) == did
        && (!encrypted || (packet.len() >= 48 && (packet.len() - 32) % 16 == 0))
}
pub fn unpad(bytes: &[u8]) -> Option<&[u8]> {
    let n = *bytes.last()? as usize;
    if n == 0
        || n > 16
        || n > bytes.len()
        || !bytes[bytes.len() - n..].iter().all(|b| *b as usize == n)
    {
        return None;
    }
    Some(&bytes[..bytes.len() - n])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncated_foreign_and_oversized_packets() {
        let mut p = [0u8; 48];
        p[..4].copy_from_slice(&[0x21, 0x31, 0, 48]);
        p[8..12].copy_from_slice(&42u32.to_be_bytes());
        assert!(valid_packet(&p, 42, true));
        assert!(!valid_packet(&p, 43, true));
        for n in 0..48 {
            assert!(!valid_packet(&p[..n], 42, true));
        }
        p[3] = 47;
        assert!(!valid_packet(&p, 42, true));
        assert!(!valid_packet(&vec![0; MAX_PACKET + 1], 42, true));
    }
    #[test]
    fn padding_is_strict() {
        assert_eq!(unpad(b"abc\x01"), Some(&b"abc"[..]));
        assert_eq!(unpad(&[16; 16]), Some(&b""[..]));
        for bad in [
            &b""[..],
            &b"abc\x00"[..],
            &b"abc\x02"[..],
            &[17; 17][..],
            &[2][..],
        ] {
            assert!(unpad(bad).is_none());
        }
    }
}

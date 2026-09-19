//! Xiaomi 0x181D service data. Based on bleparser/miscale.py and captured packets.
pub const MIN_KG: f32 = 1.0;

pub fn valid_weight(kg: f32) -> bool {
    kg.is_finite() && (MIN_KG..=200.0).contains(&kg)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub kg: f32,
    pub stable: bool,
    pub removed: bool,
}

pub fn decode(data: &[u8]) -> Option<Reading> {
    if data.len() != 10 {
        return None;
    }
    let flags = data[0];
    let raw = u16::from_le_bytes([data[1], data[2]]) as f32;
    let kg = if flags & 1 != 0 {
        raw / 100.0 * 0.45359237
    } else {
        raw / 200.0
    };
    if kg > 200.0 {
        return None;
    }
    Some(Reading {
        kg,
        stable: flags & 0x20 != 0,
        removed: flags & 0x80 != 0,
    })
}

pub fn advertisement(mut adv: &[u8]) -> Option<Reading> {
    while let Some(&len) = adv.first() {
        let end = usize::from(len) + 1;
        if len == 0 || end > adv.len() {
            return None;
        }
        let field = &adv[1..end];
        if field.len() >= 3 && field[..3] == [0x16, 0x1d, 0x18] {
            return decode(&field[3..]);
        }
        adv = &adv[end..];
    }
    None
}

/// One record per weighing; repeated unloaded advertisements do not re-arm it.
#[derive(Default)]
pub struct Session {
    saved: bool,
    last_seen_ms: Option<u64>,
}
impl Session {
    pub fn accept(&mut self, r: Reading, now_ms: u64) -> bool {
        if self
            .last_seen_ms
            .is_some_and(|last| now_ms.saturating_sub(last) > 90_000)
        {
            self.saved = false;
        }
        self.last_seen_ms = Some(now_ms);
        if r.removed || r.kg == 0.0 {
            self.saved = false;
            return false;
        }
        if !valid_weight(r.kg) || !r.stable {
            return false;
        }
        if self.saved {
            return false;
        }
        self.saved = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_weight_and_session_reset() {
        for kg in [0.0, 0.1, 0.995, f32::NAN, f32::INFINITY] {
            assert!(!valid_weight(kg));
        }
        assert!(valid_weight(1.0));
        let mut s = Session::default();
        let r = Reading {
            kg: 0.995,
            stable: true,
            removed: false,
        };
        assert!(!s.accept(r, 0));
        assert!(s.accept(Reading { kg: 1.0, ..r }, 100));
        assert!(!s.accept(r, 200));
        assert!(!s.accept(Reading { kg: 1.0, ..r }, 300));
        assert!(!s.accept(Reading { kg: 0.0, ..r }, 400));
        assert!(s.accept(Reading { kg: 1.0, ..r }, 500));
        assert!(!s.accept(Reading { removed: true, ..r }, 600));
        assert!(s.accept(Reading { kg: 70.0, ..r }, 700));
    }
    #[test]
    fn captured_packets_and_units() {
        let mut p = [0x62, 0x0e, 0x06, 0xb2, 0x07, 3, 11, 0, 21, 58];
        assert_eq!(
            decode(&p),
            Some(Reading {
                kg: 7.75,
                stable: true,
                removed: false
            })
        );
        p[0] = 0xa2;
        assert!(decode(&p).unwrap().removed);
        p[0] = 2;
        assert!(!decode(&p).unwrap().stable);
        p[0] = 0x32;
        p[1..3].copy_from_slice(&14000u16.to_le_bytes());
        assert_eq!(decode(&p).unwrap().kg, 70.0); // 140 Chinese jin
        p[0] = 0x23;
        p[1..3].copy_from_slice(&10000u16.to_le_bytes());
        assert!((decode(&p).unwrap().kg - 45.359237).abs() < 0.0001);
    }
    #[test]
    fn advertisement_boundaries() {
        let p = [
            2, 1, 6, 13, 0x16, 0x1d, 0x18, 0x22, 0x4c, 9, 0xb2, 7, 3, 11, 0, 22, 28,
        ];
        assert!((advertisement(&p).unwrap().kg - 11.9).abs() < 0.0001);
        for i in 0..p.len() {
            assert!(advertisement(&p[..i]).is_none());
        }
        assert!(decode(&[0; 9]).is_none());
        assert!(advertisement(&[255, 0x16]).is_none());
    }
    #[test]
    fn deduplicate_repeats_and_new_sessions() {
        let mut s = Session::default();
        let r = Reading {
            kg: 70.0,
            stable: true,
            removed: false,
        };
        assert!(s.accept(r, 0));
        assert!(!s.accept(r, 100));
        assert!(!s.accept(Reading { stable: false, ..r }, 200));
        assert!(!s.accept(r, 300));
        assert!(!s.accept(Reading { removed: true, ..r }, 400));
        assert!(s.accept(r, 500));
        assert!(s.accept(r, 100_501));
    }
}

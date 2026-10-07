//! LP910 emits a pointer positioning sequence, then click/swipe reports.
//! Classify a whole quiet-delimited burst, never an isolated shared click.
#[derive(Default)]
pub struct Gesture {
    reports: Vec<String>,
    overflow: bool,
    released: bool,
}
impl Gesture {
    pub fn push(&mut self, handle: u16, value: &[u8]) {
        if handle == 23 && value == [0, 0, 0] {
            self.released = true;
        } else if value.iter().any(|b| *b != 0) {
            self.released = false;
        }
        if value.is_empty() || value.iter().all(|b| *b == 0) {
            return;
        }
        let key = format!(
            "{handle}:{}",
            value.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        if self.reports.contains(&key) {
            return;
        }
        if self.reports.len() >= 16 || value.len() > 16 {
            self.overflow = true;
            return;
        }
        self.reports.push(key);
    }
    pub fn finish(&mut self) -> Option<String> {
        self.released = false;
        let overflow = std::mem::take(&mut self.overflow);
        let mut reports = std::mem::take(&mut self.reports);
        if overflow || reports.is_empty() {
            return None;
        }
        reports.sort();
        let key = reports.join(";");
        (key.len() <= 384).then_some(key)
    }
    pub fn quiet_ms(&self) -> u64 {
        // Keep two 60 ms report intervals after release so a late swipe
        // cannot be mistaken for the shared center click.
        if self.released {
            120
        } else {
            300
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn center(g: &mut Gesture) {
        for _ in 0..5 {
            g.push(31, &[1, 0xf8, 0x7f]);
        }
        g.push(31, &[0xaa, 0xf0, 0xf0]);
        g.push(23, &[1, 0, 0]);
    }
    #[test]
    fn captured_swipe_must_not_match_shared_center_click() {
        let mut g = Gesture::default();
        center(&mut g);
        g.push(23, &[0, 0, 0]);
        let center = g.finish().unwrap();
        center_reports(&mut g);
        g.push(31, &[2, 0x30, 0]);
        g.push(31, &[0, 0x20, 3]);
        g.push(31, &[0, 0x20, 3]);
        g.push(23, &[0, 0, 0]);
        let up = g.finish().unwrap();
        assert_ne!(center, up);
        g.push(23, &[0, 0, 0]);
        assert!(g.finish().is_none());
    }
    fn center_reports(g: &mut Gesture) {
        center(g);
    }
    #[test]
    fn duplicate_packets_and_notification_order_do_not_change_gesture() {
        let mut g = Gesture::default();
        center(&mut g);
        let first = g.finish();
        g.push(23, &[1, 0, 0]);
        g.push(31, &[0xaa, 0xf0, 0xf0]);
        g.push(31, &[1, 0xf8, 0x7f]);
        assert_eq!(first, g.finish());
    }
    #[test]
    fn overlong_or_incomplete_buffer_never_maps_to_partial_gesture() {
        let mut g = Gesture::default();
        for i in 1..20 {
            g.push(i, &[1]);
        }
        assert!(g.finish().is_none());
    }
    #[test]
    fn release_shortens_wait_but_late_swipe_restores_full_wait() {
        let mut g = Gesture::default();
        center(&mut g);
        assert_eq!(g.quiet_ms(), 300);
        g.push(23, &[0, 0, 0]);
        assert_eq!(g.quiet_ms(), 120);
        g.push(31, &[2, 0x30, 0]);
        assert_eq!(g.quiet_ms(), 300);
        g.push(23, &[0, 0, 0]);
        g.finish();
        assert_eq!(g.quiet_ms(), 300);
    }
}

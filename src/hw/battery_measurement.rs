//! Voltage diagnostics only; no remaining-capacity estimate.

pub fn trimmed_adc_mv(mut samples: [u16; 16]) -> u32 {
    samples.sort_unstable();
    (samples[4..12].iter().map(|&v| u32::from(v)).sum::<u32>() + 4) / 8
}

pub fn terminal_mv(adc_mv: u32) -> Option<u32> {
    let mv = adc_mv.checked_mul(3)?;
    (2500..=4500).contains(&mv).then_some(mv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adc_outliers_do_not_dominate_voltage() {
        let mut samples = [1300; 16];
        samples[..4].fill(0);
        samples[12..].fill(u16::MAX);
        assert_eq!(trimmed_adc_mv(samples), 1300);
        assert_eq!(terminal_mv(trimmed_adc_mv(samples)), Some(3900));
    }

    #[test]
    fn invalid_or_disconnected_adc_is_not_a_power_source() {
        assert_eq!(terminal_mv(0), None);
        assert_eq!(terminal_mv(800), None);
        assert_eq!(terminal_mv(1501), None);
        assert_eq!(terminal_mv(u32::MAX), None);
    }
}

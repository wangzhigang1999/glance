use std::thread::sleep;
use std::time::Duration;

use esp_idf_svc::hal::adc::attenuation::DB_6;
use esp_idf_svc::hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_svc::hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::sys::link_patches;

fn main() -> anyhow::Result<()> {
    link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;

    let adc = AdcDriver::new(peripherals.adc1)?;
    let config = AdcChannelConfig {
        attenuation: DB_6,
        calibration: Calibration::Curve, // ESP32-S3 用 Curve Fitting 校准
        ..Default::default()
    };
    let mut ch = AdcChannelDriver::new(&adc, peripherals.pins.gpio4, &config)?;

    log::info!("Battery ADC demo: GPIO4, divider 1:3, atten DB_6, Curve calibration");

    loop {
        let mut sum: u32 = 0;
        for _ in 0..16 {
            sum += ch.read()? as u32;
        }
        let adc_mv = sum / 16;
        let vbat_mv = adc_mv * 3;
        let percent = estimate_percent(vbat_mv);

        log::info!(
            "ADC = {} mV | VBAT ≈ {} mV ({:.2} V) | ~{} %",
            adc_mv,
            vbat_mv,
            vbat_mv as f32 / 1000.0,
            percent
        );

        sleep(Duration::from_secs(1));
    }
}

/// 3.0V = 0%,4.2V = 100%,线性
fn estimate_percent(vbat_mv: u32) -> u8 {
    if vbat_mv <= 3000 {
        0
    } else if vbat_mv >= 4200 {
        100
    } else {
        (((vbat_mv - 3000) * 100) / 1200) as u8
    }
}

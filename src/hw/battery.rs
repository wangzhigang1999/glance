//! 18650 电池电量检测
//!
//! 硬件:板上把电池正极接 1/3 分压(两等值电阻串联)到 GPIO4,
//!       ADC 读到的电压 × 3 = 实际电池电压。
//!
//! 锂电映射(简化线性):
//!   4.20V ~ 100%,3.70V ~ 50%,3.00V ~ 0%
//! 严格讲锂电 SoC 是非线性 OCV 曲线,线性够日常展示用了,
//! 要精确需要挂 MAX17048 / BQ27xxx 之类电量计。
//!
//! USB 供电但电池未插:ADC 读到 0 或漂动。阈值 < 2.5V 判 USB 模式。

use anyhow::{Context, Result};
use esp_idf_svc::hal::adc::attenuation::DB_12;
use esp_idf_svc::hal::adc::oneshot::{
    config::{AdcChannelConfig, Calibration},
    AdcChannelDriver, AdcDriver,
};
use esp_idf_svc::hal::adc::{ADC1, ADCCH3, ADCU1};
use esp_idf_svc::hal::gpio::Gpio4;

const DIVIDER: f32 = 3.0;
const USB_THRESHOLD_MV: u32 = 2500;
const VMAX_MV: f32 = 4200.0;
const VMIN_MV: f32 = 3000.0;

#[derive(Debug, Clone, Copy)]
pub enum PowerSource {
    /// 电池供电:mV + 百分比(0-100)
    Battery { mv: u32, percent: u8 },
    /// USB 供电或电池未插
    Usb,
}

/// 电池采样器。AdcDriver 通过 `Box::leak` 取 'static 借用,
/// 避免自引用结构体。嵌入式里程序不退出,主动泄漏无害。
type Adc1Static = AdcDriver<'static, ADCU1>;

pub struct Battery {
    pin: AdcChannelDriver<'static, ADCCH3<ADCU1>, &'static Adc1Static>,
}

impl Battery {
    pub fn new(adc1: ADC1<'static>, gpio4: Gpio4<'static>) -> Result<Self> {
        let adc = AdcDriver::new(adc1).context("AdcDriver::new(adc1)")?;
        let adc: &'static mut Adc1Static = Box::leak(Box::new(adc));

        let config = AdcChannelConfig {
            attenuation: DB_12,
            calibration: Calibration::Curve,
            ..Default::default()
        };
        let pin = AdcChannelDriver::new(&*adc, gpio4, &config)
            .context("AdcChannelDriver::new(gpio4)")?;
        Ok(Self { pin })
    }

    /// 读一次电量,8 次平均去抖动。
    pub fn read(&mut self) -> Result<PowerSource> {
        let mut sum: u32 = 0;
        for _ in 0..8 {
            sum += self.pin.read()? as u32;
        }
        let adc_mv = sum / 8;
        let bat_mv = (adc_mv as f32 * DIVIDER) as u32;

        if bat_mv < USB_THRESHOLD_MV {
            return Ok(PowerSource::Usb);
        }
        let pct = (((bat_mv as f32 - VMIN_MV) / (VMAX_MV - VMIN_MV)) * 100.0)
            .clamp(0.0, 100.0) as u8;
        Ok(PowerSource::Battery {
            mv: bat_mv,
            percent: pct,
        })
    }
}

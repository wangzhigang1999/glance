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
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::Gpio4;

/// 检测 USB 主机是否插着。原理:USB 主机每 1ms 下发一个 SOF 帧,ESP32-S3 的
/// USB_SERIAL_JTAG 外设把最近收到的帧号记在 `JFIFO_ST.sof_frame_index`(11 bit),
/// 只要主机在枚举/活跃,计数就一直涨。间隔 3ms 读两次,值变了就是插着主机。
/// 仅插充电器不接主机的情况下没有 SOF,会被判成电池供电——这种场景靠电压看也
/// 没意义(正在充电,电压不稳),按电池模式展示即可。
fn usb_host_present() -> bool {
    // USB_SERIAL_JTAG_FRAM_NUM_REG = DR_REG_USB_SERIAL_JTAG_BASE + 0x24
    // 低 11 位 = 最近收到的 SOF 帧号
    const FRAM_NUM_REG: *const u32 = 0x6003_8024 as *const u32;
    const SOF_FRAME_INDEX_MASK: u32 = 0x7FF;
    unsafe {
        let a = FRAM_NUM_REG.read_volatile() & SOF_FRAME_INDEX_MASK;
        FreeRtos::delay_ms(3);
        let b = FRAM_NUM_REG.read_volatile() & SOF_FRAME_INDEX_MASK;
        a != b
    }
}

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

    /// 读一次电量,8 次平均去抖动。优先通过 USB SOF 判断主机是否插着;
    /// 主机在枚举时不管电池充没充满都算 USB 模式。
    pub fn read(&mut self) -> Result<PowerSource> {
        if usb_host_present() {
            return Ok(PowerSource::Usb);
        }

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

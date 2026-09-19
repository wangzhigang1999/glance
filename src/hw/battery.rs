//! 18650 电池电量检测
//!
//! 硬件:200K/100K 分压到 GPIO4,
//!       ADC 读到的电压 × 3 = 实际电池电压。
//!
//! 仅测电压,不推算剩余容量;USB 主机连接时也继续采样。

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    adc::{
        attenuation::DB_12,
        oneshot::{
            config::{AdcChannelConfig, Calibration},
            AdcChannelDriver, AdcDriver,
        },
        ADC1, ADCCH3, ADCU1,
    },
    delay::FreeRtos,
    gpio::Gpio4,
};

use super::battery_measurement::{terminal_mv, trimmed_adc_mv};

/// 检测 USB 主机是否插着。原理:USB 主机每 1ms 下发一个 SOF 帧,ESP32-S3 的
/// USB_SERIAL_JTAG 外设把最近收到的帧号记在 `JFIFO_ST.sof_frame_index`(11 bit),
/// 只要主机在枚举/活跃,计数就一直涨。间隔 3ms 读两次,值变了就是插着主机。
/// 仅插充电器不接主机的情况下没有 SOF,会被判成电池供电——这种场景靠电压看也
/// 无法可靠识别;该方法不代表充电状态或满电状态。
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

const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
pub enum PowerSource {
    /// 未检测到 USB 主机,且电压在有效范围内
    Battery,
    /// 检测到活跃 USB 主机,不代表已充满
    Usb,
}

/// 电池采样器。AdcDriver 通过 `Box::leak` 取 'static 借用,
/// 避免自引用结构体。嵌入式里程序不退出,主动泄漏无害。
type Adc1Static = AdcDriver<'static, ADCU1>;

pub struct Battery {
    pin: AdcChannelDriver<'static, ADCCH3<ADCU1>, &'static Adc1Static>,
    sampled_at: Option<Instant>,
    adc_mv: Option<u32>,
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
        let pin =
            AdcChannelDriver::new(&*adc, gpio4, &config).context("AdcChannelDriver::new(gpio4)")?;
        Ok(Self {
            pin,
            sampled_at: None,
            adc_mv: None,
        })
    }

    /// ADC 每 5 秒采样一次,不随称重页面刷新加速;去掉两端异常值。
    /// 不跨时间平滑电压,避免掩盖低电压。切换供电后的电压最多滞后 5 秒。
    pub fn read(&mut self) -> Result<PowerSource> {
        let usb = usb_host_present();
        if self
            .sampled_at
            .is_none_or(|at| at.elapsed() >= SAMPLE_INTERVAL)
        {
            self.sampled_at = Some(Instant::now());
            self.adc_mv = None;
            let mut samples = [0u16; 16];
            for sample in &mut samples {
                *sample = self.pin.read()?;
            }
            self.adc_mv = Some(trimmed_adc_mv(samples));
        }
        if usb {
            return Ok(PowerSource::Usb);
        }
        self.voltage_mv()
            .context("battery voltage unavailable/out of range")?;
        Ok(PowerSource::Battery)
    }

    pub fn adc_mv(&self) -> Option<u32> {
        self.adc_mv
    }

    /// 包括 USB 模式下的端电压;有读数不代表电池存在或充满。
    pub fn voltage_mv(&self) -> Option<u32> {
        self.adc_mv.and_then(terminal_mv)
    }
}

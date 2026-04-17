//! SHTC3 温湿度传感器(I2C 0x70)。
//!
//! 协议(datasheet §5):
//! - Wake-up: 0x3517
//! - Measure T first, high-rep, no clock stretching: 0x7866
//!   等待 ≥12.1ms 后读 6 字节 (T_MSB, T_LSB, T_CRC, RH_MSB, RH_LSB, RH_CRC)
//! - Sleep: 0xB098

use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    gpio::{InputPin, OutputPin},
    i2c::{I2c, I2cConfig, I2cDriver},
    units::Hertz,
};

const ADDR: u8 = 0x70;
const CMD_WAKEUP: [u8; 2] = [0x35, 0x17];
const CMD_MEASURE_TFIRST: [u8; 2] = [0x78, 0x66];
const CMD_SLEEP: [u8; 2] = [0xB0, 0x98];

pub struct Shtc3<'d> {
    drv: I2cDriver<'d>,
}

impl<'d> Shtc3<'d> {
    pub fn new<I2C, Sda, Scl>(i2c: I2C, sda: Sda, scl: Scl) -> Result<Self>
    where
        I2C: I2c + 'd,
        Sda: InputPin + OutputPin + 'd,
        Scl: InputPin + OutputPin + 'd,
    {
        let cfg = I2cConfig::new().baudrate(Hertz(100_000));
        let drv = I2cDriver::new(i2c, sda, scl, &cfg).context("i2c init")?;
        Ok(Self { drv })
    }

    /// 返回 (温度°C, 湿度%)。内部完整 wakeup → measure → sleep。
    pub fn read(&mut self) -> Result<(f32, f32)> {
        self.drv.write(ADDR, &CMD_WAKEUP, 100).context("wakeup")?;
        sleep(Duration::from_micros(240));

        self.drv
            .write(ADDR, &CMD_MEASURE_TFIRST, 100)
            .context("measure cmd")?;
        sleep(Duration::from_millis(15));

        let mut buf = [0u8; 6];
        self.drv.read(ADDR, &mut buf, 100).context("read raw")?;

        let t_raw = u16::from_be_bytes([buf[0], buf[1]]);
        let rh_raw = u16::from_be_bytes([buf[3], buf[4]]);
        let t_c = -45.0 + 175.0 * (t_raw as f32) / 65536.0;
        let rh = 100.0 * (rh_raw as f32) / 65536.0;

        let _ = self.drv.write(ADDR, &CMD_SLEEP, 100);
        Ok((t_c, rh))
    }
}

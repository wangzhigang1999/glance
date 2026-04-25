//! 麦克风 I2S RX 通道(ESP32-S3 作 master,ES7210 是 slave)。
//!
//! 引脚:
//! - MCLK = GPIO16(ESP → ES7210)
//! - BCLK = GPIO9
//! - LRCK = GPIO45(WS)
//! - DIN  = GPIO10(ES7210 SDOUT1 → ESP)
//!
//! 配置:I2S Philips、16kHz、16-bit、stereo 槽(L=R=MIC1,见 ES7210 reg 0x4B)。
//! 上层取 mono 直接用 left 声道即可。

use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    gpio::{Gpio10, Gpio16, Gpio45, Gpio9},
    i2s::{
        config::{DataBitWidth, StdConfig},
        I2sDriver, I2sRx, I2S0,
    },
};

use super::es7210::{MCLK_MULTIPLE as _MCLK_MULTIPLE, SAMPLE_RATE_HZ};

pub struct Mic {
    drv: I2sDriver<'static, I2sRx>,
}

impl Mic {
    /// 创建并配置 I2S RX 通道。还没开始采样,需要 `start()` 才会喂时钟。
    pub fn new(
        i2s0: I2S0<'static>,
        mclk: Gpio16<'static>,
        bclk: Gpio9<'static>,
        ws: Gpio45<'static>,
        din: Gpio10<'static>,
    ) -> Result<Self> {
        // philips() 默认:Controller(master)+ stereo + MCLK=256×fs(默认 PLL_F160M 时钟源)。
        // 16kHz × 256 = 4.096 MHz,落在 ES7210 推荐范围。
        let cfg = StdConfig::philips(SAMPLE_RATE_HZ, DataBitWidth::Bits16);
        let drv = I2sDriver::<I2sRx>::new_std_rx(i2s0, &cfg, bclk, din, Some(mclk), ws)
            .context("I2sDriver::new_std_rx")?;
        log::info!("I2S RX init OK (sr={SAMPLE_RATE_HZ}Hz, 16bit, philips, mclk×{_MCLK_MULTIPLE})");
        Ok(Self { drv })
    }

    /// 启动 I2S RX,MCLK/BCLK/LRCK 开始驱动 ES7210。
    pub fn start(&mut self) -> Result<()> {
        self.drv.rx_enable().context("i2s rx_enable")?;
        Ok(())
    }

    /// 阻塞读一帧到 16-bit PCM buffer。返回采样点数(stereo 下 L+R 算两点)。
    /// timeout_ms = 1000 通常够用,采样率 16kHz 下 1024 点 ≈ 32ms。
    pub fn read(&mut self, buf: &mut [i16], timeout_ms: u32) -> Result<usize> {
        let bytes: &mut [u8] =
            unsafe { core::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 2) };
        let n_bytes = self.drv.read(bytes, timeout_ms).context("i2s read")?;
        Ok(n_bytes / 2)
    }
}

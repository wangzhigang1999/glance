//! ST7305 SPI 驱动(底层字节协议)
//!
//! 硬件接线(见 `docs/10-pinout.md`):
//!  - MOSI = GPIO12 (SDA)
//!  - SCK  = GPIO11 (SCL)
//!  - CS   = GPIO40  (由 SPI device driver 管)
//!  - DC   = GPIO5   (RS,手动控:低=命令,高=数据)
//!  - RST  = GPIO41  (手动控)
//!  - TE   = GPIO6   (本驱动暂未使用)
//!
//! **没有 MISO**,单向写入,不支持 Read ID 验证。
//!
//! SPI 配置:10 MHz,mode 0,MSB first。

use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    delay::FreeRtos,
    gpio::{InputPin, Output, OutputPin, PinDriver},
    spi::{
        config::{Config as SpiConfig, DriverConfig as SpiDriverConfig},
        Dma, SpiAnyPins, SpiDeviceDriver, SpiDriver,
    },
    units::Hertz,
};

struct InitStep {
    cmd: u8,
    data: &'static [u8],
    delay_ms: u32,
}

/// 完整 init 序列(翻译自 Waveshare `display_bsp.cpp`)
const INIT_SEQUENCE: &[InitStep] = &[
    InitStep {
        cmd: 0xD6,
        data: &[0x17, 0x02],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xD1,
        data: &[0x01],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xC0,
        data: &[0x11, 0x04],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xC1,
        data: &[0x69, 0x69, 0x69, 0x69],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xC2,
        data: &[0x19, 0x19, 0x19, 0x19],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xC4,
        data: &[0x4B, 0x4B, 0x4B, 0x4B],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xC5,
        data: &[0x19, 0x19, 0x19, 0x19],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xD8,
        data: &[0x80, 0xE9],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB2,
        data: &[0x02],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB3,
        data: &[0xE5, 0xF6, 0x05, 0x46, 0x77, 0x77, 0x77, 0x77, 0x76, 0x45],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB4,
        data: &[0x05, 0x46, 0x77, 0x77, 0x77, 0x77, 0x76, 0x45],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x62,
        data: &[0x32, 0x03, 0x1F],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB7,
        data: &[0x13],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB0,
        data: &[0x64],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x11,
        data: &[],
        delay_ms: 200,
    },
    InitStep {
        cmd: 0xC9,
        data: &[0x00],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x36,
        data: &[0x48],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x3A,
        data: &[0x11],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB9,
        data: &[0x20],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xB8,
        data: &[0x29],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x21,
        data: &[],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x2A,
        data: &[0x12, 0x2A],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x2B,
        data: &[0x00, 0xC7],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x35,
        data: &[0x00],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0xD0,
        data: &[0xFF],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x38,
        data: &[],
        delay_ms: 0,
    },
    InitStep {
        cmd: 0x29,
        data: &[],
        delay_ms: 0,
    },
];

const WINDOW_COL: (u8, u8, u8) = (0x2A, 0x12, 0x2A);
const WINDOW_ROW: (u8, u8, u8) = (0x2B, 0x00, 0xC7);
const CMD_RAMWR: u8 = 0x2C;

pub struct St7305<'d> {
    spi: SpiDeviceDriver<'d, SpiDriver<'d>>,
    dc: PinDriver<'d, Output>,
    rst: PinDriver<'d, Output>,
}

impl<'d> St7305<'d> {
    /// 构造驱动。所有 pin 可直接传 `peripherals.pins.gpioX`。
    pub fn new<SPI, Mosi, Sclk, Cs, Dc, Rst>(
        spi_peripheral: SPI,
        mosi: Mosi,
        sclk: Sclk,
        cs: Cs,
        dc: Dc,
        rst: Rst,
    ) -> Result<Self>
    where
        SPI: SpiAnyPins + 'd,
        Mosi: OutputPin + 'd,
        Sclk: OutputPin + 'd,
        Cs: OutputPin + 'd,
        Dc: OutputPin + 'd,
        Rst: OutputPin + 'd,
    {
        let cfg = SpiConfig::new()
            .baudrate(Hertz(10_000_000))
            .data_mode(esp_idf_svc::hal::spi::config::MODE_0);

        // 开 DMA,max_transfer_size 要大于一次送的字节数(15000)。设 16384 稳妥。
        let bus_cfg = SpiDriverConfig::new().dma(Dma::Auto(16384));
        let spi = SpiDeviceDriver::new_single(
            spi_peripheral,
            sclk,
            mosi,
            None::<esp_idf_svc::hal::gpio::AnyIOPin>,
            Some(cs),
            &bus_cfg,
            &cfg,
        )
        .context("spi_device new")?;

        let mut dc_pin = PinDriver::output(dc).context("dc pin")?;
        let mut rst_pin = PinDriver::output(rst).context("rst pin")?;
        dc_pin.set_high().ok();
        rst_pin.set_high().ok();

        Ok(Self {
            spi,
            dc: dc_pin,
            rst: rst_pin,
        })
    }

    /// 硬复位:50ms 高 → 20ms 低 → 50ms 高
    pub fn reset(&mut self) -> Result<()> {
        self.rst.set_high()?;
        FreeRtos::delay_ms(50);
        self.rst.set_low()?;
        FreeRtos::delay_ms(20);
        self.rst.set_high()?;
        FreeRtos::delay_ms(50);
        Ok(())
    }

    /// 发完整 init 序列。调用前应 `reset()`。
    pub fn send_init_sequence(&mut self) -> Result<()> {
        for step in INIT_SEQUENCE {
            self.write_cmd(step.cmd)?;
            if !step.data.is_empty() {
                self.write_data(step.data)?;
            }
            if step.delay_ms > 0 {
                FreeRtos::delay_ms(step.delay_ms);
            }
        }
        Ok(())
    }

    /// 把 framebuffer 全量写到 GRAM。
    pub fn write_frame(&mut self, frame: &[u8]) -> Result<()> {
        self.write_cmd(WINDOW_COL.0)?;
        self.write_data(&[WINDOW_COL.1, WINDOW_COL.2])?;
        self.write_cmd(WINDOW_ROW.0)?;
        self.write_data(&[WINDOW_ROW.1, WINDOW_ROW.2])?;
        self.write_cmd(CMD_RAMWR)?;
        self.write_data(frame)?;
        Ok(())
    }

    fn write_cmd(&mut self, cmd: u8) -> Result<()> {
        self.dc.set_low()?;
        self.spi.write(&[cmd]).context("spi write cmd")?;
        Ok(())
    }

    fn write_data(&mut self, data: &[u8]) -> Result<()> {
        self.dc.set_high()?;
        // DMA 模式下最大 16384,我们 framebuffer 15000 一次过
        self.spi.write(data).context("spi write data")?;
        Ok(())
    }
}

// 为了让 InputPin trait 约束链条在编译期可推导,我们让这个 unused import 在必要时可用
#[allow(dead_code)]
fn _ensure_trait_in_scope<T: InputPin>() {}

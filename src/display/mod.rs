//! ST7305 反射式单色 LCD 显示模块
//!
//! 板子:Waveshare ESP32-S3-RLCD-4.2,300×400 px,1-bit/pixel,双稳态。
//! 硬件:SPI2 + 手动控 DC/RST,10 MHz SCK,无 MISO(单向)。
//!
//! 对外暴露 [`Display`]:实现 `embedded-graphics` 的 `DrawTarget<Color=BinaryColor>`,
//! 任何 embedded-graphics 生态的原语、字体、图片都能直接画进来。
//!
//! 调用流程:
//! ```ignore
//! let mut display = Display::new(spi, pins)?;
//! display.init()?;
//! display.clear(BinaryColor::Off);   // 全白
//! Text::new("Hi", Point::new(10, 20), style).draw(&mut display)?;
//! display.flush()?;                  // 真正送到 ST7305
//! ```

pub mod framebuffer;
pub mod st7305;

use anyhow::Result;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::{Dimensions, DrawTarget, OriginDimensions, Size};
use embedded_graphics::Pixel;

pub use framebuffer::{FrameBuffer, HEIGHT, WIDTH};
pub use st7305::St7305;

/// 集成"ST7305 底层驱动 + framebuffer"的高层 Display。
///
/// 绘制 API 走 `embedded_graphics::DrawTarget`,内部写到 framebuffer;
/// 调 [`Display::flush`] 才真正把 framebuffer 送到屏。
pub struct Display<'d> {
    driver: St7305<'d>,
    fb: FrameBuffer,
}

impl<'d> Display<'d> {
    pub fn new(driver: St7305<'d>) -> Self {
        Self {
            driver,
            fb: FrameBuffer::new(),
        }
    }

    /// 硬 reset + 发完整 init 序列 + 清屏为白。
    pub fn init(&mut self) -> Result<()> {
        self.driver.reset()?;
        self.driver.send_init_sequence()?;
        self.fb.fill(BinaryColor::Off); // 全白
        self.flush()
    }

    /// 启动自检 / 残影清除:整屏 N 次黑-白闪烁。
    /// 双稳态液晶的顽固态这样能消,顺便肉眼验证像素映射。
    pub fn splash_flash(&mut self, cycles: u32) -> Result<()> {
        use esp_idf_svc::hal::delay::FreeRtos;
        for _ in 0..cycles {
            self.fb.fill(BinaryColor::On); // 全黑
            self.flush()?;
            FreeRtos::delay_ms(400);
            self.fb.fill(BinaryColor::Off); // 全白
            self.flush()?;
            FreeRtos::delay_ms(400);
        }
        Ok(())
    }

    /// 把 framebuffer 全量送到 ST7305。
    pub fn flush(&mut self) -> Result<()> {
        self.driver.write_frame(self.fb.raw())?;
        Ok(())
    }

    /// 暴露 fb 原始字节供镜屏/调试使用(不改动)
    pub fn fb_raw(&self) -> &[u8] {
        self.fb.raw()
    }
}

impl<'d> OriginDimensions for Display<'d> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl<'d> DrawTarget for Display<'d> {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let bounds = self.bounding_box();
        for Pixel(p, color) in pixels {
            if bounds.contains(p) {
                self.fb.set_pixel(p.x as u16, p.y as u16, color);
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.fb.fill(color);
        Ok(())
    }
}


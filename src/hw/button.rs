//! KEY 按键输入(软件去抖 + 下降沿触发)
//!
//! 电气:板上 KEY 按钮接 GPIO18,按下拉低(按钮另一端接 GND),内部上拉。
//!
//! 逻辑:连续 3 次(~30ms)都读到 low 才算按下;释放后才能再次触发。

use anyhow::Result;
use esp_idf_svc::hal::gpio::{AnyIOPin, Input, PinDriver, Pull};

pub struct Button {
    pin: PinDriver<'static, Input>,
    stable_low_count: u8,
    was_pressed: bool,
}

impl Button {
    pub fn new(pin: AnyIOPin<'static>) -> Result<Self> {
        let driver = PinDriver::input(pin, Pull::Up)?;
        Ok(Self {
            pin: driver,
            stable_low_count: 0,
            was_pressed: false,
        })
    }

    /// 轮询一次(调用方 ~100ms 调一次)。
    /// 返回 true 只在"按下沿"的那一 tick。
    pub fn poll_pressed(&mut self) -> bool {
        let low = self.pin.is_low();
        if low {
            if self.stable_low_count < 3 {
                self.stable_low_count += 1;
            }
        } else {
            self.stable_low_count = 0;
            self.was_pressed = false;
        }

        let currently_pressed = self.stable_low_count >= 3;
        let edge = currently_pressed && !self.was_pressed;
        if currently_pressed {
            self.was_pressed = true;
        }
        edge
    }
}

//! 按键输入(极简边沿触发,不去抖)
//!
//! 首次调用时记录"空闲电平"(认为没按),之后**任意方向**的电平变化都视为"按下沿",
//! 触发一次。释放(回到 idle)才能再次触发。
//!
//! 100ms tick 下不额外去抖 ——机械抖动 < 50ms,对 100ms 采样没啥影响,
//! 反而之前 3×100ms 的去抖让快按触发不了。

use anyhow::Result;
use esp_idf_svc::hal::gpio::{AnyIOPin, Input, PinDriver, Pull};

pub struct Button {
    pin: PinDriver<'static, Input>,
    idle_level: Option<bool>,
    was_pressed: bool,
}

impl Button {
    pub fn new(pin: AnyIOPin<'static>) -> Result<Self> {
        let driver = PinDriver::input(pin, Pull::Up)?;
        Ok(Self {
            pin: driver,
            idle_level: None,
            was_pressed: false,
        })
    }

    /// 返回 true 只在"按下沿"的那一 tick。
    pub fn poll_pressed(&mut self) -> bool {
        let high = self.pin.is_high();

        let idle = match self.idle_level {
            Some(l) => l,
            None => {
                self.idle_level = Some(high);
                log::info!(
                    "Button idle level latched: {}",
                    if high { "HIGH" } else { "LOW" }
                );
                return false;
            }
        };

        let active = high != idle;

        let edge = active && !self.was_pressed;
        self.was_pressed = active;
        edge
    }
}

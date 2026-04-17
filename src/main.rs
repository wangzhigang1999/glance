//! 温湿度计主程序
//!
//! 流程:
//! 1. 初始化 SHTC3(I2C) 和 Display(SPI)
//! 2. 启屏 + 画一张初始(温湿度 = 无数据)
//! 3. 循环:读 SHTC3 → 更新 AppState → 重绘 → 每 5 秒一次
//!
//! 引脚按 `docs/10-pinout.md`。

mod display;
mod hw;
mod ui;

use std::thread::sleep;
use std::time::{Duration, Instant};

use esp_idf_svc::hal::gpio::AnyOutputPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::sys::link_patches;

use crate::display::{Display, St7305};
use crate::hw::shtc3::Shtc3;
use crate::ui::AppState;

fn main() -> anyhow::Result<()> {
    link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("=== ESP32-S3-RLCD-4.2 温湿度计 ===");

    let peripherals = Peripherals::take()?;

    // ---- Display ----
    log::info!("Init ST7305 display");
    let st7305 = St7305::new(
        peripherals.spi2,
        peripherals.pins.gpio12, // MOSI / SDA
        peripherals.pins.gpio11, // SCK / SCL
        peripherals.pins.gpio40, // CS
        AnyOutputPin::from(peripherals.pins.gpio5),  // DC
        AnyOutputPin::from(peripherals.pins.gpio41), // RST
    )?;
    let mut display = Display::new(st7305);
    display.init()?;
    log::info!("Display init OK, 400x300 landscape framebuffer ready");

    // 上电自检:黑白闪 3 次,清掉双稳态残影 + 验证全屏填充
    log::info!("Splash flash x3 (clear any residual)");
    display.splash_flash(3)?;
    log::info!("Splash done");

    let mut state = AppState::default();
    let _ = ui::render(&mut display, &state);
    display.flush()?;
    log::info!("Initial frame flushed");

    // ---- SHTC3 ----
    log::info!("Init SHTC3 sensor (I2C SDA=13 SCL=14)");
    let mut sensor = Shtc3::new(
        peripherals.i2c0,
        peripherals.pins.gpio13,
        peripherals.pins.gpio14,
    )?;

    // ---- 主循环 ----
    let boot = Instant::now();
    let mut n: u32 = 0;
    loop {
        n = n.saturating_add(1);
        match sensor.read() {
            Ok((t, rh)) => {
                state.temperature_c = Some(t);
                state.humidity_pct = Some(rh);
                log::info!("#{n} T={t:.2}°C RH={rh:.2}%");
            }
            Err(e) => {
                log::error!("SHTC3 read failed: {e}");
                state.temperature_c = None;
                state.humidity_pct = None;
            }
        }
        state.uptime_secs = boot.elapsed().as_secs();
        state.sample_count = n;

        let _ = ui::render(&mut display, &state);
        if let Err(e) = display.flush() {
            log::error!("Display flush failed: {e}");
        }

        sleep(Duration::from_secs(5));
    }
}

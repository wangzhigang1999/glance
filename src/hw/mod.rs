//! 外设抽象层:把具体传感器封进 struct,方便主循环干净调用。

use std::sync::{Arc, Mutex};

use esp_idf_svc::hal::i2c::I2cDriver;

pub mod battery;
pub mod button;
pub mod chip_temp;
pub mod es7210;
pub mod mic;
pub mod rtc;
pub mod sdcard;
pub mod shtc3;
pub mod storage;
pub mod system;

/// 板上所有 I2C 设备(SHTC3 / ES7210 / ES8311 / PCF85063)共用 SDA13/SCL14。
/// 各 driver 持 clone,每次访问 lock,长事务(如 SHTC3 的 wakeup→measure→read)
/// 在调用方持锁完成,避免被其他设备的 transaction 切断。
pub type I2cBus = Arc<Mutex<I2cDriver<'static>>>;

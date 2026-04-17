//! ESP32-S3 内置温度传感器
//!
//! 注:这是**芯片核温**,不是 CPU 利用率。随负载、周围环境温度变。
//! 典型 ESP32-S3 怠速下 40-60°C,满载能到 70-80°C。

use anyhow::{anyhow, Result};
use esp_idf_svc::sys::{
    temperature_sensor_config_t, temperature_sensor_config_t__bindgen_ty_1,
    temperature_sensor_enable, temperature_sensor_get_celsius, temperature_sensor_handle_t,
    temperature_sensor_install,
};

pub struct ChipTemp {
    handle: temperature_sensor_handle_t,
}

impl ChipTemp {
    pub fn new() -> Result<Self> {
        let config = temperature_sensor_config_t {
            range_min: -10,
            range_max: 80,
            clk_src: 0, // TEMPERATURE_SENSOR_CLK_SRC_DEFAULT
            flags: temperature_sensor_config_t__bindgen_ty_1::default(),
        };
        let mut handle: temperature_sensor_handle_t = core::ptr::null_mut();
        unsafe {
            let err = temperature_sensor_install(&config, &mut handle);
            if err != 0 {
                return Err(anyhow!("temperature_sensor_install: {}", err));
            }
            let err = temperature_sensor_enable(handle);
            if err != 0 {
                return Err(anyhow!("temperature_sensor_enable: {}", err));
            }
        }
        Ok(Self { handle })
    }

    pub fn read_celsius(&self) -> Option<f32> {
        let mut c: f32 = 0.0;
        unsafe {
            let err = temperature_sensor_get_celsius(self.handle, &mut c);
            if err == 0 {
                Some(c)
            } else {
                None
            }
        }
    }
}

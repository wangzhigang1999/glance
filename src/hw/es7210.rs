//! ES7210 4 通道音频 ADC(I2C 0x40,板上 A0=A1=GND)。
//!
//! 寄存器序列直译自 espressif/esp_codec_dev 的 `device/es7210/es7210.c`(Apache-2.0),
//! 只挑 slave + MIC1 + 16kHz + 16-bit I2S Philips 这一条路径所需的寄存器。
//!
//! 用法分两步,中间必须把 I2S 跑起来:
//! ```ignore
//! let mut codec = Es7210::new(bus);
//! codec.open_mic1()?;       // I2C 配寄存器,内部时钟仍关
//! mic.start()?;             // ESP32 I2S → MCLK/BCLK/LRCK 开始喂
//! codec.enable()?;          // 上电模拟通路 + 释放时钟,从此 SDOUT 出 PCM
//! ```

use anyhow::{Context, Result};

use super::I2cBus;

const ADDR: u8 = 0x40;

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const MCLK_MULTIPLE: u32 = 256;

// 寄存器编号对照 datasheet
const RESET: u8 = 0x00;
const CLOCK_OFF: u8 = 0x01;
const MAINCLK: u8 = 0x02;
const POWER_DOWN: u8 = 0x06;
const OSR: u8 = 0x07;
const MODE_CONFIG: u8 = 0x08;
const TIME_CTRL0: u8 = 0x09;
const TIME_CTRL1: u8 = 0x0A;
const SDP_IF1: u8 = 0x11;
const SDP_IF2: u8 = 0x12;
const HPF_REG20: u8 = 0x20;
const HPF_REG21: u8 = 0x21;
const HPF_REG22: u8 = 0x22;
const HPF_REG23: u8 = 0x23;
const ANALOG: u8 = 0x40;
const MIC12_BIAS: u8 = 0x41;
const MIC34_BIAS: u8 = 0x42;
const MIC1_GAIN: u8 = 0x43;
const MIC2_GAIN: u8 = 0x44;
const MIC3_GAIN: u8 = 0x45;
const MIC4_GAIN: u8 = 0x46;
const MIC1_POWER: u8 = 0x47;
const MIC2_POWER: u8 = 0x48;
const MIC3_POWER: u8 = 0x49;
const MIC4_POWER: u8 = 0x4A;
const MIC12_POWER: u8 = 0x4B;
const MIC34_POWER: u8 = 0x4C;

/// MIC1 PGA 档位:0=0dB / 10=30dB / 14=37.5dB(芯片硬件上限)。
/// 实测板内 mic 距离 ~1m,30dB 下峰值仅 4.5% 满量程,人声听不清楚。
/// 拉满到 14(+37.5dB,~2.37× 电压),配合 fixed-point 软件再放大,清晰度上来。
const MIC1_PGA: u8 = 14;

fn write(bus: &I2cBus, reg: u8, val: u8) -> Result<()> {
    let mut drv = bus.lock().expect("i2c bus poisoned");
    drv.write(ADDR, &[reg, val], 100)
        .with_context(|| format!("es7210 W 0x{reg:02X}=0x{val:02X}"))
}

fn read(bus: &I2cBus, reg: u8) -> Result<u8> {
    let mut drv = bus.lock().expect("i2c bus poisoned");
    let mut buf = [0u8];
    drv.write_read(ADDR, &[reg], &mut buf, 100)
        .with_context(|| format!("es7210 R 0x{reg:02X}"))?;
    Ok(buf[0])
}

fn update_bits(bus: &I2cBus, reg: u8, mask: u8, val: u8) -> Result<()> {
    let cur = read(bus, reg)?;
    let new = (cur & !mask) | (val & mask);
    write(bus, reg, new)
}

pub struct Es7210 {
    bus: I2cBus,
    /// open 阶段 mic_select 后读出的 reg 0x01,enable 时写回
    off_reg: u8,
}

impl Es7210 {
    pub fn new(bus: I2cBus) -> Self {
        Self { bus, off_reg: 0x3F }
    }

    /// 阶段 1:配寄存器,内部时钟仍关。可在 I2S 启动前调用。
    pub fn open_mic1(&mut self) -> Result<()> {
        let bus = &self.bus;

        // 软复位
        write(bus, RESET, 0xFF)?;
        write(bus, RESET, 0x41)?;
        // 全部时钟关
        write(bus, CLOCK_OFF, 0x3F)?;
        // 上电时序
        write(bus, TIME_CTRL0, 0x30)?;
        write(bus, TIME_CTRL1, 0x30)?;
        // ADC HPF 预设
        write(bus, HPF_REG23, 0x2A)?;
        write(bus, HPF_REG22, 0x0A)?;
        write(bus, HPF_REG20, 0x0A)?;
        write(bus, HPF_REG21, 0x2A)?;
        // slave 模式(reg 0x08 bit0 = 0)
        update_bits(bus, MODE_CONFIG, 0x01, 0x00)?;
        // 模拟初始 + MIC bias 2.87V
        write(bus, ANALOG, 0x43)?;
        write(bus, MIC12_BIAS, 0x70)?;
        write(bus, MIC34_BIAS, 0x70)?;
        // ADC OSR = 32
        write(bus, OSR, 0x20)?;
        // MAINCLK 分频清状态
        write(bus, MAINCLK, 0xC1)?;

        // mic_select(只开 MIC1):4 个 mic 的 gain bit4 先全清
        for r in [MIC1_GAIN, MIC2_GAIN, MIC3_GAIN, MIC4_GAIN] {
            update_bits(bus, r, 0x10, 0x00)?;
        }
        // MIC12 / MIC34 整组先关
        write(bus, MIC12_POWER, 0xFF)?;
        write(bus, MIC34_POWER, 0xFF)?;
        // 启 MIC1:reg 0x01 清 ADC1 路时钟掩码 0x0B
        update_bits(bus, CLOCK_OFF, 0x0B, 0x00)?;
        // MIC12 整组上电
        write(bus, MIC12_POWER, 0x00)?;
        // MIC1 gain enable + 拉满 PGA
        update_bits(bus, MIC1_GAIN, 0x10, 0x10)?;
        update_bits(bus, MIC1_GAIN, 0x0F, MIC1_PGA)?;
        // 单 mic 不走 TDM
        write(bus, SDP_IF2, 0x00)?;

        // 16-bit + I2S philips
        // SDP_IF1 bits[7:5]=011 → 16-bit;bits[1:0]=00 → I2S NORMAL
        write(bus, SDP_IF1, 0x60)?;

        // 记下 reg 0x01 的当前值,enable 时写回(此处一般是 0x34)
        self.off_reg = read(bus, CLOCK_OFF)?;
        log::info!("ES7210 open_mic1 OK (off_reg=0x{:02X})", self.off_reg);
        Ok(())
    }

    /// 阶段 2:I2S MCLK/BCLK/LRCK 已经在跑后调用。
    /// 上电模拟通路 + 释放时钟,SDOUT 开始出 PCM。
    pub fn enable(&mut self) -> Result<()> {
        let bus = &self.bus;
        // 释放时钟到 open 阶段保存的状态
        write(bus, CLOCK_OFF, self.off_reg)?;
        // 数字电源出 power-down
        write(bus, POWER_DOWN, 0x00)?;
        // 模拟再写一次
        write(bus, ANALOG, 0x43)?;
        // 4 路 MIC ADC 模拟电源各上一档(只用 MIC1 也照官方写满,无副作用)
        write(bus, MIC1_POWER, 0x08)?;
        write(bus, MIC2_POWER, 0x08)?;
        write(bus, MIC3_POWER, 0x08)?;
        write(bus, MIC4_POWER, 0x08)?;
        // 重新 assert MIC12 整组电源 / MIC34 关
        write(bus, MIC12_POWER, 0x00)?;
        write(bus, MIC34_POWER, 0xFF)?;
        write(bus, ANALOG, 0x43)?;
        // reset 寄存器一次脉冲(官方序列)
        write(bus, RESET, 0x71)?;
        write(bus, RESET, 0x41)?;
        log::info!(
            "ES7210 enable OK (MIC1 analog up, PGA reg=0x{:02X})",
            MIC1_PGA
        );
        Ok(())
    }
}

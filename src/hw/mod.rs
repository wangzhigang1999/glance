//! 外设抽象层:把具体传感器封进 struct,方便主循环干净调用。

pub mod battery;
pub mod chip_temp;
pub mod shtc3;
pub mod system;

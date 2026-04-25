//! 板内 SPIFFS 分区(12MB),挂在 `/storage` 下,std::fs 直接读写。
//!
//! 走 partitions.csv 里 0x410000 起的 `storage` 分区,**不需要任何 GPIO**。
//! 之前的 SD 卡方案因为板上 R7(CS 上拉)默认 NC 跑不通,这是临时方案,
//! 等补焊了 R7 把 sdcard.rs 找回来即可,上层 std::fs 调用一行不用动。
//!
//! **使用注意**:
//! - 总容量 12MB,16kHz × 单声道 × 16-bit PCM 算下来 ≈ **6 分钟**音频
//! - SPIFFS 是平面 KV,**不支持真正的目录**;可以用 `/storage/recordings/foo.wav`
//!   这种带斜杠的文件名,但 `mkdir` / `opendir` 跟你想的不一样
//! - 文件名最长 64 字符(sdkconfig 调过)
//! - 满了 `std::fs::write` 返回 `ENOSPC`,Phase E 的 prune 要负责清最老的

use std::ffi::CString;

use anyhow::{Context, Result};
use esp_idf_svc::sys::{
    esp, esp_spiffs_format, esp_spiffs_info, esp_vfs_spiffs_conf_t, esp_vfs_spiffs_register,
};

pub const MOUNT_PATH: &str = "/storage";
const PARTITION_LABEL: &str = "storage";

pub struct Storage {
    label: CString,
}

impl Storage {
    /// 注册 VFS + 挂载 SPIFFS。第一次跑会自动 format(分区里全 0xFF → 视为未格式化)。
    pub fn mount() -> Result<Self> {
        let base = CString::new(MOUNT_PATH).unwrap();
        let label = CString::new(PARTITION_LABEL).unwrap();
        let conf = esp_vfs_spiffs_conf_t {
            base_path: base.as_ptr(),
            partition_label: label.as_ptr(),
            max_files: 5,
            format_if_mount_failed: true,
        };
        esp!(unsafe { esp_vfs_spiffs_register(&conf) }).context("esp_vfs_spiffs_register")?;

        let (used, total) = read_info(&label);
        log::info!(
            "SPIFFS mounted at {MOUNT_PATH} ({}/{} KB used, {}% free)",
            used / 1024,
            total / 1024,
            if total > 0 {
                100 - (used * 100 / total)
            } else {
                0
            }
        );
        Ok(Self { label })
    }

    pub fn path() -> &'static str {
        MOUNT_PATH
    }

    /// (used_bytes, total_bytes)。Phase D/E 给 HTTP 看 / prune 用。
    pub fn stats(&self) -> (usize, usize) {
        read_info(&self.label)
    }
}

/// 格式化 SPIFFS 分区(瞬秒回,远比逐文件 remove 快)。
/// 需要分区已经 mount 好;esp_spiffs_format 内部会临时 unmount 再挂回。
pub fn format() -> Result<()> {
    let label = CString::new(PARTITION_LABEL).unwrap();
    esp!(unsafe { esp_spiffs_format(label.as_ptr()) }).context("esp_spiffs_format")?;
    log::info!("SPIFFS reformatted");
    Ok(())
}

fn read_info(label: &CString) -> (usize, usize) {
    let mut total = 0usize;
    let mut used = 0usize;
    unsafe {
        let _ = esp_spiffs_info(label.as_ptr(), &mut total, &mut used);
    }
    (used, total)
}

//! SD 卡(FATFS over SDMMC 1-bit)挂在 `/storage`,std::fs 直接读写。
//!
//! 板子(Waveshare ESP32-S3-RLCD-4.2)SD 槽走的是 **SDMMC 外设 1-bit**,不是 SPI:
//! - CLK = GPIO38 / CMD = GPIO21 / D0 = GPIO39
//! - 没有 CS,跟 R7 上拉无关(早期 storage.rs 注释里那句"R7 NC"是误判)
//!
//! mount 失败时(没插卡 / FAT 损坏 / 硬件问题)返回 Err,上层退回 SPIFFS。
//! 故意挂在跟 SPIFFS 同一个 base path(/storage),recorder/HTTP 路径全不用动。

use std::ffi::CString;

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::sys::{
    esp, esp_vfs_fat_info, esp_vfs_fat_mount_config_t, esp_vfs_fat_sdcard_unmount,
    esp_vfs_fat_sdmmc_mount, gpio_num_t_GPIO_NUM_21 as GPIO_NUM_21,
    gpio_num_t_GPIO_NUM_38 as GPIO_NUM_38, gpio_num_t_GPIO_NUM_39 as GPIO_NUM_39,
    gpio_num_t_GPIO_NUM_NC as GPIO_NUM_NC, sdmmc_card_t, sdmmc_host_check_buffer_alignment,
    sdmmc_host_deinit_slot, sdmmc_host_do_transaction, sdmmc_host_get_real_freq,
    sdmmc_host_get_slot_width, sdmmc_host_init, sdmmc_host_io_int_enable, sdmmc_host_io_int_wait,
    sdmmc_host_is_slot_set_to_uhs1, sdmmc_host_set_bus_ddr_mode, sdmmc_host_set_bus_width,
    sdmmc_host_set_card_clk, sdmmc_host_set_cclk_always_on, sdmmc_host_set_input_delay,
    sdmmc_host_t, sdmmc_slot_config_t,
};

pub const MOUNT_PATH: &str = "/storage";

/// SDMMC slot 1(ESP32-S3 只有这一个 slot;slot 0 是 ESP32 才有的 8-bit slot)。
const SLOT: i32 = 1;
/// 最高 20 MHz(SDMMC_FREQ_DEFAULT)。可以试 40 MHz 但稳定性看走线。
const FREQ_KHZ: i32 = 20_000;

// flags 位定义在 sd_protocol_types.h 是 #define,bindgen 不会发出符号,自己复读
const FLAG_1BIT: u32 = 1 << 0;
const FLAG_DEINIT_ARG: u32 = 1 << 5;

pub struct Sdcard {
    base_path: CString,
    card: *mut sdmmc_card_t,
}

// 内部 *mut 仅在 mount/unmount 用,IDF 驱动自己加锁
unsafe impl Send for Sdcard {}
unsafe impl Sync for Sdcard {}

impl Sdcard {
    /// 初始化 SDMMC 1-bit 总线 + 探卡 + 挂 FATFS 到 `/storage`。
    /// 没插卡 / 卡未格式化 / FATFS 损坏都会返回 Err。
    pub fn mount() -> Result<Self> {
        let base = CString::new(MOUNT_PATH).unwrap();

        // ---- host(SDMMC peripheral)----
        // 对照 IDF v5.5.3 的 SDMMC_HOST_DEFAULT() 宏一一填,
        // 只把 flags 砍到 1BIT(板上 D1/D2/D3 没接,4-bit/8-bit 探不出来反而拖慢握手)。
        let mut host: sdmmc_host_t = unsafe { core::mem::zeroed() };
        host.flags = FLAG_1BIT | FLAG_DEINIT_ARG;
        host.slot = SLOT;
        host.max_freq_khz = FREQ_KHZ;
        host.io_voltage = 3.3;
        // 枚举默认 0 已经分别是 STRENGTH_B / 200MA / DELAY_PHASE_0,zeroed 已对
        host.command_timeout_ms = 0;
        host.init = Some(sdmmc_host_init);
        host.set_bus_width = Some(sdmmc_host_set_bus_width);
        host.get_bus_width = Some(sdmmc_host_get_slot_width);
        host.set_bus_ddr_mode = Some(sdmmc_host_set_bus_ddr_mode);
        host.set_card_clk = Some(sdmmc_host_set_card_clk);
        host.set_cclk_always_on = Some(sdmmc_host_set_cclk_always_on);
        host.do_transaction = Some(sdmmc_host_do_transaction);
        // union { deinit, deinit_p }:FLAG_DEINIT_ARG 决定走 deinit_p(int slot)
        host.__bindgen_anon_1.deinit_p = Some(sdmmc_host_deinit_slot);
        host.io_int_enable = Some(sdmmc_host_io_int_enable);
        host.io_int_wait = Some(sdmmc_host_io_int_wait);
        host.get_real_freq = Some(sdmmc_host_get_real_freq);
        host.set_input_delay = Some(sdmmc_host_set_input_delay);
        host.check_buffer_alignment = Some(sdmmc_host_check_buffer_alignment);
        host.is_slot_set_to_uhs1 = Some(sdmmc_host_is_slot_set_to_uhs1);

        // ---- slot ----
        let mut slot: sdmmc_slot_config_t = unsafe { core::mem::zeroed() };
        slot.clk = GPIO_NUM_38;
        slot.cmd = GPIO_NUM_21;
        slot.d0 = GPIO_NUM_39;
        // 1-bit 模式不用,但 d1..d7 默认 0 = GPIO0(板上是 BOOT 按键!),必须显式 NC
        slot.d1 = GPIO_NUM_NC;
        slot.d2 = GPIO_NUM_NC;
        slot.d3 = GPIO_NUM_NC;
        slot.d4 = GPIO_NUM_NC;
        slot.d5 = GPIO_NUM_NC;
        slot.d6 = GPIO_NUM_NC;
        slot.d7 = GPIO_NUM_NC;
        slot.__bindgen_anon_1.cd = GPIO_NUM_NC;
        slot.__bindgen_anon_2.wp = GPIO_NUM_NC;
        slot.width = 1;
        slot.flags = 0;

        // ---- mount ----
        let mount_cfg = esp_vfs_fat_mount_config_t {
            // 用户授权:废卡,挂不上就直接格式化为 FAT32(首次插白板新卡也走这条)
            format_if_mount_failed: true,
            max_files: 5,
            allocation_unit_size: 16 * 1024, // 16KB cluster,WAV 大文件友好
            disk_status_check_enable: false,
            use_one_fat: false,
        };

        let mut card: *mut sdmmc_card_t = core::ptr::null_mut();
        let rc = unsafe {
            esp_vfs_fat_sdmmc_mount(
                base.as_ptr(),
                &host,
                &slot as *const _ as *const core::ffi::c_void,
                &mount_cfg,
                &mut card,
            )
        };
        if rc != esp_idf_svc::sys::ESP_OK {
            return Err(anyhow!(
                "esp_vfs_fat_sdmmc_mount failed: {} ({})",
                rc,
                esp_err_name(rc)
            ));
        }
        if card.is_null() {
            // 理论上 ESP_OK 时 card 一定非空,保险起见兜一下
            return Err(anyhow!("esp_vfs_fat_sdmmc_mount returned null card"));
        }

        // 卡基本信息 + 容量
        let (cap_mb, freq_khz, name) = unsafe {
            let c = &*card;
            let cap =
                (c.csd.capacity as u64).saturating_mul(c.csd.sector_size as u64) / 1024 / 1024;
            // sdmmc_card_t 里 cid / raw_cid 是匿名 union,bindgen 包了一层 __bindgen_anon_1
            let cid = &c.__bindgen_anon_1.cid;
            // sdmmc_cid_t::name 是 char[8],可能不带 \0 结尾,按 .position 取
            let n_bytes: &[u8] =
                core::slice::from_raw_parts(cid.name.as_ptr() as *const u8, cid.name.len());
            let end = n_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(n_bytes.len());
            let nm = core::str::from_utf8(&n_bytes[..end])
                .unwrap_or("?")
                .to_string();
            (cap, c.real_freq_khz, nm)
        };
        log::info!(
            "SD mounted at {MOUNT_PATH}: name={name} cap={cap_mb}MB clk={freq_khz}kHz width=1"
        );
        if let Ok((used, total)) = stats(&base) {
            log::info!(
                "SD FS info: {}/{} MB used",
                used / 1024 / 1024,
                total / 1024 / 1024,
            );
        }

        Ok(Self {
            base_path: base,
            card,
        })
    }

    #[allow(dead_code)] // 留给 HTTP /api/storage 之类后续读
    pub fn path() -> &'static str {
        MOUNT_PATH
    }

    /// (used_bytes, total_bytes)。HTTP 那边显示用。
    #[allow(dead_code)] // 同上
    pub fn stats(&self) -> (usize, usize) {
        stats(&self.base_path).unwrap_or((0, 0))
    }
}

impl Drop for Sdcard {
    fn drop(&mut self) {
        if !self.card.is_null() {
            unsafe {
                let _ = esp_vfs_fat_sdcard_unmount(self.base_path.as_ptr(), self.card);
            }
            self.card = core::ptr::null_mut();
        }
    }
}

fn stats(base: &CString) -> Result<(usize, usize)> {
    let mut total_bytes: u64 = 0;
    let mut free_bytes: u64 = 0;
    esp!(unsafe { esp_vfs_fat_info(base.as_ptr(), &mut total_bytes, &mut free_bytes) })
        .context("esp_vfs_fat_info")?;
    let used = total_bytes.saturating_sub(free_bytes) as usize;
    Ok((used, total_bytes as usize))
}

fn esp_err_name(rc: i32) -> &'static str {
    let p = unsafe { esp_idf_svc::sys::esp_err_to_name(rc) };
    if p.is_null() {
        "?"
    } else {
        unsafe { core::ffi::CStr::from_ptr(p) }
            .to_str()
            .unwrap_or("?")
    }
}

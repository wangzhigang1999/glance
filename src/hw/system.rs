//! 系统指标:堆 / PSRAM / 栈 / 复位原因 / MAC / 固件版本
//!
//! 全部客观数据,每次 UI 刷新读一次(开销 μs 级,不心疼)。

use esp_idf_svc::sys::{
    esp_efuse_mac_get_default, esp_ota_get_running_partition, esp_partition_read, esp_reset_reason,
    heap_caps_get_free_size, heap_caps_get_minimum_free_size, heap_caps_get_total_size,
    uxTaskGetStackHighWaterMark, MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM,
};

#[derive(Debug, Clone, Copy)]
pub struct SysStats {
    /// 内部 SRAM 空闲字节
    pub heap_free: usize,
    /// 内部 SRAM 总量(不变,启动时算一次也行,这里每次读保持一致)
    pub heap_total: usize,
    /// 启动以来内部 SRAM 最低值(暴露泄漏/突刺)
    pub heap_min_ever: usize,
    /// PSRAM 空闲
    pub psram_free: usize,
    pub psram_total: usize,
    /// 主任务栈剩余字节(High Water Mark)
    pub main_stack_hwm_bytes: u32,
    /// 最近一次复位原因
    pub reset_reason: &'static str,
}

/// Flash 存储布局(静态,启动时读一次即可)。
#[derive(Debug, Clone, Copy, Default)]
pub struct FlashStats {
    /// Flash 芯片总容量(字节)—— 硬编码 16MB(ESP32-S3-RLCD-4.2 板载)
    pub flash_total: u32,
    /// 当前运行 app 所在分区容量
    pub app_part_size: u32,
    /// 当前 app 镜像实际占用字节(解析 esp_image_header_t 得到)
    pub app_used: u32,
    /// 当前运行分区 flash 偏移(调试可见用)
    pub app_part_addr: u32,
}

/// 解析运行中 app 镜像头,累加各 segment 长度得到实际镜像字节数。
/// ESP-IDF 镜像格式:
///   [24B esp_image_header] [segment_header(8B) + data]... [pad16 + 1B checksum] [opt 32B SHA256]
unsafe fn compute_app_image_len(part: *const esp_idf_svc::sys::esp_partition_t) -> u32 {
    if part.is_null() {
        return 0;
    }
    let mut hdr = [0u8; 24];
    let rc = esp_partition_read(part, 0, hdr.as_mut_ptr() as *mut _, 24);
    if rc != 0 {
        return 0;
    }
    if hdr[0] != 0xE9 {
        return 0; // magic 不对,放弃
    }
    let segment_count = hdr[1] as usize;
    let hash_appended = hdr[23] != 0;

    let mut offset: u32 = 24; // 走过 image header
    for _ in 0..segment_count {
        let mut seg = [0u8; 8];
        let rc = esp_partition_read(part, offset as usize, seg.as_mut_ptr() as *mut _, 8);
        if rc != 0 {
            return 0;
        }
        let data_len = u32::from_le_bytes([seg[4], seg[5], seg[6], seg[7]]);
        offset = offset.saturating_add(8).saturating_add(data_len);
    }
    // 对齐到 16 字节,再 +1 byte checksum,(可选) +32 SHA256
    let pad = 16 - ((offset + 1) % 16);
    offset = offset.saturating_add(pad).saturating_add(1);
    if hash_appended {
        offset = offset.saturating_add(32);
    }
    offset
}

pub fn read_flash_stats() -> FlashStats {
    unsafe {
        let running = esp_ota_get_running_partition();
        let (app_part_size, app_part_addr) = if running.is_null() {
            (0, 0)
        } else {
            ((*running).size, (*running).address)
        };
        let app_used = compute_app_image_len(running);
        FlashStats {
            flash_total: 16 * 1024 * 1024,
            app_part_size,
            app_used,
            app_part_addr,
        }
    }
}

pub fn read_sys_stats() -> SysStats {
    unsafe {
        // 全部口径对齐到 MALLOC_CAP_INTERNAL(仅内部 SRAM),否则 free 会统计
        // PSRAM 这一大池导致 free > total 出现 "0/338 0%" 的错数
        let heap_free = heap_caps_get_free_size(MALLOC_CAP_INTERNAL);
        let heap_total = heap_caps_get_total_size(MALLOC_CAP_INTERNAL);
        let heap_min_ever = heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL);
        let psram_free = heap_caps_get_free_size(MALLOC_CAP_SPIRAM);
        let psram_total = heap_caps_get_total_size(MALLOC_CAP_SPIRAM);
        // null task handle = current task = main task(我们是在 app_main 里)
        let hwm_words = uxTaskGetStackHighWaterMark(core::ptr::null_mut());
        let main_stack_hwm_bytes = hwm_words * 4; // FreeRTOS 栈以 word(4B)计
        let reset_reason = reset_reason_name(esp_reset_reason());

        SysStats {
            heap_free,
            heap_total,
            heap_min_ever,
            psram_free,
            psram_total,
            main_stack_hwm_bytes,
            reset_reason,
        }
    }
}

fn reset_reason_name(r: esp_idf_svc::sys::esp_reset_reason_t) -> &'static str {
    use esp_idf_svc::sys::*;
    #[allow(non_upper_case_globals)]
    match r {
        esp_reset_reason_t_ESP_RST_POWERON => "PWR",
        esp_reset_reason_t_ESP_RST_SW => "SW",
        esp_reset_reason_t_ESP_RST_PANIC => "PANIC",
        esp_reset_reason_t_ESP_RST_INT_WDT => "IWDT",
        esp_reset_reason_t_ESP_RST_TASK_WDT => "TWDT",
        esp_reset_reason_t_ESP_RST_WDT => "WDT",
        esp_reset_reason_t_ESP_RST_DEEPSLEEP => "DSLP",
        esp_reset_reason_t_ESP_RST_BROWNOUT => "BRWN",
        esp_reset_reason_t_ESP_RST_SDIO => "SDIO",
        esp_reset_reason_t_ESP_RST_USB => "USB",
        esp_reset_reason_t_ESP_RST_JTAG => "JTAG",
        _ => "OTH",
    }
}

/// 读 Base MAC 的后 3 字节(字符串 "XX:XX:XX")。UI 角落显示机器身份用。
pub fn mac_suffix() -> heapless::String<8> {
    let mut mac = [0u8; 6];
    let mut out: heapless::String<8> = heapless::String::new();
    unsafe {
        if esp_efuse_mac_get_default(mac.as_mut_ptr()) == 0 {
            use core::fmt::Write;
            let _ = write!(out, "{:02X}{:02X}{:02X}", mac[3], mac[4], mac[5]);
        }
    }
    out
}

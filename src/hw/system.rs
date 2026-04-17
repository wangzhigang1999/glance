//! 系统指标:堆 / PSRAM / 栈 / 复位原因 / MAC / 固件版本
//!
//! 全部客观数据,每次 UI 刷新读一次(开销 μs 级,不心疼)。

use esp_idf_svc::sys::{
    esp_efuse_mac_get_default, esp_get_free_heap_size, esp_get_minimum_free_heap_size,
    esp_reset_reason, heap_caps_get_free_size, heap_caps_get_total_size,
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

pub fn read_sys_stats() -> SysStats {
    unsafe {
        let heap_free = esp_get_free_heap_size() as usize;
        let heap_total = heap_caps_get_total_size(MALLOC_CAP_INTERNAL);
        let heap_min_ever = esp_get_minimum_free_heap_size() as usize;
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

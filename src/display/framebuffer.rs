//! ST7305 本地 framebuffer(**横屏 400×300**)
//!
//! 面板物理是 300×400 双稳态 LCD;Waveshare 官方 landscape 公式直接映射到
//! 同样的 15KB GRAM 布局,软件侧不改 0x36 Memory Access Control 寄存器。
//!
//! landscape 公式(照搬 Waveshare `InitLandscapeLUT`):
//! ```text
//! inv_y  = 299 - y        // y = 0..299
//! byte_x = x / 2          // x = 0..399  →  0..199
//! blk_y  = inv_y / 4      //         →  0..74
//! index  = byte_x * 75 + blk_y
//! local_x = x & 1;  local_y = inv_y & 3
//! bit    = 7 - (local_y * 2 + local_x)
//! ```
//!
//! 颜色语义(embedded-graphics):
//! - `BinaryColor::On`  → 前景(黑),bit 置 0
//! - `BinaryColor::Off` → 背景(白),bit 置 1
//!
//! 内存放置:`heap_caps_malloc(MALLOC_CAP_DMA)`,确保 SPI master 直送 DMA,
//! 否则 IDF 会在内部 SRAM 临时拷一份(15KB)→ 高频出现 setup_dma_priv_buffer 失败。

use embedded_graphics::pixelcolor::BinaryColor;
use esp_idf_svc::sys::{heap_caps_free, heap_caps_malloc, MALLOC_CAP_DMA};

/// 逻辑横屏宽度(px)
pub const WIDTH: u16 = 400;
/// 逻辑横屏高度(px)
pub const HEIGHT: u16 = 300;
/// GRAM 总字节。注意:`H4 = HEIGHT/4 = 75`,`W_HALF = WIDTH/2 = 200`
pub const BUF_LEN: usize = (WIDTH as usize / 2) * (HEIGHT as usize / 4); // 15_000

const H4: usize = HEIGHT as usize / 4; // 75

pub struct FrameBuffer {
    ptr: *mut u8,
}

// 单线程独占使用,但显式声明便于上层结构跨线程移交所有权。
unsafe impl Send for FrameBuffer {}

impl FrameBuffer {
    pub fn new() -> Self {
        let ptr = unsafe { heap_caps_malloc(BUF_LEN, MALLOC_CAP_DMA) as *mut u8 };
        assert!(
            !ptr.is_null(),
            "framebuffer: heap_caps_malloc(DMA, 15000) failed"
        );
        unsafe { core::ptr::write_bytes(ptr, 0xFF, BUF_LEN) };
        Self { ptr }
    }

    pub fn raw(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr, BUF_LEN) }
    }

    fn raw_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr, BUF_LEN) }
    }

    pub fn fill(&mut self, color: BinaryColor) {
        let byte = if color.is_on() { 0x00 } else { 0xFF };
        unsafe { core::ptr::write_bytes(self.ptr, byte, BUF_LEN) };
    }

    #[inline]
    pub fn set_pixel(&mut self, x: u16, y: u16, color: BinaryColor) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let (index, mask) = pixel_index_mask(x, y);
        let byte = unsafe { self.raw_mut().get_unchecked_mut(index) };
        if color.is_on() {
            *byte &= !mask;
        } else {
            *byte |= mask;
        }
    }
}

impl Drop for FrameBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { heap_caps_free(self.ptr as *mut _) };
        }
    }
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn pixel_index_mask(x: u16, y: u16) -> (usize, u8) {
    let inv_y = (HEIGHT - 1 - y) as usize;
    let byte_x = (x >> 1) as usize; // x / 2
    let blk_y = inv_y >> 2; // inv_y / 4
    let index = byte_x * H4 + blk_y;
    let local_x = (x & 1) as u8;
    let local_y = (inv_y & 3) as u8;
    let bit = 7 - ((local_y << 1) | local_x);
    (index, 1u8 << bit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buf_len_correct() {
        assert_eq!(BUF_LEN, 15_000);
    }
    #[test]
    fn topleft() {
        // (0, 0) 在 landscape 下 inv_y=299,blk_y=74,local_y=3;byte_x=0,local_x=0
        // index = 0*75 + 74 = 74;bit = 7 - (3*2 + 0) = 1;mask = 0b0000_0010
        assert_eq!(pixel_index_mask(0, 0), (74, 0b0000_0010));
    }
    #[test]
    fn bottomright() {
        // (399, 299) inv_y=0,blk_y=0,local_y=0;byte_x=199,local_x=1
        // index = 199*75 + 0 = 14925;bit = 7 - (0 + 1) = 6;mask = 0b0100_0000
        assert_eq!(pixel_index_mask(399, 299), (14925, 0b0100_0000));
    }
}

//! ST7305 的 2×4 像素/字节布局：180°旋转等于字节顺序和每字节位序同时反转。
//! 原地处理，不额外占用 15KB DMA 内存。
pub fn rotate_180(frame: &mut [u8]) {
    frame.reverse();
    for byte in frame {
        *byte = byte.reverse_bits();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(frame: &[u8], x: usize, y: usize) -> bool {
        let inv_y = 299 - y;
        let index = (x / 2) * 75 + inv_y / 4;
        let bit = 7 - ((inv_y % 4) * 2 + x % 2);
        frame[index] & (1 << bit) != 0
    }

    #[test]
    fn rotation_matches_every_physical_pixel_and_is_reversible() {
        let original: Vec<u8> = (0..15_000usize)
            .map(|i| (i.wrapping_mul(73) ^ (i >> 3) ^ (i >> 8)) as u8)
            .collect();
        let mut rotated = original.clone();
        rotate_180(&mut rotated);
        for y in 0..300 {
            for x in 0..400 {
                assert_eq!(pixel(&rotated, x, y), pixel(&original, 399 - x, 299 - y));
            }
        }
        rotate_180(&mut rotated);
        assert_eq!(rotated, original);
    }
}

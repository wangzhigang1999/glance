//! 温湿度计 UI - 信息密集仪表盘
//!
//! 布局(400×300,四段 + 顶栏):
//!
//! ```text
//!   ┌──────────────────────────────────────────────┐
//!   │ SAT · 2026-04-18              [USB]  v0.1.0 │ 顶栏
//!   ├──────────────────────────────────────────────┤
//!   │                                              │
//!   │                  22:35                       │ 巨大时钟 logisoso58
//!   │                                              │
//!   ├──────────────────────────────────────────────┤
//!   │   30.7 °C          │          51.4 %         │ T/RH
//!   ├──────────────────────────────────────────────┤
//!   │  CHIP 46.3°C    HEAP 198/246K   STACK 28.4K │ 系统指标
//!   │  PSRAM 8012/8192K              UP 02:34:12  │
//!   │  IDF v5.5.3   RST PWR   MAC CA:7A:E0        │
//!   ├──────────────────────────────────────────────┤
//!   │    ▁▃▅ CU_2089 · 192.168.1.17 · -44 dBm    │ 底栏居中
//!   └──────────────────────────────────────────────┘
//! ```

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_6X10, FONT_9X18_BOLD},
        MonoTextStyle,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Baseline, Text},
};
use profont::{PROFONT_14_POINT, PROFONT_18_POINT, PROFONT_24_POINT};
use u8g2_fonts::{
    fonts::u8g2_font_logisoso58_tn,
    types::{FontColor, HorizontalAlignment, VerticalPosition},
    FontRenderer,
};

use crate::display::{Display, HEIGHT, WIDTH};

#[derive(Debug, Clone, Default)]
pub struct AppState {
    // 传感器
    pub temperature_c: Option<f32>,
    pub humidity_pct: Option<f32>,
    pub chip_temp_c: Option<f32>,
    // 运行
    pub uptime_secs: u64,
    pub sample_count: u32,
    pub heap_free: u32,
    pub heap_total: u32,
    pub heap_min_ever: u32,
    pub psram_free: u32,
    pub psram_total: u32,
    pub stack_hwm_bytes: u32,
    pub reset_reason: &'static str,
    pub mac_suffix: heapless::String<8>,
    pub fw_version: &'static str,
    pub idf_version: &'static str,
    // 网络
    pub wifi_connected: bool,
    pub wifi_ssid: heapless::String<32>,
    pub ip_octets: Option<[u8; 4]>,
    pub rssi: Option<i32>,
    // 时钟
    pub clock_hm: Option<heapless::String<8>>,
    pub clock_date: Option<heapless::String<16>>,
    // 电源
    pub battery: Option<(u32, u8)>,
    // 配网
    pub prov_mode: bool,
    pub prov_hint: heapless::String<32>,
}

// y 坐标分隔线
const Y_SEP_TOP: i32 = 30;
const Y_SEP_CLOCK: i32 = 108;
const Y_SEP_TH: i32 = 154;
const Y_SEP_STATS: i32 = 232;

pub fn render(target: &mut Display<'_>, state: &AppState) -> Result<(), core::convert::Infallible> {
    target.clear(BinaryColor::Off)?;

    let tiny = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::On);
    let micro = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let th_label = MonoTextStyle::new(&PROFONT_14_POINT, BinaryColor::On);
    let header = MonoTextStyle::new(&PROFONT_18_POINT, BinaryColor::On);
    let th_val = MonoTextStyle::new(&PROFONT_24_POINT, BinaryColor::On);

    // 外边框
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 2))
        .draw(target)?;

    if state.prov_mode {
        return render_prov(target, state, &tiny, &header, &th_val);
    }

    // 横向分隔线
    for y in [Y_SEP_TOP, Y_SEP_CLOCK, Y_SEP_TH, Y_SEP_STATS] {
        Line::new(Point::new(6, y), Point::new(WIDTH as i32 - 6, y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(target)?;
    }

    render_top_bar(target, state, &tiny)?;
    render_clock(target, state)?;
    render_th(target, state, &th_val, &th_label)?;
    render_stats(target, state, &tiny)?;
    render_bottom_bar(target, state, &micro)?;

    Ok(())
}

// ============================================================================
// 顶栏 y=0..30
// ============================================================================
fn render_top_bar(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    // 左:日期 + 星期
    let date: &str = state
        .clock_date
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("---- -- -- ---");
    Text::with_baseline(date, Point::new(10, 7), *tiny, Baseline::Top).draw(target)?;

    // 右:电池 / USB + 版本
    let mut right: heapless::String<24> = heapless::String::new();
    match state.battery {
        Some((mv, pct)) => {
            let _ = write!(right, "{}% {}.{:02}V v{}", pct, mv / 1000, (mv % 1000) / 10, state.fw_version);
        }
        None => {
            let _ = write!(right, "USB v{}", state.fw_version);
        }
    }
    let right_px = right.len() as i32 * 9;

    // 电池图标(如有)
    if let Some((_, pct)) = state.battery {
        let icon_x = WIDTH as i32 - 10 - right_px - 32;
        draw_battery_icon(target, Point::new(icon_x, 8), pct)?;
    }
    Text::with_baseline(
        &right,
        Point::new(WIDTH as i32 - 10 - right_px, 7),
        *tiny,
        Baseline::Top,
    )
    .draw(target)?;
    Ok(())
}

fn draw_battery_icon(
    target: &mut Display<'_>,
    origin: Point,
    pct: u8,
) -> Result<(), core::convert::Infallible> {
    Rectangle::new(origin, Size::new(26, 13))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    Rectangle::new(Point::new(origin.x + 26, origin.y + 3), Size::new(2, 6))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(target)?;
    let fill_w = ((pct as u32) * 22 / 100).min(22);
    if fill_w > 0 {
        Rectangle::new(
            Point::new(origin.x + 2, origin.y + 2),
            Size::new(fill_w, 9),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(target)?;
    }
    Ok(())
}

// ============================================================================
// 大时钟 y=30..108(使用 u8g2 logisoso58 数字字体,~58px 高)
// ============================================================================
fn render_clock(
    target: &mut Display<'_>,
    state: &AppState,
) -> Result<(), core::convert::Infallible> {
    let font = FontRenderer::new::<u8g2_font_logisoso58_tn>();
    let cx = WIDTH as i32 / 2;
    let y_base = Y_SEP_CLOCK - 8; // baseline

    let text: heapless::String<8> = match &state.clock_hm {
        Some(s) => s.clone(),
        None => {
            let mut t: heapless::String<8> = heapless::String::new();
            let _ = t.push_str("--:--");
            t
        }
    };
    let _ = font.render_aligned(
        text.as_str(),
        Point::new(cx, y_base),
        VerticalPosition::Baseline,
        HorizontalAlignment::Center,
        FontColor::Transparent(BinaryColor::On),
        target,
    );
    Ok(())
}

// ============================================================================
// 温湿度 y=108..154
// ============================================================================
fn render_th(
    target: &mut Display<'_>,
    state: &AppState,
    val: &MonoTextStyle<'_, BinaryColor>,
    label: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let center_l = WIDTH as i32 / 4;
    let center_r = WIDTH as i32 * 3 / 4;
    let y_val = Y_SEP_CLOCK + 18;
    let y_lbl = Y_SEP_TH - 8;
    let style = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    // T
    let mut t_txt: heapless::String<16> = heapless::String::new();
    match state.temperature_c {
        Some(t) => {
            let _ = write!(t_txt, "{:.1} deg C", t);
        }
        None => {
            let _ = t_txt.push_str("--.- deg C");
        }
    }
    Text::with_text_style(&t_txt, Point::new(center_l, y_val), *val, style).draw(target)?;
    Text::with_text_style("TEMPERATURE", Point::new(center_l, y_lbl), *label, style)
        .draw(target)?;

    // RH
    let mut h_txt: heapless::String<16> = heapless::String::new();
    match state.humidity_pct {
        Some(h) => {
            let _ = write!(h_txt, "{:.1} %", h);
        }
        None => {
            let _ = h_txt.push_str("--.- %");
        }
    }
    Text::with_text_style(&h_txt, Point::new(center_r, y_val), *val, style).draw(target)?;
    Text::with_text_style("HUMIDITY", Point::new(center_r, y_lbl), *label, style)
        .draw(target)?;

    // 中间竖分
    Line::new(
        Point::new(WIDTH as i32 / 2, Y_SEP_CLOCK + 4),
        Point::new(WIDTH as i32 / 2, Y_SEP_TH - 4),
    )
    .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
    .draw(target)?;
    Ok(())
}

// ============================================================================
// 系统指标 3 行 y=154..232(每行 ~24px,FONT_9X18_BOLD)
// ============================================================================
fn render_stats(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let left_x = 10;
    let line1_y = Y_SEP_TH + 6;
    let line2_y = line1_y + 24;
    let line3_y = line2_y + 24;

    // Line 1:CHIP 温  |  HEAP free/total
    let mut l1: heapless::String<48> = heapless::String::new();
    match state.chip_temp_c {
        Some(c) => {
            let _ = write!(l1, "CHIP {:.1}C", c);
        }
        None => {
            let _ = l1.push_str("CHIP --.-C");
        }
    }
    let _ = write!(
        l1,
        "  HEAP {}/{}K  STK {:.1}K",
        state.heap_free / 1024,
        state.heap_total / 1024,
        state.stack_hwm_bytes as f32 / 1024.0
    );
    Text::with_baseline(&l1, Point::new(left_x, line1_y), *tiny, Baseline::Top)
        .draw(target)?;

    // Line 2:PSRAM + UP
    let mut l2: heapless::String<48> = heapless::String::new();
    let _ = write!(
        l2,
        "PSRAM {}/{}K",
        state.psram_free / 1024,
        state.psram_total / 1024
    );
    let up_h = state.uptime_secs / 3600;
    let up_m = (state.uptime_secs / 60) % 60;
    let up_s = state.uptime_secs % 60;
    let _ = write!(l2, "  UP {:02}:{:02}:{:02}", up_h, up_m, up_s);
    Text::with_baseline(&l2, Point::new(left_x, line2_y), *tiny, Baseline::Top)
        .draw(target)?;

    // Line 3:IDF 版本 + 复位原因 + MAC + 最低堆
    let mut l3: heapless::String<48> = heapless::String::new();
    let _ = write!(
        l3,
        "IDF {}  RST {}  MAC {}  LO {}K",
        state.idf_version,
        state.reset_reason,
        state.mac_suffix,
        state.heap_min_ever / 1024
    );
    Text::with_baseline(&l3, Point::new(left_x, line3_y), *tiny, Baseline::Top)
        .draw(target)?;
    Ok(())
}

// ============================================================================
// 底栏 y=232..300(WiFi 信号条 + SSID + IP + RSSI,全部居中)
// ============================================================================
fn render_bottom_bar(
    target: &mut Display<'_>,
    state: &AppState,
    micro: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let y = Y_SEP_STATS + 18;

    if !state.wifi_connected {
        let msg = "WiFi disconnected";
        let w = msg.len() as i32 * 6;
        Text::with_baseline(
            msg,
            Point::new((WIDTH as i32 - w) / 2, y - 4),
            *micro,
            Baseline::Top,
        )
        .draw(target)?;
        return Ok(());
    }

    // 组装底栏文字
    let mut text: heapless::String<80> = heapless::String::new();
    if !state.wifi_ssid.is_empty() {
        let _ = text.push_str(&state.wifi_ssid);
    } else {
        let _ = text.push_str("WiFi");
    }
    if let Some([a, b, c, d]) = state.ip_octets {
        let _ = write!(text, " - {}.{}.{}.{}", a, b, c, d);
    }
    if let Some(r) = state.rssi {
        let _ = write!(text, " - {} dBm", r);
    }

    let bars_w = 20; // 信号条占位
    let spacer = 6;
    let text_px = text.len() as i32 * 6;
    let total = bars_w + spacer + text_px;
    let start_x = (WIDTH as i32 - total) / 2;

    // 信号条
    let bars = rssi_to_bars(state.rssi);
    draw_wifi_bars(target, Point::new(start_x, y - 10), bars)?;

    // 文字
    Text::with_baseline(
        &text,
        Point::new(start_x + bars_w + spacer, y - 2),
        *micro,
        Baseline::Top,
    )
    .draw(target)?;
    Ok(())
}

fn rssi_to_bars(rssi: Option<i32>) -> u8 {
    match rssi {
        Some(r) if r >= -50 => 3,
        Some(r) if r >= -65 => 2,
        Some(r) if r >= -80 => 1,
        _ => 0,
    }
}

fn draw_wifi_bars(
    target: &mut Display<'_>,
    origin: Point,
    active: u8,
) -> Result<(), core::convert::Infallible> {
    for i in 0..3u8 {
        let w = 4i32;
        let h = 4i32 + i as i32 * 4;
        let x = origin.x + i as i32 * 6;
        let y = origin.y + (12 - h);
        let r = Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32));
        if i < active {
            r.into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(target)?;
        } else {
            r.into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(target)?;
        }
    }
    Ok(())
}

// ============================================================================
// BLE 配网模式(独立于主仪表盘)
// ============================================================================
fn render_prov(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    header: &MonoTextStyle<'_, BinaryColor>,
    big: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let cx = WIDTH as i32 / 2;
    let style = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    Text::with_text_style("BLE SETUP", Point::new(cx, 50), *big, style).draw(target)?;
    Text::with_text_style(
        "Connect to BLE device:",
        Point::new(cx, 110),
        *header,
        style,
    )
    .draw(target)?;
    Text::with_text_style(&state.prov_hint, Point::new(cx, 148), *big, style).draw(target)?;
    Text::with_text_style(
        "Open nRF Connect, scan, connect,",
        Point::new(cx, 210),
        *tiny,
        style,
    )
    .draw(target)?;
    Text::with_text_style(
        "write SSID, PASSWORD, COMMIT=01.",
        Point::new(cx, 232),
        *tiny,
        style,
    )
    .draw(target)?;
    Text::with_text_style(
        "Service 524c4344-c001-4c7c-9b4f-00..",
        Point::new(cx, 268),
        *tiny,
        style,
    )
    .draw(target)?;
    Ok(())
}

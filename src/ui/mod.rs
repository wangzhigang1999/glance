//! 温湿度计 UI(横版 400×300,单色 1-bit)
//!
//! 仪表盘式四段布局:
//!
//! ```text
//!   ┌──────────────────────────────────────────────┐
//!   │ SAT 2026-04-18                [▓▓▓▓░]85% 4.02V│  顶栏:日期 + 电池
//!   ├──────────────────────────────────────────────┤
//!   │                                              │
//!   │                22:35:41                      │  中央:大时钟
//!   │                                              │
//!   ├──────────────────────────────────────────────┤
//!   │    25.3 °C              54.3 %               │  中下:T / RH
//!   │  TEMPERATURE           HUMIDITY              │
//!   ├──────────────────────────────────────────────┤
//!   │ ▁▃▅ CU_2089  192.168.1.17         -45 dBm   │  底栏:网络
//!   └──────────────────────────────────────────────┘
//! ```

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{
        ascii::{FONT_9X18_BOLD, FONT_6X10},
        MonoTextStyle,
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Baseline, Text},
};
use profont::{PROFONT_14_POINT, PROFONT_18_POINT, PROFONT_24_POINT};

use crate::display::{Display, HEIGHT, WIDTH};

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub temperature_c: Option<f32>,
    pub humidity_pct: Option<f32>,
    pub uptime_secs: u64,
    pub sample_count: u32,
    pub wifi_connected: bool,
    pub wifi_ssid: heapless::String<32>,
    pub ip_octets: Option<[u8; 4]>,
    /// RSSI dBm(负数,越接近 0 越强),None = 未连接
    pub rssi: Option<i32>,
    /// SNTP 同步后的本地时钟 HH:MM:SS;None = 未同步
    pub clock_hms: Option<heapless::String<8>>,
    /// "2026-04-18 SAT";None = 未同步
    pub clock_date: Option<heapless::String<16>>,
    /// 电池:None = USB 供电;Some((mv, pct))
    pub battery: Option<(u32, u8)>,
    /// true = BLE 配网模式,UI 替换成配网提示
    pub prov_mode: bool,
    pub prov_hint: heapless::String<32>,
}

// 四段式 y 坐标
const Y_TOP_SEP: i32 = 32;
const Y_MID_SEP: i32 = 160;
const Y_LOW_SEP: i32 = 248;

pub fn render(target: &mut Display<'_>, state: &AppState) -> Result<(), core::convert::Infallible> {
    target.clear(BinaryColor::Off)?;

    let tiny = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::On);
    let micro = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let label = MonoTextStyle::new(&PROFONT_14_POINT, BinaryColor::On);
    let header = MonoTextStyle::new(&PROFONT_18_POINT, BinaryColor::On);
    let big = MonoTextStyle::new(&PROFONT_24_POINT, BinaryColor::On);

    // 外边框:双线感
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 2))
        .draw(target)?;
    Rectangle::new(Point::new(3, 3), Size::new(WIDTH as u32 - 6, HEIGHT as u32 - 6))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    if state.prov_mode {
        return render_prov(target, state, &tiny, &header, &big);
    }

    // 三条横向分隔线
    for y in [Y_TOP_SEP, Y_MID_SEP, Y_LOW_SEP] {
        Line::new(Point::new(8, y), Point::new(WIDTH as i32 - 8, y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(target)?;
    }

    render_top_bar(target, state, &tiny)?;
    render_clock(target, state, &big, &tiny)?;
    render_th(target, state, &header, &label)?;
    render_bottom_bar(target, state, &micro)?;

    Ok(())
}

// ============================================================================
// 顶栏:日期 + 星期(左) | 电池图标 + % + 电压(右)
// ============================================================================
fn render_top_bar(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    // 左:"SAT 2026-04-18"
    let date_text = match &state.clock_date {
        Some(s) => s.as_str(),
        None => "---- -- -- ---",
    };
    Text::with_baseline(date_text, Point::new(10, 8), *tiny, Baseline::Top).draw(target)?;

    // 右:电池 icon + % + mV
    render_battery(target, state, tiny)?;
    Ok(())
}

fn render_battery(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let right_edge = WIDTH as i32 - 10;
    match state.battery {
        None => {
            // USB 供电,画一个 "USB" 文字
            Text::with_baseline("USB", Point::new(right_edge - 27, 8), *tiny, Baseline::Top)
                .draw(target)?;
        }
        Some((mv, pct)) => {
            // 右起:"4.02V"(5ch=45px) " " "85%"(3ch=27px) " " [battery 28x14]
            let mut txt: heapless::String<16> = heapless::String::new();
            let _ = write!(txt, "{}% {}.{:02}V", pct, mv / 1000, (mv % 1000) / 10);
            let txt_px = txt.len() as i32 * 9;
            Text::with_baseline(
                &txt,
                Point::new(right_edge - txt_px, 8),
                *tiny,
                Baseline::Top,
            )
            .draw(target)?;

            // 电池图标:放在 txt 左侧,28x14 外框 + 2x6 小嘴 + 内部填充
            let icon_x = right_edge - txt_px - 4 - 28;
            let icon_y = 9;
            draw_battery_icon(target, Point::new(icon_x, icon_y), pct)?;
        }
    }
    Ok(())
}

fn draw_battery_icon(
    target: &mut Display<'_>,
    origin: Point,
    pct: u8,
) -> Result<(), core::convert::Infallible> {
    // 外框 28×14
    Rectangle::new(origin, Size::new(28, 14))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    // 正极小嘴 2×6
    Rectangle::new(Point::new(origin.x + 28, origin.y + 4), Size::new(2, 6))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(target)?;
    // 内填:inset 2px,宽 24px,按 pct 缩放
    let fill_w = ((pct as u32) * 24 / 100).min(24);
    if fill_w > 0 {
        Rectangle::new(
            Point::new(origin.x + 2, origin.y + 2),
            Size::new(fill_w, 10),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(target)?;
    }
    Ok(())
}

// ============================================================================
// 中央:大时钟 HH:MM:SS
// ============================================================================
fn render_clock(
    target: &mut Display<'_>,
    state: &AppState,
    big: &MonoTextStyle<'_, BinaryColor>,
    tiny: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let cx = WIDTH as i32 / 2;
    let style = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    let y_center = (Y_TOP_SEP + Y_MID_SEP) / 2; // ≈ 96

    match &state.clock_hms {
        Some(hms) => {
            Text::with_text_style(hms, Point::new(cx, y_center), *big, style).draw(target)?;
        }
        None => {
            // 未同步:显示 uptime,中字号
            let up_h = state.uptime_secs / 3600;
            let up_m = (state.uptime_secs / 60) % 60;
            let up_s = state.uptime_secs % 60;
            let mut txt: heapless::String<16> = heapless::String::new();
            let _ = write!(txt, "up {:02}:{:02}:{:02}", up_h, up_m, up_s);
            Text::with_text_style(&txt, Point::new(cx, y_center), *big, style).draw(target)?;

            Text::with_text_style(
                "syncing time...",
                Point::new(cx, y_center + 28),
                *tiny,
                style,
            )
            .draw(target)?;
        }
    }
    Ok(())
}

// ============================================================================
// 中下:温度 | 湿度,各占一半,行内字号 18pt,下方 label 14pt
// ============================================================================
fn render_th(
    target: &mut Display<'_>,
    state: &AppState,
    header: &MonoTextStyle<'_, BinaryColor>,
    label: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let center_x_l = WIDTH as i32 / 4; // 100
    let center_x_r = WIDTH as i32 * 3 / 4; // 300
    let y_val = Y_MID_SEP + 32;
    let y_lbl = Y_LOW_SEP - 12;
    let style = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    // 温度
    let mut t_txt: heapless::String<16> = heapless::String::new();
    match state.temperature_c {
        Some(t) => {
            let _ = write!(t_txt, "{:.1} deg C", t);
        }
        None => {
            let _ = t_txt.push_str("--.- deg C");
        }
    }
    Text::with_text_style(&t_txt, Point::new(center_x_l, y_val), *header, style)
        .draw(target)?;
    Text::with_text_style(
        "TEMPERATURE",
        Point::new(center_x_l, y_lbl),
        *label,
        style,
    )
    .draw(target)?;

    // 湿度
    let mut h_txt: heapless::String<16> = heapless::String::new();
    match state.humidity_pct {
        Some(h) => {
            let _ = write!(h_txt, "{:.1} %", h);
        }
        None => {
            let _ = h_txt.push_str("--.- %");
        }
    }
    Text::with_text_style(&h_txt, Point::new(center_x_r, y_val), *header, style)
        .draw(target)?;
    Text::with_text_style("HUMIDITY", Point::new(center_x_r, y_lbl), *label, style)
        .draw(target)?;

    // 中间竖分隔
    Line::new(
        Point::new(WIDTH as i32 / 2, Y_MID_SEP + 6),
        Point::new(WIDTH as i32 / 2, Y_LOW_SEP - 6),
    )
    .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
    .draw(target)?;

    Ok(())
}

// ============================================================================
// 底栏:WiFi 信号条 + SSID + IP + RSSI(用最小的 6x10 字保证塞下)
// ============================================================================
fn render_bottom_bar(
    target: &mut Display<'_>,
    state: &AppState,
    micro: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let y_text = Y_LOW_SEP + 14;

    if !state.wifi_connected {
        Text::with_baseline(
            "WiFi disconnected",
            Point::new(10, y_text - 4),
            *micro,
            Baseline::Top,
        )
        .draw(target)?;
        return Ok(());
    }

    // 信号条:3 格阶梯,按 RSSI 分档
    let bars = rssi_to_bars(state.rssi);
    draw_wifi_bars(target, Point::new(10, y_text - 12), bars)?;

    // 左:SSID
    let mut left: heapless::String<48> = heapless::String::new();
    if !state.wifi_ssid.is_empty() {
        let _ = left.push_str(&state.wifi_ssid);
    } else {
        let _ = left.push_str("WiFi");
    }
    if let Some([a, b, c, d]) = state.ip_octets {
        let _ = write!(left, "  {}.{}.{}.{}", a, b, c, d);
    }
    Text::with_baseline(
        &left,
        Point::new(10 + 24, y_text - 2),
        *micro,
        Baseline::Top,
    )
    .draw(target)?;

    // 右:RSSI
    if let Some(r) = state.rssi {
        let mut right: heapless::String<16> = heapless::String::new();
        let _ = write!(right, "{} dBm", r);
        let right_px = right.len() as i32 * 6;
        Text::with_baseline(
            &right,
            Point::new(WIDTH as i32 - 10 - right_px, y_text - 2),
            *micro,
            Baseline::Top,
        )
        .draw(target)?;
    }

    Ok(())
}

/// 3 格 WiFi 信号条,按 RSSI 分档:
/// >= -50 → 3 / >= -65 → 2 / >= -80 → 1 / < -80 → 0
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
    // 三条递增矩形,宽 4,高 4/8/12,间距 2
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
// BLE 配网模式
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

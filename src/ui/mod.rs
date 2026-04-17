//! 温湿度计 UI(横版 400×300,用 profont 大字)
//!
//! ```text
//!   ┌──────────────────────────────────────────────┐
//!   │ ┌── TEMPERATURE ──┐│┌──── HUMIDITY ────┐     │
//!   │ │                  ││                    │    │
//!   │ │      26.5 °       ││       54.3 %      │    │
//!   │ │          C        ││                    │    │
//!   │ └──────────────────┘│└────────────────────┘  │
//!   │ up 00:01:23 n=42              RLCD-4.2/Rust │
//!   └──────────────────────────────────────────────┘
//! ```

use core::fmt::Write;

use embedded_graphics::{
    mono_font::{ascii::FONT_9X18_BOLD, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Alignment, Baseline, Text},
};
use profont::{PROFONT_18_POINT, PROFONT_24_POINT};

use crate::display::{Display, HEIGHT, WIDTH};

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub temperature_c: Option<f32>,
    pub humidity_pct: Option<f32>,
    pub uptime_secs: u64,
    pub sample_count: u32,
    pub wifi_connected: bool,
    pub ip_octets: Option<[u8; 4]>,
    /// SNTP 同步后的本地时钟 HH:MM:SS;None = 未同步
    pub clock_hms: Option<heapless::String<8>>,
    /// true = BLE 配网模式,UI 替换成配网提示
    pub prov_mode: bool,
    /// 配网模式下的一行提示(设备名 / "connecting xxx" / ...)
    pub prov_hint: heapless::String<32>,
}

pub fn render(target: &mut Display<'_>, state: &AppState) -> Result<(), core::convert::Infallible> {
    target.clear(BinaryColor::Off)?;

    let tiny = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::On);
    let header = MonoTextStyle::new(&PROFONT_18_POINT, BinaryColor::On);
    let big = MonoTextStyle::new(&PROFONT_24_POINT, BinaryColor::On);

    // 外边框:双线感,实际两道 1 像素 rect 嵌套
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 2))
        .draw(target)?;
    Rectangle::new(Point::new(3, 3), Size::new(WIDTH as u32 - 6, HEIGHT as u32 - 6))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // BLE 配网模式:整张屏显示配网提示,不画温湿度卡片
    if state.prov_mode {
        return render_prov(target, state, &tiny, &header, &big);
    }

    // 左右两个 card:各占一半,预留 6 像素外边距
    let card_y = 20;
    let card_h = HEIGHT as i32 - 60;
    let card_w = WIDTH as i32 / 2 - 16;

    // 左卡片:温度
    draw_card(
        target,
        Point::new(10, card_y),
        Size::new(card_w as u32, card_h as u32),
        "TEMPERATURE",
        &state
            .temperature_c
            .map(format_num)
            .unwrap_or_else(|| make16("--.-")),
        &make16("deg C"), // profont 是 ASCII 字体,用 "deg C" 代替 ℃
        &header,
        &big,
    )?;

    // 右卡片:湿度
    draw_card(
        target,
        Point::new(WIDTH as i32 / 2 + 6, card_y),
        Size::new(card_w as u32, card_h as u32),
        "HUMIDITY",
        &state
            .humidity_pct
            .map(format_num)
            .unwrap_or_else(|| make16("--.-")),
        &make16("%"),
        &header,
        &big,
    )?;

    // 页脚分割线
    Line::new(
        Point::new(10, HEIGHT as i32 - 36),
        Point::new(WIDTH as i32 - 10, HEIGHT as i32 - 36),
    )
    .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
    .draw(target)?;

    // 页脚左:时钟(同步后) + uptime + 采样计数
    let mut footer_l: heapless::String<48> = heapless::String::new();
    let up_m = (state.uptime_secs / 60) % 60;
    let up_h = state.uptime_secs / 3600;
    match &state.clock_hms {
        Some(hms) => {
            let _ = write!(footer_l, "{hms}  up {:02}:{:02}  n={}", up_h, up_m, state.sample_count);
        }
        None => {
            let up_s = state.uptime_secs % 60;
            let _ = write!(
                footer_l,
                "--:--:--  up {:02}:{:02}:{:02}  n={}",
                up_h, up_m, up_s, state.sample_count
            );
        }
    }
    Text::with_baseline(
        &footer_l,
        Point::new(10, HEIGHT as i32 - 28),
        tiny,
        Baseline::Top,
    )
    .draw(target)?;

    // 页脚右:WiFi 状态
    let mut footer_r: heapless::String<32> = heapless::String::new();
    match (state.wifi_connected, &state.ip_octets) {
        (true, Some([a, b, c, d])) => {
            let _ = write!(footer_r, "WiFi {}.{}.{}.{}", a, b, c, d);
        }
        (true, None) => {
            let _ = footer_r.push_str("WiFi up");
        }
        (false, _) => {
            let _ = footer_r.push_str("WiFi ...");
        }
    }
    // 右对齐:按字符宽 9px 估宽,tiny 是 FONT_9X18_BOLD
    let right_px = footer_r.len() as i32 * 9;
    Text::with_baseline(
        &footer_r,
        Point::new(WIDTH as i32 - 10 - right_px, HEIGHT as i32 - 28),
        tiny,
        Baseline::Top,
    )
    .draw(target)?;

    Ok(())
}

/// BLE 配网模式下的全屏提示,引导用户用手机连接 BLE 写凭据。
fn render_prov(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    header: &MonoTextStyle<'_, BinaryColor>,
    big: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let cx = WIDTH as i32 / 2;
    let center_style = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    // 顶部:大标题
    Text::with_text_style("BLE SETUP", Point::new(cx, 50), *big, center_style)
        .draw(target)?;

    // 中部:设备名
    Text::with_text_style(
        "Connect to BLE device:",
        Point::new(cx, 110),
        *header,
        center_style,
    )
    .draw(target)?;
    Text::with_text_style(&state.prov_hint, Point::new(cx, 148), *big, center_style)
        .draw(target)?;

    // 底部:使用说明
    Text::with_text_style(
        "Open nRF Connect, scan, connect,",
        Point::new(cx, 210),
        *tiny,
        center_style,
    )
    .draw(target)?;
    Text::with_text_style(
        "write SSID, PASSWORD, COMMIT=01.",
        Point::new(cx, 232),
        *tiny,
        center_style,
    )
    .draw(target)?;
    Text::with_text_style(
        "Service 524c4344-c001-4c7c-9b4f-00..",
        Point::new(cx, 268),
        *tiny,
        center_style,
    )
    .draw(target)?;

    Ok(())
}

fn draw_card<D>(
    target: &mut D,
    origin: Point,
    size: Size,
    title: &str,
    value: &heapless::String<16>,
    unit: &heapless::String<16>,
    header_style: &MonoTextStyle<'_, BinaryColor>,
    big_style: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    // 卡片框
    Rectangle::new(origin, size)
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // 标题横带背景(反色)
    let title_h: u32 = 24;
    Rectangle::new(origin, Size::new(size.width, title_h))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(target)?;

    // 标题文字(白字在黑底上)
    let title_style = MonoTextStyle::new(&profont::PROFONT_14_POINT, BinaryColor::Off);
    Text::with_text_style(
        title,
        Point::new(origin.x + size.width as i32 / 2, origin.y + title_h as i32 / 2 + 2),
        title_style,
        embedded_graphics::text::TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Middle)
            .build(),
    )
    .draw(target)?;

    // 大数值,水平居中
    Text::with_text_style(
        value,
        Point::new(origin.x + size.width as i32 / 2, origin.y + 70),
        *big_style,
        embedded_graphics::text::TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Middle)
            .build(),
    )
    .draw(target)?;

    // 单位(小一号,居中到 value 下方)
    Text::with_text_style(
        unit,
        Point::new(origin.x + size.width as i32 / 2, origin.y + 120),
        *header_style,
        embedded_graphics::text::TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Middle)
            .build(),
    )
    .draw(target)?;

    Ok(())
}

fn format_num(v: f32) -> heapless::String<16> {
    let mut s: heapless::String<16> = heapless::String::new();
    let _ = write!(s, "{:.1}", v);
    s
}

fn make16(s: &str) -> heapless::String<16> {
    let mut out: heapless::String<16> = heapless::String::new();
    let _ = out.push_str(s);
    out
}

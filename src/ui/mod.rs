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

#[derive(Debug, Clone)]
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

    // 传感器历史(5s/样,120样=10min)
    pub temp_hist: heapless::HistoryBuffer<f32, 120>,
    pub rh_hist: heapless::HistoryBuffer<f32, 120>,

    // GitHub 贡献活动(53 周 × 7 天 = 371 格),`contrib_valid=false` 表示未加载
    pub contrib: [u8; 371],         // level 0..=4
    pub contrib_counts: [u16; 371], // 当天 commit 数(per day)
    pub contrib_weeks: u16,
    pub contrib_valid: bool,
    pub contrib_total_year: u32,
    pub contrib_error: heapless::String<80>, // 拉取失败最近一次错误,空=没错误

    // GitHub Notifications(未读)
    pub notif_count: u32,
    pub notif_top_title: heapless::String<96>, // 最新一条 title
    pub notif_top_repo: heapless::String<48>,  // 最新一条仓库 full_name
    pub notif_valid: bool,

    // GitHub 活动:最近一条 event 摘要 + 详情 + open PR 数
    pub last_event_line: heapless::String<80>,
    pub last_event_detail: heapless::String<96>,
    pub open_prs: u32,
    pub activity_valid: bool,
    pub activity_error: heapless::String<80>,

    // GitHub 用户名(运行时可改,主循环从 SharedConfig 拷过来)
    pub gh_user: heapless::String<40>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            temperature_c: None,
            humidity_pct: None,
            chip_temp_c: None,
            uptime_secs: 0,
            sample_count: 0,
            heap_free: 0,
            heap_total: 0,
            heap_min_ever: 0,
            psram_free: 0,
            psram_total: 0,
            stack_hwm_bytes: 0,
            reset_reason: "",
            mac_suffix: heapless::String::new(),
            fw_version: "",
            idf_version: "",
            wifi_connected: false,
            wifi_ssid: heapless::String::new(),
            ip_octets: None,
            rssi: None,
            clock_hm: None,
            clock_date: None,
            battery: None,
            prov_mode: false,
            prov_hint: heapless::String::new(),
            temp_hist: heapless::HistoryBuffer::new(),
            rh_hist: heapless::HistoryBuffer::new(),
            contrib: [0u8; 371],
            contrib_counts: [0u16; 371],
            contrib_weeks: 0,
            contrib_valid: false,
            contrib_total_year: 0,
            contrib_error: heapless::String::new(),
            notif_count: 0,
            notif_top_title: heapless::String::new(),
            notif_top_repo: heapless::String::new(),
            notif_valid: false,
            last_event_line: heapless::String::new(),
            last_event_detail: heapless::String::new(),
            open_prs: 0,
            activity_valid: false,
            activity_error: heapless::String::new(),
            gh_user: heapless::String::new(),
        }
    }
}

// y 坐标分隔线
const Y_SEP_TOP: i32 = 30;
const Y_SEP_CLOCK: i32 = 108;
const Y_SEP_TH: i32 = 154;
const Y_SEP_STATS: i32 = 232;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    History,
    Github,
}

impl Page {
    pub fn next(self) -> Self {
        match self {
            Self::Dashboard => Self::History,
            Self::History => Self::Github,
            Self::Github => Self::Dashboard,
        }
    }

    pub const ALL: &'static [Page] = &[Page::Dashboard, Page::History, Page::Github];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

impl Default for Page {
    fn default() -> Self {
        Page::Dashboard
    }
}

pub fn render(
    target: &mut Display<'_>,
    state: &AppState,
    page: Page,
) -> Result<(), core::convert::Infallible> {
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

    match page {
        Page::Dashboard => render_dashboard(target, state, &tiny, &micro, &th_val, &th_label)?,
        Page::History => render_history(target, state, &tiny, &micro, &header)?,
        Page::Github => render_github(target, state, &tiny, &micro, &header, &th_val)?,
    }

    // 页面指示点(右下角,不覆盖主内容)
    draw_page_dots(target, page)?;

    Ok(())
}

fn render_dashboard(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    micro: &MonoTextStyle<'_, BinaryColor>,
    th_val: &MonoTextStyle<'_, BinaryColor>,
    th_label: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    for y in [Y_SEP_TOP, Y_SEP_CLOCK, Y_SEP_TH, Y_SEP_STATS] {
        Line::new(Point::new(6, y), Point::new(WIDTH as i32 - 6, y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(target)?;
    }
    render_top_bar(target, state, tiny)?;
    render_clock(target, state)?;
    render_th(target, state, th_val, th_label)?;
    render_stats(target, state, tiny)?;
    render_bottom_bar(target, state, micro)?;
    Ok(())
}

fn draw_page_dots(
    target: &mut Display<'_>,
    current: Page,
) -> Result<(), core::convert::Infallible> {
    // 右下角一排小圆点,落在"底栏正中线"上:● 当前 / ○ 其他
    let right = WIDTH as i32 - 10;
    let bar_mid_y = (Y_SEP_STATS + HEIGHT as i32) / 2;
    let spacing = 10i32;
    let r = 3u32;
    let cur_idx = current.index();
    let n = Page::ALL.len() as i32;
    for (i, _) in Page::ALL.iter().enumerate() {
        let cx = right - (n - 1 - i as i32) * spacing;
        let cy = bar_mid_y;
        let top_left = Point::new(cx - r as i32, cy - r as i32);
        let rect = Rectangle::new(top_left, Size::new(r * 2, r * 2));
        if i == cur_idx {
            rect.into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(target)?;
        } else {
            rect.into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(target)?;
        }
    }
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
    // bar 范围 [Y_SEP_STATS, HEIGHT),竖向正中 = (Y_SEP_STATS + HEIGHT) / 2
    let bar_mid_y = (Y_SEP_STATS + HEIGHT as i32) / 2;
    // FONT_6X10 baseline 在 Top:文字高度 10,想让中线落在 bar_mid_y 上,top = mid - 5
    let text_top_y = bar_mid_y - 5;

    if !state.wifi_connected {
        let msg = "WiFi disconnected";
        let w = msg.len() as i32 * 6;
        Text::with_baseline(
            msg,
            Point::new((WIDTH as i32 - w) / 2, text_top_y),
            *micro,
            Baseline::Top,
        )
        .draw(target)?;
        return Ok(());
    }

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

    let bars_w = 20;
    let spacer = 6;
    let text_px = text.len() as i32 * 6;
    let total = bars_w + spacer + text_px;
    let start_x = (WIDTH as i32 - total) / 2;

    // 信号条竖直居中:条高 12,top = bar_mid_y - 6
    let bars_top_y = bar_mid_y - 6;
    let bars = rssi_to_bars(state.rssi);
    draw_wifi_bars(target, Point::new(start_x, bars_top_y), bars)?;

    Text::with_baseline(
        &text,
        Point::new(start_x + bars_w + spacer, text_top_y),
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
// History 页 - 温湿度 10 min sparkline
// ============================================================================

fn render_history(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    micro: &MonoTextStyle<'_, BinaryColor>,
    header: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    // 页头
    Text::with_baseline(
        "ENVIRONMENT HISTORY",
        Point::new(14, 6),
        *tiny,
        Baseline::Top,
    )
    .draw(target)?;
    let hint = "10 min @ 5s";
    let hint_px = hint.len() as i32 * 6;
    Text::with_baseline(
        hint,
        Point::new(WIDTH as i32 - 14 - hint_px, 10),
        *micro,
        Baseline::Top,
    )
    .draw(target)?;
    Line::new(Point::new(6, 28), Point::new(WIDTH as i32 - 6, 28))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // 温度图
    render_sparkline_block(
        target,
        "TEMP",
        state.temperature_c,
        "C",
        &state.temp_hist,
        Rectangle::new(Point::new(10, 32), Size::new((WIDTH as u32) - 20, 124)),
        tiny,
        micro,
        header,
    )?;

    // 湿度图
    render_sparkline_block(
        target,
        "RH",
        state.humidity_pct,
        "%",
        &state.rh_hist,
        Rectangle::new(Point::new(10, 160), Size::new((WIDTH as u32) - 20, 124)),
        tiny,
        micro,
        header,
    )?;

    Ok(())
}

fn render_sparkline_block(
    target: &mut Display<'_>,
    title: &str,
    current: Option<f32>,
    unit: &str,
    hist: &heapless::HistoryBuffer<f32, 120>,
    bounds: Rectangle,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    micro: &MonoTextStyle<'_, BinaryColor>,
    header: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    // 外框
    bounds
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // 头部标签条:反色横条
    let head_h = 18u32;
    let head = Rectangle::new(bounds.top_left, Size::new(bounds.size.width, head_h));
    head.into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(target)?;
    let label_style = MonoTextStyle::new(&FONT_9X18_BOLD, BinaryColor::Off);
    Text::with_baseline(
        title,
        Point::new(bounds.top_left.x + 8, bounds.top_left.y + 1),
        label_style,
        Baseline::Top,
    )
    .draw(target)?;

    // 当前值大字(右上)
    let mut cur_txt: heapless::String<16> = heapless::String::new();
    match current {
        Some(v) => {
            let _ = core::fmt::write(&mut cur_txt, format_args!("{:.1} {}", v, unit));
        }
        None => {
            let _ = cur_txt.push_str("--.-");
        }
    }
    let cur_px = cur_txt.len() as i32 * 9;
    Text::with_baseline(
        &cur_txt,
        Point::new(
            bounds.top_left.x + bounds.size.width as i32 - cur_px - 8,
            bounds.top_left.y + 22,
        ),
        *header,
        Baseline::Top,
    )
    .draw(target)?;

    // 数据统计:min/max/avg
    let mut values: heapless::Vec<f32, 120> = heapless::Vec::new();
    for v in hist.oldest_ordered() {
        let _ = values.push(*v);
    }
    let (vmin, vmax, vavg) = if values.is_empty() {
        (0.0f32, 0.0, 0.0)
    } else {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        let mut sum = 0.0f32;
        for v in values.iter() {
            if *v < lo {
                lo = *v;
            }
            if *v > hi {
                hi = *v;
            }
            sum += *v;
        }
        (lo, hi, sum / values.len() as f32)
    };

    // 左侧:min/max/avg 小字
    let mut stats: heapless::String<32> = heapless::String::new();
    if !values.is_empty() {
        let _ = core::fmt::write(
            &mut stats,
            format_args!("min {:.1}  max {:.1}  avg {:.1}", vmin, vmax, vavg),
        );
    } else {
        let _ = stats.push_str("no data yet");
    }
    Text::with_baseline(
        &stats,
        Point::new(bounds.top_left.x + 8, bounds.top_left.y + 28),
        *micro,
        Baseline::Top,
    )
    .draw(target)?;
    let _ = tiny;

    // 图表区域:y=top+48 到 bottom-4
    let chart_x = bounds.top_left.x + 8;
    let chart_y = bounds.top_left.y + 48;
    let chart_w = bounds.size.width as i32 - 16;
    let chart_h = bounds.size.height as i32 - 48 - 6;
    if values.len() >= 2 {
        draw_sparkline(
            target,
            Point::new(chart_x, chart_y),
            Size::new(chart_w as u32, chart_h as u32),
            &values,
            vmin,
            vmax,
        )?;
    } else {
        // 样本不够画图,画 baseline
        Line::new(
            Point::new(chart_x, chart_y + chart_h / 2),
            Point::new(chart_x + chart_w, chart_y + chart_h / 2),
        )
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    }

    Ok(())
}

/// 把 `values` 等间距映射到 bounding box 内,画 polyline
fn draw_sparkline(
    target: &mut Display<'_>,
    origin: Point,
    size: Size,
    values: &[f32],
    vmin: f32,
    vmax: f32,
) -> Result<(), core::convert::Infallible> {
    let n = values.len() as i32;
    if n < 2 {
        return Ok(());
    }
    let w = size.width as i32;
    let h = size.height as i32;
    // vmin==vmax 时避免除零
    let range = (vmax - vmin).max(0.1);

    // 基准线(0 位等价,这里取中点作轴)
    let axis_y = origin.y + h - 1;
    Line::new(
        Point::new(origin.x, axis_y),
        Point::new(origin.x + w - 1, axis_y),
    )
    .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
    .draw(target)?;

    let to_point = |i: i32, v: f32| -> Point {
        let x = origin.x + (i * (w - 1)) / (n - 1);
        let ratio = ((v - vmin) / range).clamp(0.0, 1.0);
        // y 在 bound 内从 2 到 h-4 (留几像素给轴)
        let y_span = h - 6;
        let y = origin.y + 2 + ((1.0 - ratio) * y_span as f32) as i32;
        Point::new(x, y)
    };

    for i in 1..n {
        let p0 = to_point(i - 1, values[(i - 1) as usize]);
        let p1 = to_point(i, values[i as usize]);
        Line::new(p0, p1)
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(target)?;
    }

    // 最后一个点画个小实心圆圈突出
    let last = to_point(n - 1, values[(n - 1) as usize]);
    Rectangle::new(
        Point::new(last.x - 1, last.y - 1),
        Size::new(3, 3),
    )
    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
    .draw(target)?;

    Ok(())
}

// ============================================================================
// GitHub 页
// ============================================================================

fn render_github(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    micro: &MonoTextStyle<'_, BinaryColor>,
    header: &MonoTextStyle<'_, BinaryColor>,
    _big: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let cx = WIDTH as i32 / 2;
    let center = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    let mut uname: heapless::String<48> = heapless::String::new();
    let user_str = if state.gh_user.is_empty() {
        "unset"
    } else {
        state.gh_user.as_str()
    };
    let _ = core::fmt::write(&mut uname, format_args!("@{}", user_str));

    // ===== 顶栏 y=0..30 =====
    Text::with_baseline(&uname, Point::new(14, 7), *header, Baseline::Top).draw(target)?;
    if state.contrib_valid && state.contrib_total_year > 0 {
        let mut right: heapless::String<40> = heapless::String::new();
        let _ = core::fmt::write(
            &mut right,
            format_args!("{} contributions this year", state.contrib_total_year),
        );
        let w = right.len() as i32 * 6;
        Text::with_baseline(
            &right,
            Point::new(WIDTH as i32 - 14 - w, 17),
            *micro,
            Baseline::Top,
        )
        .draw(target)?;
    }
    Line::new(Point::new(14, 32), Point::new(WIDTH as i32 - 14, 32))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // ===== 数据准备:最近 28 天 =====
    const DAYS: usize = 28;
    let valid_n = (state.contrib_weeks as usize) * 7;
    let have = state.contrib_valid && valid_n >= DAYS;
    let start = valid_n.saturating_sub(DAYS);
    let mut commits: u32 = 0;
    let mut active: u32 = 0;
    let mut max_streak: u32 = 0;
    let mut cur_streak: u32 = 0;
    if have {
        for i in 0..DAYS {
            let lvl = state.contrib[start + i];
            commits += state.contrib_counts[start + i] as u32;
            if lvl > 0 {
                active += 1;
                cur_streak += 1;
                if cur_streak > max_streak {
                    max_streak = cur_streak;
                }
            } else {
                cur_streak = 0;
            }
        }
    }

    // ===== 热力图 7 行 × 4 列(左半)=====
    // cell 18×12, gap 2 → grid 78×96
    const CELL_W: i32 = 18;
    const CELL_H: i32 = 12;
    const CELL_GAP: i32 = 2;
    let col_step = CELL_W + CELL_GAP; // 20
    let row_step = CELL_H + CELL_GAP; // 14
    let grid_w = 4 * CELL_W + 3 * CELL_GAP; // 78
    let grid_h = 7 * CELL_H + 6 * CELL_GAP; // 96
    let grid_x = 40; // 左边 40 px 留给 Mon-Sun 行标
    let grid_y = 60;

    // 顶部列标 4w/3w/2w/1w
    for (i, lbl) in ["4w", "3w", "2w", "1w"].iter().enumerate() {
        let col_cx = grid_x + (i as i32) * col_step + CELL_W / 2;
        let style = embedded_graphics::text::TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Top)
            .build();
        Text::with_text_style(lbl, Point::new(col_cx, grid_y - 14), *micro, style).draw(target)?;
    }

    // 左侧行标 Mon-Sun
    for (i, lbl) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().enumerate() {
        let cy = grid_y + (i as i32) * row_step + CELL_H / 2;
        let style = embedded_graphics::text::TextStyleBuilder::new()
            .alignment(Alignment::Right)
            .baseline(Baseline::Middle)
            .build();
        Text::with_text_style(lbl, Point::new(grid_x - 4, cy), *micro, style).draw(target)?;
    }
    // 格子
    if have {
        for week in 0..4 {
            for day in 0..7 {
                // 按时间顺序:第 week 列是第 w 个 7-天段,day 0..6 对应该段内索引
                let idx = (week as usize) * 7 + (day as usize);
                let level = state.contrib[start + idx];
                let x = grid_x + week * col_step;
                let y = grid_y + day * row_step;
                draw_day_cell(target, Point::new(x, y), CELL_W as u32, CELL_H as u32, level)?;
            }
        }
    } else {
        // 占位框
        Rectangle::new(
            Point::new(grid_x, grid_y),
            Size::new(grid_w as u32, grid_h as u32),
        )
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
        // 框内显示 fetching,错误消息显示在热力图右侧下方(用剩余空间)
        Text::with_text_style(
            "fetching",
            Point::new(grid_x + grid_w / 2, grid_y + grid_h / 2),
            *micro,
            center,
        )
        .draw(target)?;
        if !state.contrib_error.is_empty() {
            let msg = truncate_chars(&state.contrib_error, 40);
            // 画在右侧摘要区下方空位(y≈158 附近),覆盖"1 PR | ..."那行是可以的
            // 更安全:画在左侧热力图下方一行(grid_y + grid_h + 4 = 160)
            let err_y = grid_y + grid_h + 4;
            Text::with_baseline(&msg, Point::new(14, err_y), *micro, Baseline::Top)
                .draw(target)?;
        }
    }

    // ===== 右侧摘要(右半)=====
    let sx = 160i32;
    Text::with_baseline("28-DAY SNAPSHOT", Point::new(sx, 46), *micro, Baseline::Top)
        .draw(target)?;

    let mut l1: heapless::String<24> = heapless::String::new();
    let _ = core::fmt::write(&mut l1, format_args!("{} commits", commits));
    Text::with_baseline(&l1, Point::new(sx, 64), *tiny, Baseline::Top).draw(target)?;

    let mut l2: heapless::String<24> = heapless::String::new();
    let _ = core::fmt::write(&mut l2, format_args!("{} active days", active));
    Text::with_baseline(&l2, Point::new(sx, 86), *tiny, Baseline::Top).draw(target)?;

    let mut l3: heapless::String<24> = heapless::String::new();
    let _ = core::fmt::write(&mut l3, format_args!("{} day streak", max_streak));
    Text::with_baseline(&l3, Point::new(sx, 108), *tiny, Baseline::Top).draw(target)?;

    // 小分隔
    Line::new(Point::new(sx, 132), Point::new(sx + 200, 132))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // 合并 open PR + unread 到一行
    let mut l4: heapless::String<32> = heapless::String::new();
    let _ = core::fmt::write(
        &mut l4,
        format_args!("{} PR | {} unread", state.open_prs, state.notif_count),
    );
    Text::with_baseline(&l4, Point::new(sx, 138), *tiny, Baseline::Top).draw(target)?;

    // ===== 分隔 y=164 =====
    Line::new(Point::new(14, 164), Point::new(WIDTH as i32 - 14, 164))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // ===== LATEST y=170..202 =====
    Text::with_baseline("LATEST", Point::new(14, 170), *micro, Baseline::Top).draw(target)?;
    if state.activity_valid && !state.last_event_line.is_empty() {
        let line_trunc = truncate_chars(&state.last_event_line, 40);
        Text::with_baseline(&line_trunc, Point::new(14, 184), *tiny, Baseline::Top).draw(target)?;
    } else if !state.activity_error.is_empty() {
        let err = truncate_chars(&state.activity_error, 60);
        Text::with_baseline(&err, Point::new(14, 186), *micro, Baseline::Top).draw(target)?;
    } else {
        Text::with_baseline("(fetching...)", Point::new(14, 184), *tiny, Baseline::Top)
            .draw(target)?;
    }

    // ===== 分隔 y=208 =====
    Line::new(Point::new(14, 208), Point::new(WIDTH as i32 - 14, 208))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // ===== UNREAD y=212..282 =====
    // 三点指示 x≈370..390,y≈263..269;文字 x<=346 保证不重叠
    let mut hdr: heapless::String<24> = heapless::String::new();
    if state.notif_valid {
        let _ = core::fmt::write(&mut hdr, format_args!("UNREAD ({})", state.notif_count));
    } else {
        let _ = hdr.push_str("UNREAD");
    }
    Text::with_baseline(&hdr, Point::new(14, 214), *micro, Baseline::Top).draw(target)?;

    if state.notif_valid && state.notif_count == 0 {
        Text::with_baseline("all caught up", Point::new(14, 234), *tiny, Baseline::Top)
            .draw(target)?;
    } else if state.notif_valid && !state.notif_top_title.is_empty() {
        let repo = truncate_chars(&state.notif_top_repo, 37);
        Text::with_baseline(&repo, Point::new(14, 232), *tiny, Baseline::Top).draw(target)?;
        let title = truncate_chars(&state.notif_top_title, 36);
        Text::with_baseline(&title, Point::new(14, 258), *tiny, Baseline::Top).draw(target)?;
    } else {
        Text::with_baseline("(fetching...)", Point::new(14, 234), *tiny, Baseline::Top)
            .draw(target)?;
    }

    Ok(())
}

fn truncate_chars(src: &str, max_chars: usize) -> heapless::String<80> {
    let mut out: heapless::String<80> = heapless::String::new();
    let n = src.chars().count();
    if n <= max_chars {
        for c in src.chars() {
            if out.push(c).is_err() {
                break;
            }
        }
    } else {
        for (i, c) in src.chars().enumerate() {
            if i >= max_chars.saturating_sub(1) {
                break;
            }
            if out.push(c).is_err() {
                break;
            }
        }
        let _ = out.push_str("..");
    }
    out
}

/// 日方块:长方形 (w×h),level 0..=4 映射填充密度。
/// 0 = 仅边框  1 = 1px inset 实心  2 = 2px inset 更窄  3 = 完全填充(略大)  4 = 完全填充
fn draw_day_cell(
    target: &mut Display<'_>,
    origin: Point,
    w: u32,
    h: u32,
    level: u8,
) -> Result<(), core::convert::Infallible> {
    let iw = w as i32;
    let ih = h as i32;
    // 外围 1px 内缩,留 gap 给相邻格视觉分隔
    let ox = origin.x + 1;
    let oy = origin.y + 1;
    let cw = (iw - 2).max(1) as u32;
    let ch = (ih - 2).max(1) as u32;
    match level {
        0 => {
            // 空日:空心边框
            Rectangle::new(Point::new(ox, oy), Size::new(cw, ch))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(target)?;
        }
        1 => {
            // 内部小方块(居中,~30% 面积)
            let inset_x = (cw as i32 / 3).max(1);
            let inset_y = (ch as i32 / 3).max(1);
            Rectangle::new(Point::new(ox, oy), Size::new(cw, ch))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(target)?;
            Rectangle::new(
                Point::new(ox + inset_x, oy + inset_y),
                Size::new((cw as i32 - 2 * inset_x).max(1) as u32, (ch as i32 - 2 * inset_y).max(1) as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(target)?;
        }
        2 => {
            // 上下各留 2px 横条
            Rectangle::new(Point::new(ox, oy), Size::new(cw, ch))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(target)?;
            let pad = 2i32;
            Rectangle::new(
                Point::new(ox + pad, oy + pad),
                Size::new((cw as i32 - 2 * pad).max(1) as u32, (ch as i32 - 2 * pad).max(1) as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(target)?;
        }
        3 => {
            // 几乎全填,留 1px 边框
            Rectangle::new(Point::new(ox, oy), Size::new(cw, ch))
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(target)?;
            Rectangle::new(
                Point::new(ox + 1, oy + 1),
                Size::new((cw as i32 - 2).max(1) as u32, (ch as i32 - 2).max(1) as u32),
            )
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::Off, 1))
            .draw(target)?;
        }
        _ => {
            // 4 = 实心
            Rectangle::new(Point::new(ox, oy), Size::new(cw, ch))
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
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

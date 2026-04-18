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
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
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
    // 实际填充的天数(最后一个索引 = 今天);GraphQL 当周只到今天,不补未来日
    pub contrib_days: u16,
    pub contrib_valid: bool,
    pub contrib_total_year: u32,
    pub contrib_error: heapless::String<80>, // 拉取失败最近一次错误,空=没错误

    // GitHub Notifications(未读)
    pub notif_count: u32,
    pub notif_top_title: heapless::String<96>, // 最新一条 title
    pub notif_top_repo: heapless::String<48>,  // 最新一条仓库 full_name
    pub notif_valid: bool,

    // GitHub 活动:最近一条 event 摘要 + 详情 + 发生时间 + open PR 数
    pub last_event_line: heapless::String<80>,
    pub last_event_detail: heapless::String<96>,
    pub last_event_at_epoch: u64, // 0 = 未知
    pub open_prs: u32,
    pub activity_valid: bool,
    pub activity_error: heapless::String<80>,

    // GitHub 用户名(运行时可改,主循环从 SharedConfig 拷过来)
    pub gh_user: heapless::String<40>,

    // Flash 存储(启动时一次性填,之后不变)
    pub flash_total: u32,
    pub app_part_size: u32,
    pub app_used: u32,
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
            contrib_days: 0,
            contrib_valid: false,
            contrib_total_year: 0,
            contrib_error: heapless::String::new(),
            notif_count: 0,
            notif_top_title: heapless::String::new(),
            notif_top_repo: heapless::String::new(),
            notif_valid: false,
            last_event_line: heapless::String::new(),
            last_event_detail: heapless::String::new(),
            last_event_at_epoch: 0,
            open_prs: 0,
            activity_valid: false,
            activity_error: heapless::String::new(),
            gh_user: heapless::String::new(),
            flash_total: 0,
            app_part_size: 0,
            app_used: 0,
        }
    }
}

// y 坐标分隔线
const Y_SEP_TOP: i32 = 30;
const Y_SEP_CLOCK: i32 = 108;
// TH 段含值 + 下方 mini sparkline(T/RH 10 分钟趋势),高 102px 留足曲线空间
const Y_SEP_TH: i32 = 210;
// wifi 瘦到 24px
const Y_SEP_STATS: i32 = 276;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Github,
}

impl Page {
    pub fn next(self) -> Self {
        match self {
            Self::Dashboard => Self::Github,
            Self::Github => Self::Dashboard,
        }
    }

    pub const ALL: &'static [Page] = &[Page::Dashboard, Page::Github];

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
    // Y_SEP_TH 不画:sparkline 下方紧挨一条直线容易被读成曲线 axis,内容跳跃(曲线→文字)本身足够区分
    for y in [Y_SEP_TOP, Y_SEP_CLOCK, Y_SEP_STATS] {
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

    // 右:电池 / USB(版本挪去 /settings 页,日常无意义)
    let mut right: heapless::String<24> = heapless::String::new();
    match state.battery {
        Some((mv, pct)) => {
            let _ = write!(right, "{}% {}.{:02}V", pct, mv / 1000, (mv % 1000) / 10);
        }
        None => {
            let _ = right.push_str("USB");
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
// 值占中线,无标签(C/%/° 已明示单位)。PROFONT_24 字宽 ~14 px,在数字右侧画
// 5x5 空心圆当 ° 符号,后接 "C"。
// ============================================================================
fn render_th(
    target: &mut Display<'_>,
    state: &AppState,
    val: &MonoTextStyle<'_, BinaryColor>,
    _label: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let center_l = WIDTH as i32 / 4;
    let center_r = WIDTH as i32 * 3 / 4;
    // 值固定在 TH 段上部;下面 ~60px 给 sparkline
    let y_mid = 128;
    let style = embedded_graphics::text::TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    // ---- T:"28.5 °C" 形式,° 靠 Circle 画 ----
    let mut t_txt: heapless::String<8> = heapless::String::new();
    match state.temperature_c {
        Some(t) => {
            let _ = write!(t_txt, "{:.1} C", t);
        }
        None => {
            let _ = t_txt.push_str("--.- C");
        }
    }
    // 整体居中渲染"28.5 C",然后在空格位置叠画 °
    Text::with_text_style(&t_txt, Point::new(center_l, y_mid), *val, style).draw(target)?;
    // "28.5 C" 共 6 字符,空格是第 5 个(索引 4);文字整体中心=center_l,
    // 空格中心相对文字中心偏移:(4 - (6-1)/2.0) * 14 = 21 px
    let space_cx = center_l + 21;
    Circle::new(Point::new(space_cx - 3, y_mid - 11), 6)
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 2))
        .draw(target)?;

    // ---- RH:"57.3 %" ----
    let mut h_txt: heapless::String<8> = heapless::String::new();
    match state.humidity_pct {
        Some(h) => {
            let _ = write!(h_txt, "{:.1} %", h);
        }
        None => {
            let _ = h_txt.push_str("--.- %");
        }
    }
    Text::with_text_style(&h_txt, Point::new(center_r, y_mid), *val, style).draw(target)?;

    // ---- mini sparkline:T 和 RH 各画 10 分钟趋势线,高度 28 ----
    let spark_y = 146;
    let spark_h = 58u32;
    let spark_w = (WIDTH as i32 / 2 - 20) as u32;
    let t_x = 10;
    let rh_x = WIDTH as i32 / 2 + 10;
    // 最小动态范围:温度 0.1°C / 湿度 0.5%;亚度级变化也能看到
    draw_mini_spark(
        target,
        &state.temp_hist,
        Point::new(t_x, spark_y),
        Size::new(spark_w, spark_h),
        0.1,
    )?;
    draw_mini_spark(
        target,
        &state.rh_hist,
        Point::new(rh_x, spark_y),
        Size::new(spark_w, spark_h),
        0.5,
    )?;

    // 中间竖分
    Line::new(
        Point::new(WIDTH as i32 / 2, Y_SEP_CLOCK + 4),
        Point::new(WIDTH as i32 / 2, Y_SEP_TH - 4),
    )
    .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
    .draw(target)?;
    Ok(())
}

/// 从 HistoryBuffer 拉 values,以"中值 ± max(实际半幅, min_span/2)"为 y 范围
/// 画 sparkline。min_span 大于实际波动时曲线会显扁(真"平稳")。
fn draw_mini_spark(
    target: &mut Display<'_>,
    hist: &heapless::HistoryBuffer<f32, 120>,
    origin: Point,
    size: Size,
    min_span: f32,
) -> Result<(), core::convert::Infallible> {
    let mut vals: heapless::Vec<f32, 120> = heapless::Vec::new();
    for v in hist.oldest_ordered() {
        let _ = vals.push(*v);
    }
    if vals.len() < 2 {
        let y = origin.y + size.height as i32 - 1;
        Line::new(
            Point::new(origin.x, y),
            Point::new(origin.x + size.width as i32 - 1, y),
        )
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
        return Ok(());
    }
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for v in vals.iter() {
        if *v < lo {
            lo = *v;
        }
        if *v > hi {
            hi = *v;
        }
    }
    let mid = (hi + lo) / 2.0;
    // 保底半幅 = min_span/2;实际半幅更大才撑开
    let half = ((hi - lo) / 2.0).max(min_span / 2.0);
    draw_sparkline(target, origin, size, &vals, mid - half, mid + half)
}

// ============================================================================
// 系统指标 y=154..232(78px):两行带进度条(APP/HEAP)+ 一行 UP/RST/IDF
// 用 micro(FONT_6X10,高 10px)够清晰又腾得开空间,bar 彻底不会和文字咬。
// 预算:
//   154 分隔
//   160..170 text1(10)
//   174..180 bar1(6)
//   186..196 text2(10)
//   200..206 bar2(6)
//   212..222 text3(10)
//   232 分隔
// ============================================================================
fn render_stats(
    target: &mut Display<'_>,
    state: &AppState,
    _tiny: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let micro = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let left_x = 10;
    // stats 段 210..276(66px),紧凑但清楚
    let line1_y = 216;
    let bar1_y = 228;
    let line2_y = 238;
    let bar2_y = 250;
    let line3_y = 260;

    let bar_w = WIDTH as i32 - 2 * left_x;
    let bar_h = 6i32;

    // Line 1: APP 镜像 / 分区容量
    let app_pct = if state.app_part_size > 0 && state.app_used > 0 {
        (((state.app_used as u64) * 100) / (state.app_part_size as u64)).min(100) as u32
    } else {
        0
    };
    let mut l1: heapless::String<56> = heapless::String::new();
    if state.app_part_size > 0 && state.app_used > 0 {
        let _ = write!(
            l1,
            "APP  {} / {} MB  {}%",
            fmt_mb(state.app_used),
            fmt_mb(state.app_part_size),
            app_pct,
        );
    } else {
        let _ = write!(l1, "FLASH  chip 16.0 MB");
    }
    Text::with_baseline(&l1, Point::new(left_x, line1_y), micro, Baseline::Top).draw(target)?;
    if state.app_part_size > 0 && state.app_used > 0 {
        draw_progress_bar(target, left_x, bar1_y, bar_w, bar_h, app_pct)?;
    }

    // Line 2: SRAM(内部,紧) + PSRAM(外部 8MB,闲)并列同一行
    //   左列 = 内部 SRAM:DMA/WiFi/BLE 必用,吃紧就是吃它
    //   右列 = 外部 PSRAM:大堆,基本空着
    let col_gap = 10i32;
    let col_w = (bar_w - col_gap) / 2;
    let sram_x = left_x;
    let psram_x = left_x + col_w + col_gap;

    let sram_used = state.heap_total.saturating_sub(state.heap_free);
    let sram_pct = if state.heap_total > 0 {
        ((sram_used as u64 * 100) / state.heap_total as u64).min(100) as u32
    } else {
        0
    };
    let mut l_sram: heapless::String<24> = heapless::String::new();
    // SRAM 加一个空格凑齐与 "PSRAM " 同宽(5+1),让两列数值按相同列起点
    let _ = write!(
        l_sram,
        "SRAM  {}/{}K {}%",
        sram_used / 1024,
        state.heap_total / 1024,
        sram_pct,
    );
    Text::with_baseline(&l_sram, Point::new(sram_x, line2_y), micro, Baseline::Top).draw(target)?;
    draw_progress_bar(target, sram_x, bar2_y, col_w, bar_h, sram_pct)?;

    let psram_used = state.psram_total.saturating_sub(state.psram_free);
    let psram_pct = if state.psram_total > 0 {
        ((psram_used as u64 * 100) / state.psram_total as u64).min(100) as u32
    } else {
        0
    };
    let mut l_psram: heapless::String<24> = heapless::String::new();
    if state.psram_total == 0 {
        let _ = write!(l_psram, "PSRAM n/a");
    } else {
        // 0.1 MB 精度;1 MB = 1 048 576,0.1 MB ≈ 104 858
        let used_d = ((psram_used as u64) * 10 + 524_288) / 1_048_576;
        let total_d = ((state.psram_total as u64) * 10 + 524_288) / 1_048_576;
        let _ = write!(
            l_psram,
            "PSRAM {}.{}/{}.{}M {}%",
            used_d / 10,
            used_d % 10,
            total_d / 10,
            total_d % 10,
            psram_pct,
        );
    }
    Text::with_baseline(&l_psram, Point::new(psram_x, line2_y), micro, Baseline::Top)
        .draw(target)?;
    draw_progress_bar(target, psram_x, bar2_y, col_w, bar_h, psram_pct)?;

    // Line 3: UP + RST + IDF
    let up_h = state.uptime_secs / 3600;
    let up_m = (state.uptime_secs / 60) % 60;
    let up_s = state.uptime_secs % 60;
    let mut l3: heapless::String<56> = heapless::String::new();
    let _ = write!(
        l3,
        "UP {:02}:{:02}:{:02}  RST {}  IDF {}",
        up_h, up_m, up_s, state.reset_reason, state.idf_version
    );
    Text::with_baseline(&l3, Point::new(left_x, line3_y), micro, Baseline::Top).draw(target)?;
    Ok(())
}

fn draw_progress_bar(
    target: &mut Display<'_>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pct: u32,
) -> Result<(), core::convert::Infallible> {
    Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    let fill_w = ((w - 2) as u64 * pct as u64 / 100) as u32;
    if fill_w > 0 {
        Rectangle::new(Point::new(x + 1, y + 1), Size::new(fill_w, (h - 2) as u32))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(target)?;
    }
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

    // 瘦身:去掉 SSID(已在 /settings 可见),只保留 IP + RSSI
    let mut text: heapless::String<48> = heapless::String::new();
    if let Some([a, b, c, d]) = state.ip_octets {
        let _ = write!(text, "{}.{}.{}.{}", a, b, c, d);
    }
    if let Some(r) = state.rssi {
        if !text.is_empty() {
            let _ = text.push_str("  ");
        }
        let _ = write!(text, "{} dBm", r);
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

/// 把 `values` 等间距映射到 bounding box 内,画 polyline。
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
    // t_idx = 今天在 contrib[] 里的索引(GraphQL 当周只到今天);
    // 今天的 Mon-first weekday = (t_idx % 7 + 6) % 7(GraphQL 是 Sun-first)
    const DAYS: usize = 28;
    let have = state.contrib_valid && state.contrib_days >= DAYS as u16;
    let t_idx: i32 = (state.contrib_days as i32).saturating_sub(1);
    let today_row_mon: i32 = if t_idx >= 0 { ((t_idx % 7) + 6) % 7 } else { 6 };
    let mut commits: u32 = 0;
    let mut active: u32 = 0;
    let mut max_streak: u32 = 0;
    let mut cur_streak: u32 = 0;
    if have {
        // 往回数 27 天到今天;k=0 最旧,k=27 今天
        for k in 0..(DAYS as i32) {
            let idx = t_idx - (27 - k);
            if idx < 0 {
                continue;
            }
            let idx = idx as usize;
            let lvl = state.contrib[idx];
            commits += state.contrib_counts[idx] as u32;
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
        // 今天位于 (col=3, row=today_row_mon);每格日期 = 今天 +
        // (day - today_row_mon) + (week - 3) * 7
        for week in 0..4i32 {
            for day in 0..7i32 {
                let offset_days = (day - today_row_mon) + (week - 3) * 7;
                let level = if offset_days > 0 {
                    0 // 未来日:空框
                } else {
                    let idx = t_idx + offset_days;
                    if idx < 0 {
                        0
                    } else {
                        state.contrib[idx as usize]
                    }
                };
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

    // ===== LATEST y=170..228 (58px,含 header + event 大字 + detail 小字) =====
    Text::with_baseline("LATEST", Point::new(14, 170), *micro, Baseline::Top).draw(target)?;
    // 右上:相对时间 "5m ago"
    if state.last_event_at_epoch > 0 {
        if let Some(now) = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
        {
            if now > state.last_event_at_epoch {
                let ago = format_ago(now - state.last_event_at_epoch);
                let w = ago.len() as i32 * 6;
                Text::with_baseline(
                    &ago,
                    Point::new(WIDTH as i32 - 14 - w, 170),
                    *micro,
                    Baseline::Top,
                )
                .draw(target)?;
            }
        }
    }
    if state.activity_valid && !state.last_event_line.is_empty() {
        let line_trunc = truncate_chars(&state.last_event_line, 40);
        Text::with_baseline(&line_trunc, Point::new(14, 184), *tiny, Baseline::Top).draw(target)?;
        // 下方小字:commit msg / comment body / PR title 等上下文
        if !state.last_event_detail.is_empty() {
            let detail = truncate_chars(&state.last_event_detail, 62);
            Text::with_baseline(&detail, Point::new(14, 208), *micro, Baseline::Top)
                .draw(target)?;
        }
    } else if !state.activity_error.is_empty() {
        let err = truncate_chars(&state.activity_error, 60);
        Text::with_baseline(&err, Point::new(14, 186), *micro, Baseline::Top).draw(target)?;
    } else {
        Text::with_baseline("(fetching...)", Point::new(14, 184), *tiny, Baseline::Top)
            .draw(target)?;
    }

    // ===== 分隔 y=232 =====
    Line::new(Point::new(14, 232), Point::new(WIDTH as i32 - 14, 232))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // ===== UNREAD y=236..280(44px,单行:"UNREAD (N) repo" + title 一行) =====
    // 翻页点 x≈370..390, y≈285..291(Y_SEP_STATS=276 时)
    let mut hdr: heapless::String<80> = heapless::String::new();
    if state.notif_valid {
        let _ = core::fmt::write(&mut hdr, format_args!("UNREAD ({})", state.notif_count));
    } else {
        let _ = hdr.push_str("UNREAD");
    }
    // 头行加上 repo:把之前独立一行的 repo 并进 header 省掉一整行
    if state.notif_valid && state.notif_count > 0 && !state.notif_top_repo.is_empty() {
        let _ = core::fmt::write(&mut hdr, format_args!("  {}", state.notif_top_repo));
    }
    let hdr_trunc = truncate_chars(&hdr, 60);
    Text::with_baseline(&hdr_trunc, Point::new(14, 240), *micro, Baseline::Top).draw(target)?;

    if state.notif_valid && state.notif_count == 0 {
        Text::with_baseline("all caught up", Point::new(14, 256), *tiny, Baseline::Top)
            .draw(target)?;
    } else if state.notif_valid && !state.notif_top_title.is_empty() {
        let title = truncate_chars(&state.notif_top_title, 36);
        Text::with_baseline(&title, Point::new(14, 256), *tiny, Baseline::Top).draw(target)?;
    } else {
        Text::with_baseline("(fetching...)", Point::new(14, 256), *tiny, Baseline::Top)
            .draw(target)?;
    }

    Ok(())
}

/// 秒差格式化:"30s ago" / "5m ago" / "2h ago" / "3d ago" / "4mo ago"
fn format_ago(sec: u64) -> heapless::String<12> {
    let mut s: heapless::String<12> = heapless::String::new();
    let _ = if sec < 60 {
        core::fmt::write(&mut s, format_args!("{}s ago", sec))
    } else if sec < 3600 {
        core::fmt::write(&mut s, format_args!("{}m ago", sec / 60))
    } else if sec < 86400 {
        core::fmt::write(&mut s, format_args!("{}h ago", sec / 3600))
    } else if sec < 30 * 86400 {
        core::fmt::write(&mut s, format_args!("{}d ago", sec / 86400))
    } else {
        core::fmt::write(&mut s, format_args!("{}mo ago", sec / (30 * 86400)))
    };
    s
}

/// 把字节数格式化成"1.7"或"16.0"这种一位小数 MB 数。
fn fmt_mb(bytes: u32) -> heapless::String<8> {
    let mut s: heapless::String<8> = heapless::String::new();
    // 用整数算法避免 f32 开销和精度误差:(bytes * 10 / 1024 / 1024) → 得到 *.1 一位小数的整数
    let tenths = ((bytes as u64) * 10 / (1024 * 1024)) as u32;
    let whole = tenths / 10;
    let frac = tenths % 10;
    let _ = core::fmt::write(&mut s, format_args!("{}.{}", whole, frac));
    s
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

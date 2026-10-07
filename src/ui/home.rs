//! Useful household status instead of a mostly online/offline device grid.
use u8g2_fonts::fonts::u8g2_font_wqy16_t_gb2312;

use super::*;

fn label(target: &mut Display<'_>, text: &str, x: i32, y: i32) {
    let font = FontRenderer::new::<u8g2_font_wqy16_t_gb2312>();
    let _ = font.render_aligned(
        text,
        Point::new(x, y),
        VerticalPosition::Baseline,
        HorizontalAlignment::Left,
        FontColor::Transparent(BinaryColor::On),
        target,
    );
}

pub(super) fn render_home(
    target: &mut Display<'_>,
    state: &AppState,
    _tiny: &MonoTextStyle<'_, BinaryColor>,
    _micro: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let bt = &state.bluetooth;
    label(target, "迈极炫灯", 18, 28);
    label(
        target,
        if bt.lamp_ready {
            "蓝牙已连接"
        } else {
            "蓝牙未连接"
        },
        280,
        28,
    );
    Line::new(Point::new(18, 42), Point::new(382, 42))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    let battery = if bt.lamp_ready { bt.battery } else { None };
    let value = battery.map_or_else(|| "--".into(), |v| v.to_string());
    let font = FontRenderer::new::<u8g2_font_logisoso58_tn>();
    let _ = font.render_aligned(
        value.as_str(),
        Point::new(108, 128),
        VerticalPosition::Baseline,
        HorizontalAlignment::Center,
        FontColor::Transparent(BinaryColor::On),
        target,
    );
    label(target, "%", 171, 126);
    label(target, "电量（设备估值）", 38, 158);
    let power = if !bt.lamp_ready {
        "状态未知".to_owned()
    } else {
        match bt.level {
            Some(0) => "灯已关闭".into(),
            Some(v) => format!("亮度 {v} 档"),
            None => "亮度待确认".into(),
        }
    };
    label(target, &power, 239, 92);
    label(target, "翻页器", 239, 125);
    label(
        target,
        if bt.remote_ready {
            "已连接"
        } else {
            "未连接"
        },
        239,
        152,
    );
    let age = match (battery, bt.battery_age_s) {
        (Some(_), Some(s)) if s < 60 => "电量刚刚更新".into(),
        (Some(_), Some(s)) => format!("电量 {} 分钟前更新", s / 60),
        _ => "等待新的电量读数".into(),
    };
    label(target, &age, 18, 191);
    Line::new(Point::new(18, 210), Point::new(382, 210))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    label(target, "室内温湿度", 18, 239);
    let clock = &state.scale.clock;
    let temp = clock
        .temperature
        .as_ref()
        .filter(|r| r.received.elapsed().as_secs() < 600)
        .map_or_else(|| "--".into(), |r| format!("{:.1}", r.value));
    let rh = clock
        .humidity
        .as_ref()
        .filter(|r| r.received.elapsed().as_secs() < 600)
        .map_or_else(|| "--".into(), |r| format!("{:.0}", r.value));
    label(target, &format!("{temp} ℃     {rh}%"), 18, 272);
    label(target, "米家时钟", 302, 272);
    Ok(())
}

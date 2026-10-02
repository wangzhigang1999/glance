//! Compact device tiles; larger households rotate in groups of nine.
use super::*;
use crate::net::miio::DeviceState;

fn label(device: &DeviceState) -> String {
    let model = device.model.as_str();
    if model.contains("acpartner") {
        // Stable suffix survives DHCP changes and distinguishes identical models.
        let suffix = &device.id[device.id.len().saturating_sub(3)..];
        format!("AC #{suffix}")
    } else {
        match model {
            m if m.contains("humidifier") => "HUMIDIFIER",
            m if m.contains("kettle") => "KETTLE",
            m if m.contains("plug") => "SMART PLUG",
            m if m.contains("gateway") => "HOME HUB",
            m if m.contains("camera") => "CAMERA",
            m if m.contains("airp") => "AIR PURIFIER",
            m if m.contains("light") => "MONITOR LIGHT",
            _ => "DEVICE",
        }
        .into()
    }
}

fn reading(device: &DeviceState) -> (String, &'static str) {
    if !device.online {
        return ("--".into(), "OFFLINE");
    }
    let fields: &[(&str, &str)] = if device.model.contains("humidifier") {
        &[("humidity_pct", "% RH")]
    } else if device.model.contains("kettle") {
        &[("temperature_c", "CELSIUS")]
    } else {
        &[("power_w", "WATTS"), ("pm25_ug_m3", "PM2.5 ug/m3")]
    };
    for &(key, unit) in fields {
        if let Some(value) = device.values.get(key).and_then(|v| v.as_f64()) {
            return (format!("{value:.0}"), unit);
        }
    }
    match device.values.get("on").and_then(|v| v.as_bool()) {
        Some(on) => (if on { "ON" } else { "OFF" }.into(), "POWER"),
        None => ("READY".into(), "CONNECTED"),
    }
}

pub(super) fn render_home(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    micro: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let devices = &state.home.devices;
    let online = devices.iter().filter(|d| d.online).count();
    Text::new("HOME DEVICES", Point::new(14, 24), *tiny).draw(target)?;
    Text::with_alignment(
        &format!("{online}/{} ONLINE", devices.len()),
        Point::new(384, 24),
        *micro,
        Alignment::Right,
    )
    .draw(target)?;
    // Keep the type readable at 400x300 instead of squeezing sixteen devices into one page.
    let pages = devices.len().div_ceil(9).max(1);
    let page = (state.uptime_secs / 15) as usize % pages;
    for (index, device) in devices.iter().skip(page * 9).take(9).enumerate() {
        let x = 12 + (index % 3) as i32 * 128;
        let y = 38 + (index / 3) as i32 * 78;
        Rectangle::new(Point::new(x, y), Size::new(120, 70))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(target)?;
        Text::new(&label(device), Point::new(x + 7, y + 14), *micro).draw(target)?;
        let (value, unit) = reading(device);
        Text::with_alignment(&value, Point::new(x + 60, y + 40), *tiny, Alignment::Center)
            .draw(target)?;
        Text::with_alignment(unit, Point::new(x + 60, y + 57), *micro, Alignment::Center)
            .draw(target)?;
    }
    if devices.is_empty() {
        Text::new("ADD DEVICES IN THE WEB APP", Point::new(28, 146), *micro).draw(target)?;
    }
    let footer = if pages > 1 {
        format!("LOCAL / 60s     {}/{}", page + 1, pages)
    } else {
        "LOCAL READINGS / 60s".into()
    };
    Text::new(&footer, Point::new(14, 291), *micro).draw(target)?;
    Ok(())
}

use super::*;

pub(super) fn render_weight(
    target: &mut Display<'_>,
    state: &AppState,
    tiny: &MonoTextStyle<'_, BinaryColor>,
    micro: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let s = &state.scale;
    Text::new("WEIGHT", Point::new(14, 23), *tiny).draw(target)?;
    let scan_status = if !s.error.is_empty() {
        "CHECK ERROR"
    } else if s.scanning {
        "LISTENING"
    } else {
        "STARTING"
    };
    Text::with_alignment(scan_status, Point::new(386, 23), *micro, Alignment::Right)
        .draw(target)?;
    Line::new(Point::new(2, 32), Point::new(397, 32))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    let fresh = s.last_seen.is_some_and(|t| t.elapsed().as_secs() < 15);
    let live = if fresh { s.live_kg } else { None };
    let value = live.or_else(|| s.history.last().map(|r| r.kg));
    let number = value
        .map(|kg| format!("{kg:.2}"))
        .unwrap_or_else(|| "--.--".into());
    let font = FontRenderer::new::<u8g2_font_logisoso58_tn>();
    let _ = font.render_aligned(
        number.as_str(),
        Point::new(200, 115),
        VerticalPosition::Baseline,
        HorizontalAlignment::Center,
        FontColor::Transparent(BinaryColor::On),
        target,
    );
    Text::with_alignment("kg", Point::new(200, 139), *tiny, Alignment::Center).draw(target)?;
    let label = match (live, value, s.stable) {
        (Some(_), _, true) => "STABLE",
        (Some(_), _, false) => "MEASURING...",
        (None, Some(_), _) => "LAST STABLE READING",
        _ => "STEP ON YOUR SCALE",
    };
    Text::with_alignment(label, Point::new(200, 166), *tiny, Alignment::Center).draw(target)?;
    Line::new(Point::new(14, 181), Point::new(386, 181))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    Text::new("RECENT STABLE READINGS", Point::new(14, 198), *micro).draw(target)?;
    for (i, reading) in s.history.iter().rev().take(3).enumerate() {
        let date = reading
            .unix_secs
            .map(|t| {
                let (_, m, d, h, min, _) = crate::net::time::utc_from_unix(t + state.tz_offset);
                format!("{m:02}-{d:02} {h:02}:{min:02}")
            })
            .unwrap_or_else(|| "time unknown".into());
        let text = format!("{}  {}  {:.2} kg", i + 1, date, reading.kg);
        Text::new(&text, Point::new(14, 218 + i as i32 * 17), *micro).draw(target)?;
    }
    let detail = if s.history.is_empty() && s.error.is_empty() {
        "WAITING FOR FIRST STABLE READING".to_string()
    } else if !s.error.is_empty() || s.storage_pending {
        "LOCAL SAVE PENDING - RETRYING".to_string()
    } else if state.cloud_error || !state.cloud_connected {
        format!(
            "LOCAL SAVED | CLOUD OFFLINE | {} queued",
            state.cloud_queued
        )
    } else if state.cloud_queued > 0 {
        format!("LOCAL SAVED | UPLOADING {}", state.cloud_queued)
    } else {
        "LOCAL SAVED | CLOUD CONNECTED".to_string()
    };
    Text::new(&detail, Point::new(14, 269), *micro).draw(target)?;
    render_bottom_bar(target, state, micro)
}

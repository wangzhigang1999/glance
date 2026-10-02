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
    // Keep persistence and connectivity next to the receiver status, away from the chart.
    let local = if s.storage_pending || !s.error.is_empty() {
        "SAVE PENDING"
    } else if s.history.is_empty() {
        "NO DATA"
    } else {
        "SAVED"
    };
    let cloud = if state.cloud_error || !state.cloud_connected {
        "CLOUD OFF"
    } else if state.cloud_queued > 0 {
        "CLOUD QUEUED"
    } else {
        "CLOUD OK"
    };
    Text::new(&format!("{local} / {cloud}"), Point::new(104, 23), *micro).draw(target)?;
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
    let number_bounds = font.render_aligned(
        number.as_str(),
        Point::new(200, 102),
        VerticalPosition::Baseline,
        HorizontalAlignment::Center,
        FontColor::Transparent(BinaryColor::On),
        target,
    );
    // Attach the unit to the actual glyph bounds, including wider three-digit weights.
    let unit_x = number_bounds
        .ok()
        .flatten()
        .and_then(|bounds| bounds.bottom_right())
        .map_or(300, |corner| corner.x + 9);
    Text::new("kg", Point::new(unit_x, 104), *micro).draw(target)?;
    Line::new(Point::new(14, 118), Point::new(386, 118))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;
    render_history(target, state, micro)
}

/// Summaries of retained local readings only; no cloud read path.
fn render_history(
    target: &mut Display<'_>,
    state: &AppState,
    micro: &MonoTextStyle<'_, BinaryColor>,
) -> Result<(), core::convert::Infallible> {
    let days = super::weight_daily::summarize(
        state.scale.history.iter().map(|r| (r.unix_secs, r.kg)),
        state.tz_offset,
    );
    if days.is_empty() {
        Text::with_alignment(
            "NO DATED LOCAL READINGS",
            Point::new(200, 207),
            *micro,
            Alignment::Center,
        )
        .draw(target)?;
        return Ok(());
    }
    let minimum = days.iter().map(|d| d.min).fold(f32::INFINITY, f32::min);
    let maximum = days.iter().map(|d| d.max).fold(f32::NEG_INFINITY, f32::max);
    let lower = ((minimum - 0.1) * 2.0).floor() / 2.0;
    let upper = ((maximum + 0.1) * 2.0).ceil() / 2.0;
    const LEFT: i32 = 55;
    const RIGHT: i32 = 378;
    const TOP: i32 = 132;
    const BOTTOM: i32 = 263;
    let y_for =
        |kg: f32| BOTTOM - ((kg - lower) / (upper - lower) * (BOTTOM - TOP) as f32).round() as i32;
    for kg in [upper, (upper + lower) / 2.0, lower] {
        let y = y_for(kg);
        Text::with_alignment(
            &format!("{kg:.2}"),
            Point::new(46, y + 3),
            *micro,
            Alignment::Right,
        )
        .draw(target)?;
        for x in (LEFT..RIGHT).step_by(6) {
            Line::new(Point::new(x, y), Point::new(x + 1, y))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(target)?;
        }
    }
    let start = days[0].day;
    let span = days.last().unwrap().day - start;
    let x_for = |day: i64| {
        if span == 0 {
            (LEFT + RIGHT) / 2
        } else {
            LEFT + ((day - start) * i64::from(RIGHT - LEFT) / span) as i32
        }
    };
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let mut previous: Option<(i64, Point)> = None;
    for day in &days {
        let x = x_for(day.day);
        let low = y_for(day.min);
        let high = y_for(day.max);
        let middle = Point::new(x, y_for(day.median));
        // Whiskers stay legible on a one-bit LCD, unlike translucent web range bands.
        Line::new(Point::new(x, high), Point::new(x, low))
            .into_styled(stroke)
            .draw(target)?;
        for y in [high, low] {
            Line::new(Point::new(x - 4, y), Point::new(x + 4, y))
                .into_styled(stroke)
                .draw(target)?;
        }
        if let Some((before_day, before)) = previous {
            // A missing calendar day stays a visible gap, never an interpolated observation.
            if day.day == before_day + 1 {
                Line::new(before, middle).into_styled(stroke).draw(target)?;
            }
        }
        Rectangle::new(Point::new(x - 2, middle.y - 2), Size::new(5, 5))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(target)?;
        previous = Some((day.day, middle));
    }
    // Up to seven date labels; the positions always preserve calendar-day spacing.
    let tick_count = span.min(6);
    for index in 0..=tick_count {
        let day = if tick_count == 0 {
            start
        } else {
            start + index * span / tick_count
        };
        let (_, month, date, _, _, _) = crate::net::time::utc_from_unix(day * 86400);
        let alignment = if tick_count == 0 {
            Alignment::Center
        } else if index == 0 {
            Alignment::Left
        } else if index == tick_count {
            Alignment::Right
        } else {
            Alignment::Center
        };
        Text::with_alignment(
            &format!("{month:02}/{date:02}"),
            Point::new(x_for(day), 281),
            *micro,
            alignment,
        )
        .draw(target)?;
    }
    Ok(())
}

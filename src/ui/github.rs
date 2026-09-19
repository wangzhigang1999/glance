use super::*;

pub(super) fn render_github(
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

    // 未配置 user 或 token 时,整页显示配置提示,不渲染空壳面板
    if state.gh_user.is_empty() || !state.gh_token_set {
        return render_github_unconfigured(target, state, tiny, micro, header);
    }

    let mut uname: heapless::String<48> = heapless::String::new();
    let username = truncate_chars(&state.gh_user, 15);
    let user_str = username.as_str();
    let _ = core::fmt::write(&mut uname, format_args!("@{}", user_str));

    let health = if state.gh_health.failed {
        "UPDATE FAILED - CACHED DATA".to_string()
    } else if let Some(t) = state.gh_health.updated {
        format!("UPDATED {}", format_ago(t.elapsed().as_secs()))
    } else {
        "UPDATING...".to_string()
    };
    Text::new(&health, Point::new(14, 292), *micro).draw(target)?;

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
    for (i, lbl) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .iter()
        .enumerate()
    {
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
                draw_day_cell(
                    target,
                    Point::new(x, y),
                    CELL_W as u32,
                    CELL_H as u32,
                    level,
                )?;
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
            Text::with_baseline(&msg, Point::new(14, err_y), *micro, Baseline::Top).draw(target)?;
        }
    }

    // ===== 右侧摘要(右半)=====
    let sx = 160i32;
    Text::with_baseline("28-DAY SNAPSHOT", Point::new(sx, 46), *micro, Baseline::Top)
        .draw(target)?;

    let mut l1: heapless::String<24> = heapless::String::new();
    let _ = core::fmt::write(&mut l1, format_args!("{} contributions", commits));
    Text::with_baseline(&l1, Point::new(sx, 64), *tiny, Baseline::Top).draw(target)?;

    let mut l2: heapless::String<24> = heapless::String::new();
    let _ = core::fmt::write(&mut l2, format_args!("{} active days", active));
    Text::with_baseline(&l2, Point::new(sx, 86), *tiny, Baseline::Top).draw(target)?;

    let mut l3: heapless::String<24> = heapless::String::new();
    let _ = core::fmt::write(&mut l3, format_args!("best streak: {}d", max_streak));
    Text::with_baseline(&l3, Point::new(sx, 108), *tiny, Baseline::Top).draw(target)?;

    // 小分隔
    Line::new(Point::new(sx, 132), Point::new(sx + 200, 132))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(target)?;

    // 合并 open PR + unread 到一行
    let mut l4: heapless::String<32> = heapless::String::new();
    let _ = core::fmt::write(
        &mut l4,
        format_args!(
            "{} PR | {}{} unread",
            state
                .open_prs
                .map(|n| n.to_string())
                .unwrap_or_else(|| "--".into()),
            state.notif_count,
            if state.notif_count >= 30 { "+" } else { "" }
        ),
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
        let _ = core::fmt::write(
            &mut hdr,
            format_args!(
                "UNREAD ({}{})",
                state.notif_count,
                if state.notif_count >= 30 { "+" } else { "" }
            ),
        );
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

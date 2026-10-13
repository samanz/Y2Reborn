//! The time charts: Hourly (temperature curve, icons and the chance of rain
//! under an hour cursor), Rain (amount and chance over two days, then a
//! week) and Wind (a compass and speed with gusts over a day).
use super::{
    amount, arrow, card, degrees, fall, hour_icon, label, line, no_hours, pair, shape, speed,
    speed_unit, window, BOTTOM, DROP, HOURS_AHEAD, SIDE,
};
use crate::{
    canvas::Canvas,
    text::{self, Face},
    theme::{alpha, color, mix, WIDTH},
};
use reborn_core::weather::{beaufort, compass, day_name, hour_text, weekday, Forecast, Hour};

/// Hours shown at once on Hourly.
const SPAN: usize = 24;

/// Catmull-Rom points through `p`, `per` steps between each pair.
fn smooth(p: &[(f32, f32)], per: usize) -> Vec<(f32, f32)> {
    if p.len() < 3 {
        return p.to_vec();
    }
    let mut out = Vec::with_capacity(p.len() * per);
    for i in 0..p.len() - 1 {
        let p0 = p[i.saturating_sub(1)];
        let (p1, p2) = (p[i], p[i + 1]);
        let p3 = p[(i + 2).min(p.len() - 1)];
        for s in 0..per {
            let t = s as f32 / per as f32;
            let (t2, t3) = (t * t, t * t * t);
            let at = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2. * b
                    + (-a + c) * t
                    + (2. * a - 5. * b + 4. * c - d) * t2
                    + (-a + 3. * b - 3. * c + d) * t3)
            };
            out.push((at(p0.0, p1.0, p2.0, p3.0), at(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out.push(p[p.len() - 1]);
    out
}

/// A filled curve: `fill` fading down to `floor`, the line on top.
fn curve(c: &mut Canvas, points: &[(f32, f32)], floor: f32, fill: u32, stroke: u32, width: f32) {
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        shape(
            c,
            [[a.0, a.1], [b.0, b.1], [a.0, floor], [b.0, floor]],
            fill,
            alpha(fill, 0x02),
        );
    }
    for w in points.windows(2) {
        line(c, w[0], w[1], width, stroke);
    }
}

/// "Now", "3 pm", or "Tue 3 pm" on another day.
fn when(f: &Forecast, h: &Hour, first: bool, now: i64) -> String {
    if first {
        return "Now".to_owned();
    }
    let t = f.local(h.time);
    if t.days == f.local(now).days {
        f.hour_label(h.time)
    } else {
        format!("{} {}", day_name(&t), f.hour_label(h.time))
    }
}

/// An axis label for an hour: the day's name at midnight, else the hour,
/// short ("6 am", "18").
fn axis_label(f: &Forecast, time: i64) -> String {
    let t = f.local(time);
    if t.hour == 0 {
        day_name(&t).to_owned()
    } else if f.twelve_hour() {
        hour_text(t.hour, true)
    } else {
        format!("{:02}", t.hour)
    }
}

/// The two days ahead, from the current hour.
fn ahead(f: &Forecast, now: i64) -> &[Hour] {
    let hs = f.hours_from(now);
    &hs[..hs.len().min(HOURS_AHEAD)]
}

/// The temperature curve over a day with icons and the chance of rain under
/// a cursor the wheel moves; the cursor's hour in full above.
pub(super) fn hourly(c: &mut Canvas, f: &Forecast, cursor: usize, accent: u32, now: i64) {
    let hs = ahead(f, now);
    if hs.len() < 2 {
        return no_hours(c);
    }
    let cursor = cursor.min(hs.len() - 1);
    let h = &hs[cursor];
    // The hour under the cursor.
    let head = when(f, h, cursor == 0, now);
    c.text_mid(SIDE, 86., 130., &head, Face::S17, color::TEXT);
    c.text_mid(
        SIDE,
        108.,
        130.,
        h.condition.label(),
        Face::R13,
        color::MUTED,
    );
    hour_icon(c, h, 160., 96., 40.);
    let temp = degrees(h.temperature);
    c.text_mid(186., 96., 70., &temp, Face::S28, color::TEXT);
    let (a, b) = (264., 384.);
    pair(c, a, 86., "Feels", &degrees(h.feels_like), Face::R13);
    pair(c, b, 86., "Rain", &format!("{}%", h.rain_chance), Face::R13);
    let wind = format!("{} {}", speed(f, h.wind), compass(h.wind_direction));
    pair(c, a, 108., "Wind", &wind, Face::R13);
    pair(
        c,
        b,
        108.,
        "UV",
        &format!("{}", h.uv.round() as i32),
        Face::R13,
    );

    // The chart.
    let start = window(hs.len(), cursor, SPAN);
    let shown = &hs[start..(start + SPAN).min(hs.len())];
    let (x0, x1) = (SIDE + 12., WIDTH - SIDE - 12.);
    let step = (x1 - x0) / (SPAN - 1) as f32;
    let x = |i: usize| x0 + i as f32 * step;
    let (top, bottom) = (128., 302.);
    // Night behind the hours of darkness.
    let mut i = 0;
    while i < shown.len() {
        if shown[i].is_day {
            i += 1;
            continue;
        }
        let from = i;
        while i < shown.len() && !shown[i].is_day {
            i += 1;
        }
        let left = (x(from) - step / 2.).max(SIDE);
        let right = (x(i - 1) + step / 2.).min(WIDTH - SIDE);
        c.rrect(
            left,
            top,
            right - left,
            bottom - top,
            8.,
            color::GLASS_FAINT,
        );
    }
    // The cursor's column.
    let cx = x(cursor - start);
    c.rrect(
        cx - step / 2. + 1.,
        top,
        step - 2.,
        bottom - top,
        7.,
        color::GLASS,
    );
    // Temperatures.
    let (lo, hi) = shown.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| {
        (lo.min(h.temperature), hi.max(h.temperature))
    });
    let pad = ((6. - (hi - lo)) / 2.).max(0.);
    let (lo, hi) = (lo - pad, hi + pad);
    let (ct, cb) = (170., 224.);
    let y = |t: f32| cb - (t - lo) / (hi - lo) * (cb - ct);
    let points: Vec<_> = shown
        .iter()
        .enumerate()
        .map(|(i, h)| (x(i), y(h.temperature)))
        .collect();
    let stroke = mix(accent, color::TEXT, 0.35);
    curve(
        c,
        &smooth(&points, 4),
        236.,
        alpha(accent, 0x48),
        stroke,
        2.6,
    );
    for (i, h) in shown.iter().enumerate() {
        let hour = f.local(h.time).hour;
        let (px, py) = points[i];
        if hour.is_multiple_of(3) {
            hour_icon(c, h, px, 142., 20.);
        }
        // Every third hour's temperature, and the cursor's (its neighbours
        // give way to it).
        let at_cursor = i + start == cursor;
        if at_cursor || (hour.is_multiple_of(3) && (i + start).abs_diff(cursor) > 1) {
            let (face, tint) = if at_cursor {
                (Face::S13, color::TEXT)
            } else {
                (Face::R11, color::MUTED)
            };
            c.text_mid_center(px, py - 13., &degrees(h.temperature), face, tint);
        }
    }
    // The cursor on the curve.
    let (px, py) = points[cursor - start];
    c.disc(px, py, 6., accent);
    c.disc(px, py, 2.5, color::BG);
    // Chance of rain.
    let (bt, bb) = (246., 280.);
    for (i, h) in shown.iter().enumerate() {
        let chance = f32::from(h.rain_chance) / 100.;
        let tall = (chance * (bb - bt)).max(2.);
        let tint = alpha(DROP, (0x38 as f32 + chance * 0xB0 as f32) as u8);
        c.rrect(x(i) - step / 2. + 3., bb - tall, step - 6., tall, 2., tint);
    }
    // Hours.
    for (i, h) in shown.iter().enumerate() {
        let first = start + i == 0;
        let hour = f.local(h.time).hour;
        if first || (hour.is_multiple_of(6) && !(start == 0 && i < 3)) {
            let label = if first {
                "Now".to_owned()
            } else {
                axis_label(f, h.time)
            };
            let w = text::width(Face::R11, &label);
            let lx = x(i).clamp(SIDE + w / 2., WIDTH - SIDE - w / 2.);
            let tint = if hour == 0 { color::TEXT } else { color::DIM };
            c.text_mid_center(lx, 292., &label, Face::R11, tint);
        }
    }
}

/// Amount and chance hour by hour over two days, then a week of totals.
pub(super) fn rain(c: &mut Canvas, f: &Forecast, now: i64) {
    c.text_mid(
        SIDE,
        84.,
        WIDTH - 2. * SIDE,
        &f.summary(now),
        Face::S15,
        color::TEXT,
    );
    let hs = ahead(f, now);
    if hs.len() < 2 {
        return no_hours(c);
    }
    let total: f32 = hs.iter().map(|h| h.precipitation).sum();
    let tiny = if f.imperial { 0.005 } else { 0.1 };
    let sub = if total < tiny {
        format!("No rain expected in the next {} hours", hs.len())
    } else {
        format!("{} in the next {} hours", amount(f, total), hs.len())
    };
    c.text_mid(SIDE, 106., 300., &sub, Face::R13, color::MUTED);
    // Legend.
    let lx = WIDTH - SIDE - 140.;
    c.rrect(lx, 101., 10., 10., 3., alpha(DROP, 0x40));
    c.text_mid(lx + 15., 106., 60., "Chance", Face::R11, color::DIM);
    c.rrect(lx + 72., 101., 10., 10., 3., DROP);
    c.text_mid(lx + 87., 106., 60., "Amount", Face::R11, color::DIM);

    let (top, bottom) = (128., 204.);
    let step = (WIDTH - 2. * SIDE) / hs.len() as f32;
    let scale = hs
        .iter()
        .map(|h| h.precipitation)
        .fold(if f.imperial { 0.08 } else { 2. }, f32::max);
    for i in 0..3 {
        let gy = top + i as f32 * (bottom - top) / 2.;
        c.rect(SIDE, gy, WIDTH - 2. * SIDE, 1., color::RULE);
    }
    for (i, h) in hs.iter().enumerate() {
        let x = SIDE + i as f32 * step;
        let chance = f32::from(h.rain_chance) / 100. * (bottom - top);
        if chance >= 1. {
            c.rrect(
                x + 1.,
                bottom - chance,
                step - 2.,
                chance,
                2.,
                alpha(DROP, 0x40),
            );
        }
        let wet = h.precipitation / scale * (bottom - top);
        if wet >= 0.5 {
            let wet = wet.max(2.);
            fall(
                c,
                x + 2.,
                bottom - wet,
                step - 4.,
                wet,
                mix(DROP, 0xFFFFFFFF, 0.35),
                DROP,
            );
        }
        let t = f.local(h.time);
        if t.hour == 0 && i > 0 {
            c.rect(x, top - 4., 1., bottom - top + 8., color::TRACK);
        }
        if t.hour.is_multiple_of(6) && i > 1 && i + 2 < hs.len() {
            let tint = if t.hour == 0 { color::TEXT } else { color::DIM };
            c.text_mid_center(x, 216., &axis_label(f, h.time), Face::R11, tint);
        }
    }
    c.text_mid(SIDE, 216., 40., "Now", Face::R11, color::DIM);
    // The week.
    let days = f.days_from(now);
    let days = &days[..days.len().min(7)];
    if days.is_empty() {
        return;
    }
    let top = 230.;
    card(c, SIDE, top, WIDTH - 2. * SIDE, BOTTOM - top);
    let col = (WIDTH - 2. * SIDE) / days.len() as f32;
    let scale = days
        .iter()
        .map(|d| d.precipitation)
        .fold(if f.imperial { 0.2 } else { 5. }, f32::max);
    for (i, d) in days.iter().enumerate() {
        let mid = SIDE + col * (i as f32 + 0.5);
        let name = if i == 0 { "Today" } else { weekday(&d.date) };
        c.text_mid_center(mid, top + 14., name, Face::R11, color::MUTED);
        let (bt, bb) = (top + 26., top + 64.);
        c.rrect(mid - 9., bt, 18., bb - bt, 5., color::GLASS_FAINT);
        let tall = d.precipitation / scale * (bb - bt);
        if tall >= 0.5 {
            let tall = tall.max(4.);
            c.rrect(mid - 9., bb - tall, 18., tall, 5., DROP);
        }
        c.text_mid_center(
            mid,
            top + 77.,
            &amount(f, d.precipitation),
            Face::R11,
            if tall >= 0.5 { color::TEXT } else { color::DIM },
        );
        if let Some(r) = d.rain_chance {
            c.text_mid_center(mid, top + 91., &format!("{r}%"), Face::R11, DROP);
        }
    }
}

/// A compass with the wind now; speed and gusts over a day with the
/// direction every three hours; the strongest wind today.
pub(super) fn wind(c: &mut Canvas, f: &Forecast, accent: u32, now: i64) {
    let h = f.conditions(now);
    let (cx, cy, r) = (106., 186., 78.);
    // The dial.
    for i in 0..72 {
        let (s, k) = (i as f32 * 5.).to_radians().sin_cos();
        let (inner, tint, w) = if i % 18 == 0 {
            (r - 10., color::TEXT, 2.)
        } else if i % 2 == 0 {
            (r - 6., color::MUTED, 1.2)
        } else {
            (r - 4., color::TRACK, 1.)
        };
        line(
            c,
            (cx + s * inner, cy - k * inner),
            (cx + s * r, cy - k * r),
            w,
            tint,
        );
    }
    for (name, a) in [("N", 0.), ("E", 90.), ("S", 180.), ("W", 270.)] {
        let (s, k) = f32::to_radians(a).sin_cos();
        let d = r - 22.;
        c.text_mid_center(cx + s * d, cy - k * d, name, Face::S13, color::MUTED);
    }
    // The wind blows from its direction across the dial.
    let from = f32::from(h.wind_direction);
    let (s, k) = from.to_radians().sin_cos();
    let at = |d: f32| (cx + s * d, cy - k * d);
    let tail = at(r - 12.);
    c.disc(tail.0, tail.1, 4.5, accent);
    line(c, at(r - 14.), at(-(r - 34.)), 3., accent);
    let head = at(-(r - 20.));
    arrow(c, head.0, head.1, 22., from + 180., accent);
    // The hub.
    c.disc(cx, cy, 34., color::BG);
    c.disc(cx, cy, 34., color::GLASS_FAINT);
    let speed_now = format!("{}", h.wind.round() as i32);
    c.text_mid_center(cx, cy - 6., &speed_now, Face::S28, color::TEXT);
    c.text_mid_center(cx, cy + 18., speed_unit(f), Face::R11, color::MUTED);
    let kmh = if f.imperial { h.wind * 1.609 } else { h.wind };
    c.text_mid_center(cx, 290., beaufort(kmh), Face::S15, color::TEXT);
    let gusts = format!(
        "From {}, gusts {}",
        compass(h.wind_direction),
        speed(f, h.gusts)
    );
    c.text_mid_center(
        cx,
        312.,
        &text::fit(Face::R13, &gusts, 196.),
        Face::R13,
        color::MUTED,
    );

    // Speed and gusts over the next day.
    let hs = ahead(f, now);
    let (x0, x1) = (230., WIDTH - SIDE - 8.);
    if hs.len() < 2 {
        c.text_mid_center(
            (x0 + x1) / 2.,
            180.,
            "No hourly wind yet",
            Face::R13,
            color::MUTED,
        );
        return;
    }
    let hs = &hs[..hs.len().min(SPAN + 1)];
    let step = (x1 - x0) / (hs.len() - 1) as f32;
    let x = |i: usize| x0 + i as f32 * step;
    let (top, bottom) = (100., 214.);
    let unit = if f.imperial { 10. } else { 20. };
    let most = hs.iter().map(|h| h.gusts.max(h.wind)).fold(unit, f32::max);
    let scale = (most / unit).ceil() * unit;
    let y = |v: f32| bottom - v / scale * (bottom - top);
    // Grid with its speeds.
    let lines = (scale / unit) as usize;
    let every = lines.div_ceil(3).max(1);
    for g in (0..=lines).step_by(every) {
        let v = g as f32 * unit;
        c.rect(x0, y(v), x1 - x0, 1., color::RULE);
        if g > 0 {
            let label = format!("{}", v as i32);
            c.text_mid_right(x0 - 6., y(v), &label, Face::R11, color::DIM);
        }
    }
    let speeds: Vec<_> = hs
        .iter()
        .enumerate()
        .map(|(i, h)| (x(i), y(h.wind)))
        .collect();
    let gusts: Vec<_> = hs
        .iter()
        .enumerate()
        .map(|(i, h)| (x(i), y(h.gusts.max(h.wind))))
        .collect();
    let (speeds, gusts) = (smooth(&speeds, 3), smooth(&gusts, 3));
    // The gusts above the speed, as a band.
    for (g, s) in gusts.windows(2).zip(speeds.windows(2)) {
        shape(
            c,
            [
                [g[0].0, g[0].1],
                [g[1].0, g[1].1],
                [s[0].0, s[0].1],
                [s[1].0, s[1].1],
            ],
            alpha(accent, 0x50),
            alpha(accent, 0x20),
        );
    }
    for w in gusts.windows(2) {
        line(c, w[0], w[1], 1.2, alpha(accent, 0x90));
    }
    let stroke = mix(accent, color::TEXT, 0.35);
    for w in speeds.windows(2) {
        line(c, w[0], w[1], 2.6, stroke);
    }
    // Legend.
    c.rect(x0, 83., 14., 3., stroke);
    c.text_mid(x0 + 20., 84., 60., "Speed", Face::R11, color::DIM);
    c.rrect(x0 + 72., 79., 14., 10., 3., alpha(accent, 0x50));
    c.text_mid(x0 + 92., 84., 60., "Gusts", Face::R11, color::DIM);
    c.text_mid_right(x1, 84., speed_unit(f), Face::R11, color::DIM);
    // Direction every three hours, then the hours.
    for (i, h) in hs.iter().enumerate() {
        let hour = f.local(h.time).hour;
        if i % 3 == 0 {
            let to = f32::from(h.wind_direction) + 180.;
            arrow(c, x(i), 234., 15., to, color::MUTED);
        }
        if i == 0 || (hour.is_multiple_of(6) && i > 1 && i + 2 < hs.len()) {
            let label = if i == 0 {
                "Now".to_owned()
            } else {
                axis_label(f, h.time)
            };
            let w = text::width(Face::R11, &label);
            let lx = x(i).clamp(x0 + w / 2. - 8., x1 - w / 2.);
            c.text_mid_center(lx, 256., &label, Face::R11, color::DIM);
        }
    }
    // Today's strongest.
    if let Some(d) = f.today(now).map(|i| &f.days[i]) {
        let top = 272.;
        card(c, x0 - 18., top, x1 - x0 + 26., BOTTOM - top);
        label(c, x0 - 4., top + 17., 220., "Strongest today");
        c.text_mid(
            x0 - 4.,
            top + 39.,
            x1 - x0,
            &format!(
                "{}, gusts {} from {}",
                speed(f, d.wind_max),
                d.gust_max.round() as i32,
                compass(d.wind_direction)
            ),
            Face::S13,
            color::TEXT,
        );
    }
}

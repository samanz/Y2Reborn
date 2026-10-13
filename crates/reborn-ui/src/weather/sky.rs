//! The day pages: 10 days on one temperature scale, Sun & moon, Air & UV
//! and the Details grid.
use super::{
    amount, card, celsius, degrees, fitting, gradient_pill, icon, label, line, no_hours, ramp,
    shape, temp_color, BOTTOM, CLOUD, DROP, MOON, SIDE, SUN, TOP,
};
use crate::{
    canvas::Canvas,
    text::{self, Face},
    theme::{alpha, color, mix, WIDTH},
};
use reborn_core::weather::{
    aqi_category, date_label, daylight_change_text, duration_text, moon_illumination, moon_name,
    moon_phase, next_moon, uv_category, weekday, Forecast, Hour, Place,
};

/// Where a day's low-to-high bar sits on a scale shared by every day,
/// (x, width), at least `min_w` wide and inside the track.
fn range_bar(low: f32, high: f32, lo: f32, hi: f32, x: f32, w: f32, min_w: f32) -> (f32, f32) {
    let span = (hi - lo).max(1e-3);
    let at = |t: f32| x + ((t - lo) / span).clamp(0., 1.) * w;
    let (mut a, mut b) = (at(low.min(high)), at(high.max(low)));
    if b - a < min_w {
        let mid = ((a + b) / 2.).clamp(x + min_w / 2., x + w - min_w / 2.);
        (a, b) = (mid - min_w / 2., mid + min_w / 2.);
    }
    (a, b - a)
}

/// Ten days: name, icon, chance of rain, low, a range bar coloured by
/// temperature, high; today's bar marks the temperature now.
pub(super) fn days(c: &mut Canvas, f: &Forecast, now: i64) {
    let ds = f.days_from(now);
    let ds = &ds[..ds.len().min(10)];
    if ds.is_empty() {
        return no_hours(c);
    }
    let lo = ds.iter().map(|d| d.low).fold(f32::MAX, f32::min);
    let hi = ds.iter().map(|d| d.high).fold(f32::MIN, f32::max);
    let row = (BOTTOM - TOP) / 10.;
    let (bx, bw) = (186., 178.);
    let now_temp = f.conditions(now).temperature;
    for (i, d) in ds.iter().enumerate() {
        let cy = TOP + row * (i as f32 + 0.5);
        if i > 0 {
            c.rect(
                SIDE,
                TOP + row * i as f32,
                WIDTH - 2. * SIDE,
                1.,
                color::RULE,
            );
        }
        let name = if i == 0 { "Today" } else { weekday(&d.date) };
        c.text_mid(SIDE, cy, 52., name, Face::S13, color::TEXT);
        icon(c, d.condition, true, 70., cy - 11., 22.);
        if let Some(r) = d.rain_chance.filter(|&r| r >= 20) {
            c.text_mid(98., cy, 40., &format!("{r}%"), Face::R11, DROP);
        }
        c.text_mid_right(bx - 10., cy, &degrees(d.low), Face::R13, color::MUTED);
        c.rrect(bx, cy - 3., bw, 6., 3., color::GLASS);
        let (x, w) = range_bar(d.low, d.high, lo, hi, bx, bw, 6.);
        gradient_pill(
            c,
            x,
            cy - 3.,
            w,
            6.,
            temp_color(celsius(f, d.low)),
            temp_color(celsius(f, d.high)),
        );
        if i == 0 {
            let (tx, _) = range_bar(now_temp, now_temp, lo, hi, bx, bw, 0.);
            c.disc(tx, cy, 5.5, color::BG);
            c.disc(tx, cy, 3.5, color::TEXT);
        }
        let high = degrees(d.high);
        c.text_mid(bx + bw + 10., cy, 50., &high, Face::S13, color::TEXT);
        let wet = amount(f, d.precipitation);
        if !wet.starts_with("0 ") {
            c.text_mid_right(WIDTH - SIDE, cy, &wet, Face::R11, color::MUTED);
        }
    }
}

/// The moon's disc lit for `phase`, mirrored south of the equator.
fn moon(c: &mut Canvas, cx: f32, cy: f32, r: f32, phase: f32, south: bool) {
    let (dark, lit) = (0x2A2D36FF, MOON);
    let glow = (moon_illumination(phase) * 60.) as u8;
    c.glow(cx, cy, r * 1.7, r * 1.7, alpha(lit, glow));
    c.disc(cx, cy, r, dark);
    let k = (std::f32::consts::TAU * phase).cos();
    for row in 0..(2. * r) as i32 {
        let dy = row as f32 + 0.5 - r;
        let half = (r * r - dy * dy).max(0.).sqrt();
        // Lit from the terminator to the limb: the right side waxing.
        let (mut a, mut b) = if phase < 0.5 {
            (half * k, half)
        } else {
            (-half, -half * k)
        };
        if south {
            (a, b) = (-b, -a);
        }
        let (a, b) = (cx + a, cx + b);
        if b - a < 0.05 {
            continue;
        }
        let y = (cy + dy - 0.5).round();
        // Whole pixels, then the partly covered ones at each end.
        let (fa, fb) = (a.ceil(), b.floor());
        if fb > fa {
            c.rect(fa, y, fb - fa, 1., lit);
        }
        let edge = |c: &mut Canvas, px: f32, cover: f32| {
            let cover = cover.clamp(0., 1.);
            if cover > 0.02 {
                c.rect(px, y, 1., 1., alpha(lit, (cover * 255.) as u8));
            }
        };
        if fb >= fa {
            edge(c, fa - 1., fa - a);
            edge(c, fb, b - fb);
        } else {
            edge(c, a.floor(), b - a);
        }
    }
}

/// The sun's arc from sunrise to sunset with where it is now, the length of
/// the day and its change; the moon's phase and the next full and new moon.
pub(super) fn sun_and_moon(c: &mut Canvas, f: &Forecast, p: &Place, now: i64) {
    let Some(d) = f.today(now).map(|i| &f.days[i]) else {
        return no_hours(c);
    };
    let top = TOP;
    card(c, SIDE, top, WIDTH - 2. * SIDE, 158.);
    let (cx, horizon, rx, ry) = (WIDTH / 2., top + 116., 168., 92.);
    let at = |t: f32| {
        let a = std::f32::consts::PI * (1. - t);
        (cx + rx * a.cos(), horizon - ry * a.sin())
    };
    let span = (d.sunset - d.sunrise) as f32;
    let up = d.sunrise > 0 && d.sunset > d.sunrise;
    let done = if up {
        ((now - d.sunrise) as f32 / span).clamp(0., 1.)
    } else {
        0.
    };
    // The sky filled under the part of the arc the sun has crossed.
    const STEPS: usize = 64;
    for k in 0..STEPS {
        let (t0, t1) = (k as f32 / STEPS as f32, (k + 1) as f32 / STEPS as f32);
        let (a, b) = (at(t0), at(t1));
        if t1 <= done + 1e-3 {
            shape(
                c,
                [[a.0, a.1], [b.0, b.1], [a.0, horizon], [b.0, horizon]],
                alpha(SUN, 0x48),
                alpha(SUN, 0x06),
            );
            line(c, a, b, 2.6, SUN);
        } else if k % 2 == 0 {
            line(c, a, b, 1.6, color::TRACK);
        }
    }
    c.rect(
        SIDE + 14.,
        horizon,
        WIDTH - 2. * SIDE - 28.,
        1.,
        color::TRACK,
    );
    if up && now > d.sunrise && now < d.sunset {
        let (sx, sy) = at(done);
        c.glow(sx, sy, 34., 34., alpha(SUN, 0x70));
        c.disc(sx, sy, 9., SUN);
    }
    let (l, r) = (at(0.).0, at(1.).0);
    if up {
        for (x, name, t) in [(l, "Sunrise", d.sunrise), (r, "Sunset", d.sunset)] {
            let w = text::width(Face::R11, name);
            label(c, x - w / 2., horizon + 14., w + 2., name);
            c.text_mid_center(x, horizon + 31., &f.time_label(t), Face::S13, color::TEXT);
        }
    }
    let length = if d.daylight > 0. {
        format!("{} of daylight", duration_text(d.daylight))
    } else {
        "No sunrise today".to_owned()
    };
    c.text_mid_center(cx, horizon + 14., &length, Face::S13, color::TEXT);
    if let Some(change) = f.daylight_change(now) {
        c.text_mid_center(
            cx,
            horizon + 31.,
            &daylight_change_text(change),
            Face::R11,
            color::MUTED,
        );
    }
    // Before sunrise or after sunset: when the sun is next up.
    if up && (now < d.sunrise || now >= d.sunset) {
        let next = if now < d.sunrise {
            Some(d.sunrise)
        } else {
            f.today(now)
                .and_then(|i| f.days.get(i + 1))
                .map(|d| d.sunrise)
                .filter(|&t| t > now)
        };
        if let Some(t) = next {
            let mins = (t - now) / 60;
            c.text_mid_center(
                cx,
                top + 66.,
                &format!("Sunrise in {} h {} m", mins / 60, mins % 60),
                Face::R13,
                color::MUTED,
            );
        }
    }

    // The moon.
    let top = top + 166.;
    card(c, SIDE, top, WIDTH - 2. * SIDE, BOTTOM - top);
    let phase = moon_phase(now);
    let my = top + (BOTTOM - top) / 2.;
    moon(c, SIDE + 50., my, 30., phase, p.latitude < 0.);
    let x = SIDE + 98.;
    c.text_mid(x, top + 22., 170., moon_name(phase), Face::S17, color::TEXT);
    c.text_mid(
        x,
        top + 46.,
        170.,
        &format!("{}% lit", (moon_illumination(phase) * 100.).round() as i32),
        Face::R13,
        color::MUTED,
    );
    c.text_mid(
        x,
        top + 67.,
        170.,
        &format!("Day {} of 29", (phase * 29.53).floor() as i32 + 1),
        Face::R11,
        color::DIM,
    );
    let mut next = [
        ("Full moon", next_moon(now, 0.5)),
        ("New moon", next_moon(now, 0.)),
    ];
    next.sort_by_key(|n| n.1);
    let x = WIDTH - SIDE - 150.;
    for (i, (name, t)) in next.iter().enumerate() {
        let y = top + 20. + i as f32 * 36.;
        label(c, x, y, 140., name);
        let date = date_label(&f.local(*t));
        c.text_mid(x, y + 16., 140., &date, Face::S13, color::TEXT);
    }
}

/// The US AQI's colour.
fn aqi_color(aqi: f32) -> u32 {
    match aqi as u32 {
        0..=50 => 0x3CC878FF,
        51..=100 => 0xF2D04BFF,
        101..=150 => 0xFF9440FF,
        151..=200 => 0xFF5148FF,
        201..=300 => 0xA65BD9FF,
        _ => 0x9B2D4AFF,
    }
}

/// The UV index's colour.
fn uv_color(uv: f32) -> u32 {
    match uv.round() as i32 {
        ..=2 => 0x3CC878FF,
        3..=5 => 0xF2D04BFF,
        6..=7 => 0xFF9440FF,
        8..=10 => 0xFF5148FF,
        _ => 0xA65BD9FF,
    }
}

/// Pollen in grains per cubic metre, in words.
fn pollen_level(grains: f32) -> &'static str {
    match grains {
        g if g < 10. => "Low",
        g if g < 50. => "Moderate",
        g if g < 200. => "High",
        _ => "Very high",
    }
}

/// A gauge of the air quality index, the UV index on its scale with today's
/// peak, and the pollutants (and pollen where it is measured).
pub(super) fn air(c: &mut Canvas, f: &Forecast, now: i64) {
    let top = TOP;
    let half = (WIDTH - 2. * SIDE - 10.) / 2.;
    let tall = 176.;
    card(c, SIDE, top, half, tall);
    card(c, SIDE + half + 10., top, half, tall);
    let aqi = f.air.as_ref().and_then(|a| a.us_aqi);
    // The gauge: 240 degrees, 0 to 300.
    let (gx, gy, gr) = (SIDE + half / 2., top + 86., 58.);
    label(c, SIDE + 14., top + 18., half, "Air quality");
    match aqi {
        Some(aqi) => {
            let value = f32::from(aqi).min(300.);
            let at = |v: f32, r: f32| {
                let (s, k) = (-120. + 240. * v / 300.).to_radians().sin_cos();
                [gx + s * r, gy - k * r]
            };
            // A ring of joined pieces: the coloured bands up to the value,
            // the track beyond.
            let (outer, inner) = (gr + 4., gr - 4.);
            const STEPS: usize = 72;
            for k in 0..STEPS {
                let (v0, v1) = (
                    300. * k as f32 / STEPS as f32,
                    300. * (k + 1) as f32 / STEPS as f32,
                );
                let band = aqi_color((v0 + v1) / 2.);
                let tint = if v0 < value { band } else { alpha(band, 0x34) };
                let v1 = if v0 < value { v1.min(value) } else { v1 };
                let piece = [at(v0, outer), at(v1, outer), at(v0, inner), at(v1, inner)];
                shape(c, piece, tint, tint);
                // The rest of the piece the value falls in.
                let end = v0 + 300. / STEPS as f32;
                if v0 < value && v1 < end {
                    let rest = alpha(band, 0x34);
                    let piece = [at(v1, outer), at(end, outer), at(v1, inner), at(end, inner)];
                    shape(c, piece, rest, rest);
                }
            }
            let [ex, ey] = at(0., gr);
            c.disc(ex, ey, 4., aqi_color(0.));
            let [mx, my] = at(value, gr);
            c.disc(mx, my, 8., color::TEXT);
            c.disc(mx, my, 5., aqi_color(value));
            c.text_mid_center(gx, gy - 2., &format!("{aqi}"), Face::S36, color::TEXT);
            c.text_mid_center(gx, gy + 26., "US AQI", Face::R11, color::DIM);
            c.text_mid_center(
                gx,
                top + tall - 22.,
                aqi_category(aqi),
                Face::S13,
                mix(aqi_color(value), color::TEXT, 0.15),
            );
        }
        None => c.text_mid_center(gx, gy, "Not available here", Face::R13, color::MUTED),
    }

    // UV.
    let h = f.conditions(now);
    let x = SIDE + half + 24.;
    let w = half - 28.;
    label(c, x, top + 18., w, "UV index");
    let value = format!("{}", h.uv.round() as i32);
    c.text_mid(x, top + 56., 60., &value, Face::S36, color::TEXT);
    c.text_mid(
        x + text::width(Face::S36, &value) + 10.,
        top + 60.,
        w,
        uv_category(h.uv),
        Face::S15,
        mix(uv_color(h.uv), color::TEXT, 0.15),
    );
    let (sy, sh) = (top + 98., 8.);
    let stops = [
        (0., 0x3CC878FF),
        (3., 0xF2D04BFF),
        (6., 0xFF9440FF),
        (8.5, 0xFF5148FF),
        (12., 0xA65BD9FF),
    ];
    let sx = |uv: f32| x + uv.clamp(0., 12.) / 12. * w;
    for pair in stops.windows(2) {
        let (a, b) = (sx(pair[0].0), sx(pair[1].0));
        ramp(c, a, sy, b - a, sh, pair[0].1, pair[1].1);
    }
    c.disc(x, sy + sh / 2., sh / 2., stops[0].1);
    c.disc(x + w, sy + sh / 2., sh / 2., stops[4].1);
    let mx = sx(h.uv);
    c.disc(mx, sy + sh / 2., 7., color::TEXT);
    c.disc(mx, sy + sh / 2., 4., uv_color(h.uv));
    for (v, mark) in [(0., "0"), (3., "3"), (6., "6"), (8., "8"), (11., "11+")] {
        c.text_mid_center(sx(v).max(x + 4.), sy + 22., mark, Face::R11, color::DIM);
    }
    // Today's peak, from the hours when known.
    let today = f.local(now).days;
    let peak = f
        .hours
        .iter()
        .filter(|h| f.local(h.time).days == today)
        .max_by(|a, b| a.uv.total_cmp(&b.uv));
    let line_text = match (peak, f.today(now).map(|i| &f.days[i])) {
        (Some(p), _) if p.uv >= 0.5 => {
            format!("Peak {} at {}", p.uv.round() as i32, f.hour_label(p.time))
        }
        (_, Some(d)) => format!("Peak today {}", d.uv_max.round() as i32),
        _ => String::new(),
    };
    c.text_mid(x, top + 146., w, &line_text, Face::R13, color::MUTED);

    // Pollutants and pollen.
    let top = top + tall + 8.;
    card(c, SIDE, top, WIDTH - 2. * SIDE, BOTTOM - top);
    let Some(a) = &f.air else {
        c.text_mid_center(
            WIDTH / 2.,
            (top + BOTTOM) / 2.,
            "No air quality readings",
            Face::R13,
            color::MUTED,
        );
        return;
    };
    let unit = "µg/m³";
    let mut cols: Vec<(String, String, String)> = [
        ("PM2.5", a.pm2_5),
        ("PM10", a.pm10),
        ("Ozone", a.ozone),
        ("NO2", a.nitrogen_dioxide),
    ]
    .into_iter()
    .map(|(name, v)| {
        let value = v.map_or("-".to_owned(), |v| {
            if v < 10. {
                format!("{v:.1}")
            } else {
                format!("{}", v.round() as i32)
            }
        });
        (name.to_owned(), value, unit.to_owned())
    })
    .collect();
    if let Some(p) = a.pollen.iter().max_by(|x, y| x.grains.total_cmp(&y.grains)) {
        cols.push((
            "Pollen".to_owned(),
            pollen_level(p.grains).to_owned(),
            if p.grains >= 1. {
                p.name.clone()
            } else {
                "All types".to_owned()
            },
        ));
    }
    let col = (WIDTH - 2. * SIDE) / cols.len() as f32;
    for (i, (name, value, sub)) in cols.iter().enumerate() {
        let mid = SIDE + col * (i as f32 + 0.5);
        label(
            c,
            mid - text::width(Face::R11, name) / 2.,
            top + 17.,
            col,
            name,
        );
        c.text_mid_center(mid, top + 38., value, Face::S17, color::TEXT);
        c.text_mid_center(
            mid,
            top + 58.,
            &text::fit(Face::R11, sub, col - 8.),
            Face::R11,
            color::DIM,
        );
    }
}

/// A slim bar with `fraction` filled.
fn meter(c: &mut Canvas, x: f32, y: f32, w: f32, fraction: f32, tint: u32) {
    c.rrect(x, y, w, 6., 3., color::GLASS);
    let fill = (fraction.clamp(0., 1.) * w).max(6.);
    c.rrect(x, y, fill, 6., 3., tint);
}

/// Humidity, pressure and its trend, visibility, cloud cover, dew point
/// and what the temperature feels like, each with a small picture.
pub(super) fn details(c: &mut Canvas, f: &Forecast, accent: u32, now: i64) {
    let h = f.conditions(now);
    let gap = 8.;
    let cw = (WIDTH - 2. * SIDE - 2. * gap) / 3.;
    let ch = (BOTTOM - TOP - gap) / 2.;
    let cell = |i: usize| {
        (
            SIDE + (i % 3) as f32 * (cw + gap),
            TOP + (i / 3) as f32 * (ch + gap),
        )
    };
    let value = |c: &mut Canvas, x: f32, y: f32, v: &str, unit: &str| {
        let room = cw - 28. - text::width(Face::R13, unit) - 4.;
        let face = fitting(&[Face::S25, Face::S20, Face::S17], v, room);
        c.text_mid(x + 14., y + 52., cw - 28., v, face, color::TEXT);
        if !unit.is_empty() {
            let vw = text::width(face, v);
            c.text_mid(x + 18. + vw, y + 56., cw, unit, Face::R13, color::DIM);
        }
    };
    let head = |c: &mut Canvas, i: usize, name: &str, note: &str| {
        let (x, y) = cell(i);
        card(c, x, y, cw, ch);
        label(c, x + 14., y + 20., cw - 28., name);
        c.text_mid(x + 14., y + 82., cw - 28., note, Face::R13, color::MUTED);
        (x, y)
    };
    let bar_y = |y: f32| y + ch - 20.;

    // Humidity.
    let dew = format!("Dew point {}", degrees(h.dew_point));
    let (x, y) = head(c, 0, "Humidity", &dew);
    value(c, x, y, &format!("{}%", h.humidity), "");
    let wet = f32::from(h.humidity) / 100.;
    meter(c, x + 14., bar_y(y), cw - 28., wet, DROP);

    // Pressure, with four hours past and twelve to come.
    let trend = f.pressure_trend(now);
    let note = match trend {
        Some(t) if t >= 1. => "Rising",
        Some(t) if t <= -1. => "Falling",
        Some(_) => "Steady",
        None => "",
    };
    let (x, y) = head(c, 1, "Pressure", note);
    if h.pressure > 0. {
        let (v, unit) = if f.imperial {
            (format!("{:.2}", h.pressure * 0.02953), "inHg")
        } else {
            (format!("{}", h.pressure.round() as i32), "hPa")
        };
        value(c, x, y, &v, unit);
    } else {
        value(c, x, y, "-", "");
    }
    let spark: Vec<&Hour> = f
        .hours
        .iter()
        .filter(|p| p.time >= now - 4 * 3600 && p.time <= now + 12 * 3600 && p.pressure > 0.)
        .collect();
    if spark.len() >= 2 {
        if let Some(t) = trend {
            let change = format!("{t:+.1}");
            c.text_mid_right(x + cw - 14., y + 82., &change, Face::R11, color::DIM);
        }
        let (lo, hi) = spark.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.pressure), hi.max(p.pressure))
        });
        let pad = ((4. - (hi - lo)) / 2.).max(0.);
        let (lo, hi) = (lo - pad, hi + pad);
        let (sx0, sw, sy0, sh) = (x + 14., cw - 28., bar_y(y) - 8., 16.);
        let (t0, t1) = (spark[0].time, spark[spark.len() - 1].time);
        let pt = |p: &Hour| {
            (
                sx0 + (p.time - t0) as f32 / (t1 - t0) as f32 * sw,
                sy0 + sh - (p.pressure - lo) / (hi - lo) * sh,
            )
        };
        for w in spark.windows(2) {
            let tint = if w[1].time <= now {
                color::MUTED
            } else {
                accent
            };
            line(c, pt(w[0]), pt(w[1]), 2., tint);
        }
        if let Some(n) = spark.iter().find(|p| p.time <= now && now < p.time + 3600) {
            let (px, py) = pt(n);
            c.disc(px, py, 4., color::TEXT);
        }
    }

    // Visibility.
    let km = if f.imperial {
        h.visibility * 1.609
    } else {
        h.visibility
    };
    let (word, far) = match km {
        v if v >= 10. => ("Clear", 1.),
        v if v >= 5. => ("Good", 0.7),
        v if v >= 2. => ("Moderate", 0.45),
        v if v >= 1. => ("Poor", 0.25),
        _ => ("Fog", 0.1),
    };
    let note = if h.visibility > 0. { word } else { "" };
    let (x, y) = head(c, 2, "Visibility", note);
    let v = if h.visibility >= 10. {
        format!("{}", h.visibility.round() as i32)
    } else {
        format!("{:.1}", h.visibility)
    };
    value(c, x, y, &v, if f.imperial { "mi" } else { "km" });
    let tint = mix(accent, color::TEXT, 0.3);
    meter(c, x + 14., bar_y(y), cw - 28., far, tint);

    // Cloud cover.
    let word = match h.cloud_cover {
        88.. => "Overcast",
        63..=87 => "Mostly cloudy",
        25..=62 => "Partly cloudy",
        5..=24 => "Mostly clear",
        _ => "Clear",
    };
    let (x, y) = head(c, 3, "Cloud cover", word);
    value(c, x, y, &format!("{}%", h.cloud_cover), "");
    let cover = f32::from(h.cloud_cover) / 100.;
    meter(c, x + 14., bar_y(y), cw - 28., cover, CLOUD);

    // Dew point on a comfort scale.
    let dew = celsius(f, h.dew_point);
    let word = match dew {
        d if d < 10. => "Dry",
        d if d < 16. => "Comfortable",
        d if d < 21. => "Humid",
        _ => "Muggy",
    };
    let (x, y) = head(c, 4, "Dew point", word);
    value(c, x, y, &degrees(h.dew_point), "");
    let (sx, sw) = (x + 14., cw - 28.);
    let by = bar_y(y);
    let bands = [0x5AA8FFFF, 0x6BDB8CFF, 0xF2D04BFF, 0xFF9440FF];
    let bw = sw / bands.len() as f32;
    for (i, tint) in bands.iter().enumerate() {
        let bx = sx + i as f32 * bw + 1.;
        c.rrect(bx, by, bw - 2., 6., 3., alpha(*tint, 0xB0));
    }
    let at = sx + ((dew + 4.) / 32.).clamp(0., 1.) * sw;
    c.disc(at, by + 3., 6., color::TEXT);
    c.disc(at, by + 3., 3., temp_color(dew));

    // Feels like.
    let diff = h.feels_like - h.temperature;
    let note = match diff {
        d if d <= -1.5 => format!("{} below actual", degrees(-d)),
        d if d >= 1.5 => format!("{} above actual", degrees(d)),
        _ => "Close to actual".to_owned(),
    };
    let (x, y) = head(c, 5, "Feels like", &note);
    value(c, x, y, &degrees(h.feels_like), "");
    let (lo, hi) = (
        h.temperature.min(h.feels_like),
        h.temperature.max(h.feels_like),
    );
    let (sx, sw) = (x + 14., cw - 28.);
    c.rrect(sx, bar_y(y), sw, 6., 3., color::GLASS);
    let scale = |t: f32| sx + ((t - (lo - 6.)) / (hi - lo + 12.)).clamp(0., 1.) * sw;
    let (a, b) = (scale(lo), scale(hi));
    gradient_pill(
        c,
        a - 3.,
        bar_y(y),
        b - a + 6.,
        6.,
        temp_color(celsius(f, lo)),
        temp_color(celsius(f, hi)),
    );
    c.disc(scale(h.feels_like), bar_y(y) + 3., 5., color::TEXT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_bars_share_one_scale() {
        // Lows 0 to highs 20 across 200 px.
        assert_eq!(range_bar(5., 15., 0., 20., 100., 200., 6.), (150., 100.));
        assert_eq!(range_bar(0., 20., 0., 20., 100., 200., 6.), (100., 200.));
        // A day with no range still shows a dot-wide bar, inside the track.
        assert_eq!(range_bar(20., 20., 0., 20., 100., 200., 6.), (294., 6.));
        assert_eq!(range_bar(0., 0., 0., 20., 100., 200., 6.), (100., 6.));
        // Out of range values stay on the track.
        let (x, w) = range_bar(-10., 40., 0., 20., 100., 200., 6.);
        assert_eq!((x, x + w), (100., 300.));
    }
}

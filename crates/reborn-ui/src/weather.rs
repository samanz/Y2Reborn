//! Weather: the chosen city's forecast, fetched over Wi-Fi by the app and
//! shown as eight pages: Now, Hourly, Rain, Wind, 10 days, Sun & moon, Air &
//! UV and Details. Select and Next/Previous turn the page; the wheel turns it
//! too, except on Hourly, where it moves an hour cursor along the next two
//! days and carries on to the neighbouring page past either end. Holding
//! Select chooses another city. The saved forecast shows offline with how
//! old it is.
use crate::{
    canvas::Canvas,
    text::{self, Face},
    theme::{alpha, color, mix, WIDTH},
    Item, RowKind, Ui,
};
use reborn_core::{
    cities::{cities_in, countries, CITIES},
    weather::{compass, uv_category, Condition, Forecast, Hour, Place},
    Action, AppModel, Effect, Screen,
};
use reborn_graphics::{Quad, Warp};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod charts;
mod sky;

/// Forecasts older than this are refreshed when the screen opens or a page
/// is turned.
pub const STALE_AFTER: Duration = Duration::from_secs(15 * 60);

/// The pages, in the order Next and the wheel visit them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    #[default]
    Now,
    Hourly,
    Rain,
    Wind,
    Days,
    Sky,
    Air,
    Details,
}

pub const PAGES: [Page; 8] = [
    Page::Now,
    Page::Hourly,
    Page::Rain,
    Page::Wind,
    Page::Days,
    Page::Sky,
    Page::Air,
    Page::Details,
];

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Self::Now => "Now",
            Self::Hourly => "Hourly",
            Self::Rain => "Rain",
            Self::Wind => "Wind",
            Self::Days => "10 days",
            Self::Sky => "Sun & moon",
            Self::Air => "Air & UV",
            Self::Details => "Details",
        }
    }

    fn index(self) -> usize {
        PAGES.iter().position(|&p| p == self).unwrap_or(0)
    }

    /// The page `by` places on, wrapping round.
    pub fn step(self, by: i32) -> Self {
        PAGES[(self.index() as i32 + by).rem_euclid(PAGES.len() as i32) as usize]
    }
}

#[derive(Debug, Default, Clone)]
pub struct WeatherView {
    pub place: Option<Place>,
    pub forecast: Option<Forecast>,
    /// When this session last fetched `forecast` (None: loaded from disk).
    pub fetched: Option<Instant>,
    pub busy: bool,
    pub error: Option<String>,
    pub page: Page,
    /// Hours after the current one under the Hourly cursor.
    pub cursor: usize,
    /// Unix time to draw at instead of the clock (previews and tests).
    pub clock: Option<i64>,
}

impl WeatherView {
    pub fn stale(&self, now: Instant) -> bool {
        self.fetched
            .is_none_or(|t| now.saturating_duration_since(t) >= STALE_AFTER)
    }

    /// Unix time now.
    pub fn now(&self) -> i64 {
        self.clock.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        })
    }

    /// Opening the screen starts on Now.
    pub fn open(&mut self) {
        self.page = Page::Now;
        self.cursor = 0;
    }

    /// Next or Previous: another page, the hour cursor back on now.
    pub fn flip(&mut self, by: i32) {
        self.page = self.page.step(by);
        self.cursor = 0;
    }

    /// The wheel `by` detents with `hours` hours ahead: on Hourly the cursor
    /// moves, and past either end the page turns; elsewhere the page turns.
    pub fn turn(&mut self, by: i32, hours: usize) {
        if self.page != Page::Hourly || hours == 0 || by == 0 {
            return self.flip(by.signum());
        }
        let to = self.cursor as i32 + by;
        if to < 0 {
            self.flip(-1);
        } else if to as usize >= hours {
            self.flip(1);
        } else {
            self.cursor = to as usize;
        }
    }

    /// A fetch is due: a city, nothing in flight and an old forecast.
    fn due(&self) -> bool {
        self.place.is_some() && !self.busy && self.stale(Instant::now())
    }
}

/// Hours the Hourly cursor can reach.
const HOURS_AHEAD: usize = 48;

/// The city picker's rows: every country, or the cities of one.
pub(crate) fn city_rows(country: &str) -> Vec<Item> {
    if country.is_empty() {
        return countries()
            .into_iter()
            .map(|name| {
                let n = cities_in(name).len();
                Item::new(name, format!("weather_country:{name}"))
                    .with_secondary(if n == 1 {
                        CITIES[cities_in(name)[0]].0.to_owned()
                    } else {
                        format!("{n} cities")
                    })
                    .with_kind(RowKind::Chevron)
            })
            .collect();
    }
    cities_in(country)
        .into_iter()
        .map(|i| Item::new(CITIES[i].0, format!("weather_city:{i}")))
        .collect()
}

impl Ui {
    /// Rows that open Weather or choose its city.
    pub(crate) fn weather_key(&mut self, m: &mut AppModel, key: &str) -> Option<Effect> {
        if key == "weather" {
            Self::go(m, Screen::Weather, "");
            self.weather.open();
            return Some(if self.weather.due() {
                Effect::WeatherRefresh
            } else {
                Effect::None
            });
        }
        if let Some(country) = key.strip_prefix("weather_country:") {
            Self::go(m, Screen::WeatherCity, country);
            return Some(Effect::None);
        }
        let i = key
            .strip_prefix("weather_city:")?
            .parse::<usize>()
            .ok()
            .filter(|&i| i < CITIES.len())?;
        // Back past the cities and countries to the forecast.
        while m.screen == Screen::WeatherCity {
            Self::back(m);
        }
        if m.screen != Screen::Weather {
            Self::go(m, Screen::Weather, "");
        }
        self.weather.open();
        Some(Effect::WeatherCity(i))
    }

    /// Weather's buttons and wheel. With a forecast, Select and Next or
    /// Previous turn the page (so they are not transport here); holding
    /// Select chooses the city. `None` lets Back, Home and the rest through.
    pub(crate) fn weather_action(&mut self, m: &mut AppModel, action: Action) -> Option<Effect> {
        if m.screen != Screen::Weather
            || m.screen_off
            || m.navigation.modal.is_some()
            || self.text_entry
            || self.pairing.is_some()
            || self.alert.is_some()
        {
            return None;
        }
        let w = &mut self.weather;
        let now = w.now();
        let Some(f) = &w.forecast else {
            return match action {
                Action::Select if w.place.is_some() => Some(if w.busy {
                    Effect::None
                } else {
                    Effect::WeatherRefresh
                }),
                Action::Select | Action::ContextMenu => {
                    Self::go(m, Screen::WeatherCity, "");
                    Some(Effect::None)
                }
                _ => None,
            };
        };
        let hours = f.hours_from(now).len().min(HOURS_AHEAD);
        let detents = |n: u8| i32::from(n.clamp(1, 6));
        match action {
            Action::WheelClockwise(n) => w.turn(detents(n), hours),
            Action::WheelCounterClockwise(n) => w.turn(-detents(n), hours),
            Action::NavigateDown => w.turn(1, hours),
            Action::NavigateUp => w.turn(-1, hours),
            Action::Select | Action::NextTrack | Action::NavigateRight => w.flip(1),
            Action::PreviousTrack | Action::NavigateLeft => w.flip(-1),
            Action::ContextMenu => {
                Self::go(m, Screen::WeatherCity, "");
                return Some(Effect::None);
            }
            _ => return None,
        }
        // Looking through an old forecast fetches a new one.
        Some(if w.due() {
            Effect::WeatherRefresh
        } else {
            Effect::None
        })
    }
}

// ------------------------------------------------------------------ colours

const SUN: u32 = 0xFFC24BFF;
const MOON: u32 = 0xDCE0EAFF;
const BOLT: u32 = 0xFFD84AFF;
const CLOUD: u32 = 0xC6CDD8FF;
const STORM_CLOUD: u32 = 0x7E8796FF;
/// Rain: bars, chances and drops.
const DROP: u32 = 0x6EA0FFFF;
const SNOW: u32 = 0xE6EEFFFF;

/// A colour for a temperature in °C, from violet-blue cold to red hot.
fn temp_color(celsius: f32) -> u32 {
    const STOPS: [(f32, u32); 7] = [
        (-15., 0x8C7CFFFF),
        (-2., 0x5AA8FFFF),
        (6., 0x4FD1D9FF),
        (13., 0x6BDB8CFF),
        (20., 0xF2D04BFF),
        (27., 0xFF9440FF),
        (34., 0xFF5148FF),
    ];
    if celsius <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for pair in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (pair[0], pair[1]);
        if celsius <= b {
            return mix(ca, cb, (celsius - a) / (b - a));
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// The glow behind a condition.
fn condition_tint(kind: Condition, day: bool) -> u32 {
    match kind {
        Condition::Clear | Condition::PartlyCloudy if day => SUN,
        Condition::Clear | Condition::PartlyCloudy => MOON,
        Condition::Drizzle | Condition::Rain => DROP,
        Condition::Storm => BOLT,
        Condition::Snow => SNOW,
        _ => CLOUD,
    }
}

// ------------------------------------------------------------------- values

fn degrees(t: f32) -> String {
    format!("{}°", t.round() as i32)
}

/// °C for colours, whatever the units shown.
fn celsius(f: &Forecast, t: f32) -> f32 {
    if f.imperial {
        (t - 32.) * 5. / 9.
    } else {
        t
    }
}

fn speed_unit(f: &Forecast) -> &'static str {
    if f.imperial {
        "mph"
    } else {
        "km/h"
    }
}

fn speed(f: &Forecast, v: f32) -> String {
    format!("{} {}", v.round() as i32, speed_unit(f))
}

/// Precipitation: "6.1 mm", "0.24 in".
fn amount(f: &Forecast, v: f32) -> String {
    if f.imperial {
        if v < 0.005 {
            "0 in".to_owned()
        } else {
            format!("{v:.2} in")
        }
    } else if v < 0.05 {
        "0 mm".to_owned()
    } else if v >= 10. {
        format!("{v:.0} mm")
    } else {
        format!("{v:.1} mm")
    }
}

/// "Updated 2 h ago" from Unix seconds.
fn age_text(fetched_at: i64, now: i64) -> String {
    let minutes = (now - fetched_at).max(0) / 60;
    match minutes {
        0 => "Updated just now".to_owned(),
        1..=59 => format!("Updated {minutes} min ago"),
        60..=2879 => format!("Updated {} h ago", minutes / 60),
        _ => format!("Updated {} days ago", minutes / 1440),
    }
}

// ----------------------------------------------------------------- drawing

/// Content runs between the page header and the footer.
const TOP: f32 = 72.;
const BOTTOM: f32 = 330.;
const SIDE: f32 = 16.;

/// A raised card.
fn card(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    c.rrect(x, y, w, h, 14., color::GLASS_FAINT);
    c.rrect_outline(x, y, w, h, 14., 1., color::RULE);
}

/// A four-sided shape (top-left, top-right, bottom-left, bottom-right),
/// fading from `top` to `bottom`, drawn as one warped quad.
fn shape(c: &mut Canvas, corners: [[f32; 2]; 4], top: u32, bottom: u32) {
    let [tl, tr, bl, br] = corners;
    let xs = corners.map(|p| p[0]);
    let ys = corners.map(|p| p[1]);
    let (x, y) = (
        xs.into_iter().fold(f32::MAX, f32::min),
        ys.into_iter().fold(f32::MAX, f32::min),
    );
    let (r, b) = (
        xs.into_iter().fold(f32::MIN, f32::max),
        ys.into_iter().fold(f32::MIN, f32::max),
    );
    c.draw.push(Quad {
        warp: Some(Warp {
            corners: [tl, tr, br, bl],
            colors: [top, top, bottom, bottom],
        }),
        ..Quad::rect(x, y, r - x, b - y, top)
    });
}

/// A straight line `width` wide, its edges softened by a fainter halo.
fn line(c: &mut Canvas, a: (f32, f32), b: (f32, f32), width: f32, tint: u32) {
    let halo = alpha(tint, ((tint & 0xFF) / 3) as u8);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.3 {
        return;
    }
    for (w, tint) in [(width + 1.2, halo), (width, tint)] {
        let (nx, ny) = (-dy / len * w / 2., dx / len * w / 2.);
        shape(
            c,
            [
                [a.0 + nx, a.1 + ny],
                [b.0 + nx, b.1 + ny],
                [a.0 - nx, a.1 - ny],
                [b.0 - nx, b.1 - ny],
            ],
            tint,
            tint,
        );
    }
}

/// A rectangle fading from `top` to `bottom`.
fn fall(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, top: u32, bottom: u32) {
    if w > 0. && h > 0. {
        shape(
            c,
            [[x, y], [x + w, y], [x, y + h], [x + w, y + h]],
            top,
            bottom,
        );
    }
}

/// A rectangle fading from `from` at the left to `to` at the right.
fn ramp(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, from: u32, to: u32) {
    if w > 0. && h > 0. {
        c.draw.push(Quad {
            warp: Some(Warp {
                corners: [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
                colors: [from, to, to, from],
            }),
            ..Quad::rect(x, y, w, h, from)
        });
    }
}

/// A pill `h` high fading from `from` at the left to `to` at the right.
fn gradient_pill(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, from: u32, to: u32) {
    let r = h / 2.;
    c.disc(x + r, y + r, r, from);
    c.disc(x + w - r, y + r, r, to);
    ramp(c, x + r, y, w - h, h, from, to);
}

/// An arrow pointing along `degrees` (clockwise from up) through (cx, cy).
fn arrow(c: &mut Canvas, cx: f32, cy: f32, len: f32, degrees: f32, tint: u32) {
    let (s, k) = degrees.to_radians().sin_cos();
    let (dx, dy) = (s, -k);
    let tip = (cx + dx * len / 2., cy + dy * len / 2.);
    let tail = (cx - dx * len / 2., cy - dy * len / 2.);
    let head = len * 0.42;
    let base = (tip.0 - dx * head, tip.1 - dy * head);
    line(c, tail, base, (len * 0.12).max(1.6), tint);
    // The head: a narrow-topped trapezoid that reads as a triangle.
    let (px, py) = (-dy, dx);
    let half = head * 0.6;
    shape(
        c,
        [
            [tip.0 + px * 0.4, tip.1 + py * 0.4],
            [tip.0 - px * 0.4, tip.1 - py * 0.4],
            [base.0 + px * half, base.1 + py * half],
            [base.0 - px * half, base.1 - py * half],
        ],
        tint,
        tint,
    );
}

/// A card's small label.
fn label(c: &mut Canvas, x: f32, cy: f32, width: f32, s: &str) {
    c.text_mid(x, cy, width, s, Face::R11, color::MUTED);
}

/// The first of `faces` `s` fits `width` in (the last when none does).
fn fitting(faces: &[Face], s: &str, width: f32) -> Face {
    faces
        .iter()
        .copied()
        .find(|&f| text::width(f, s) <= width)
        .unwrap_or(faces[faces.len() - 1])
}

/// A label then a value on one line; returns the width drawn.
fn pair(c: &mut Canvas, x: f32, cy: f32, label: &str, value: &str, face: Face) -> f32 {
    let lw = text::width(face, label);
    c.text_mid(x, cy, lw + 1., label, face, color::DIM);
    let vw = text::width(face, value);
    c.text_mid(x + lw + 5., cy, vw + 1., value, face, color::TEXT);
    lw + 5. + vw
}

/// The screen below the status bar (which the caller draws).
pub(crate) fn draw(c: &mut Canvas, w: &WeatherView, accent: u32) {
    let now = w.now();
    match (&w.place, &w.forecast) {
        (None, _) => {
            let head = if w.busy {
                "Finding the city…"
            } else {
                "Choose a city"
            };
            c.glow(WIDTH / 2., 150., 140., 70., alpha(accent, 0x22));
            icon(c, Condition::PartlyCloudy, true, WIDTH / 2. - 36., 70., 72.);
            c.text_mid_center(WIDTH / 2., 172., head, Face::S20, color::TEXT);
            c.text_mid_center(
                WIDTH / 2.,
                202.,
                "Select to pick a country and city",
                Face::R15,
                color::MUTED,
            );
            c.text_mid_center(
                WIDTH / 2.,
                228.,
                "Uses Wi-Fi · data from Open-Meteo",
                Face::R11,
                color::DIM,
            );
            status(c, w, now);
        }
        (Some(p), None) => {
            let room = WIDTH - 2. * SIDE;
            let name = text::fit(Face::S20, &p.name, room);
            c.text_mid_center(WIDTH / 2., 150., &name, Face::S20, color::TEXT);
            let region = text::fit(Face::R13, &p.region, room);
            c.text_mid_center(WIDTH / 2., 176., &region, Face::R13, color::MUTED);
            let head = if w.busy {
                "Fetching the forecast…"
            } else {
                "Select to fetch the forecast"
            };
            c.text_mid_center(WIDTH / 2., 210., head, Face::S15, color::TEXT);
            status(c, w, now);
        }
        (Some(p), Some(f)) => {
            header(c, w.page, accent);
            match w.page {
                Page::Now => now_page(c, f, accent, now),
                Page::Hourly => charts::hourly(c, f, w.cursor, accent, now),
                Page::Rain => charts::rain(c, f, now),
                Page::Wind => charts::wind(c, f, accent, now),
                Page::Days => sky::days(c, f, now),
                Page::Sky => sky::sun_and_moon(c, f, p, now),
                Page::Air => sky::air(c, f, now),
                Page::Details => sky::details(c, f, accent, now),
            }
            footer(c, w, p, now);
        }
    }
}

/// The page's name and a dot for each page.
fn header(c: &mut Canvas, page: Page, accent: u32) {
    c.text_mid(SIDE, 54., 200., page.title(), Face::S20, color::TEXT);
    let n = PAGES.len() as f32;
    let (gap, active) = (12., 18.);
    let right = WIDTH - SIDE;
    let mut x = right - (n - 1.) * gap - active + 6.;
    for p in PAGES {
        if p == page {
            c.rrect(x - 3., 51., active, 6., 3., accent);
            x += active - 6. + gap;
        } else {
            c.disc(x, 54., 3., color::TRACK);
            x += gap;
        }
    }
}

/// How fresh the forecast is, and the place.
fn footer(c: &mut Canvas, w: &WeatherView, p: &Place, now: i64) {
    let (line, tint) = status_line(w, now);
    let place = text::fit(Face::R11, &p.name, 150.);
    let pw = text::width(Face::R11, &place);
    c.text_mid(
        SIDE,
        344.,
        WIDTH - 2. * SIDE - pw - 16.,
        &line,
        Face::R11,
        tint,
    );
    c.text_mid_right(WIDTH - SIDE, 344., &place, Face::R11, color::DIM);
}

fn status_line(w: &WeatherView, now: i64) -> (String, u32) {
    if let Some(error) = &w.error {
        let age = w
            .forecast
            .as_ref()
            .map(|f| format!(" · {}", age_text(f.fetched_at, now).to_lowercase()))
            .unwrap_or_default();
        return (format!("{error}{age}"), color::DANGER);
    }
    if w.busy && w.forecast.is_some() {
        return ("Updating…".to_owned(), color::MUTED);
    }
    match &w.forecast {
        // Open-Meteo's data is used under its attribution licence.
        Some(f) => (
            format!("{} · Open-Meteo", age_text(f.fetched_at, now)),
            color::DIM,
        ),
        None => (String::new(), color::DIM),
    }
}

/// The status line for the screens before a forecast.
fn status(c: &mut Canvas, w: &WeatherView, now: i64) {
    let (line, tint) = status_line(w, now);
    c.text_mid(SIDE, 316., WIDTH - 2. * SIDE, &line, Face::R11, tint);
    if w.place.is_some() {
        c.text_mid(
            SIDE,
            338.,
            WIDTH - 2. * SIDE,
            "Hold Select to change city",
            Face::R11,
            color::DIM,
        );
    }
}

/// Big temperature, the condition, feels like, high and low, one line on
/// what is coming and four measures.
fn now_page(c: &mut Canvas, f: &Forecast, accent: u32, now: i64) {
    let h = f.conditions(now);
    let today = f.today(now).map(|i| &f.days[i]);
    c.glow(
        74.,
        124.,
        100.,
        78.,
        alpha(condition_tint(h.condition, h.is_day), 0x2E),
    );
    icon(c, h.condition, h.is_day, 22., 76., 104.);
    let temp = degrees(h.temperature);
    c.text_mid(140., 118., 150., &temp, Face::L44, color::TEXT);
    let x = 140. + text::width(Face::L44, &temp) + 20.;
    let room = WIDTH - SIDE - x;
    c.text_mid(x, 94., room, h.condition.label(), Face::S17, color::TEXT);
    c.text_mid(
        x,
        118.,
        room,
        &format!("Feels like {}", degrees(h.feels_like)),
        Face::R13,
        color::MUTED,
    );
    if let Some(d) = today {
        let x = x + pair(c, x, 141., "H", &degrees(d.high), Face::R13) + 12.;
        pair(c, x, 141., "L", &degrees(d.low), Face::R13);
    }
    // What is coming.
    let (y, hgt) = (184., 44.);
    c.rrect(SIDE, y, WIDTH - 2. * SIDE, hgt, 14., alpha(accent, 0x24));
    c.rrect(SIDE + 14., y + 14., 4., hgt - 28., 2., accent);
    c.text_mid(
        SIDE + 28.,
        y + hgt / 2.,
        WIDTH - 2. * SIDE - 40.,
        &f.summary(now),
        Face::S15,
        color::TEXT,
    );
    // Four measures: label, value, its unit, a note.
    let chips = [
        (
            "Wind",
            format!("{}", h.wind.round() as i32),
            speed_unit(f),
            format!("{}, gusts {}", compass(h.wind_direction), h.gusts.round()),
        ),
        (
            "Humidity",
            format!("{}%", h.humidity),
            "",
            format!("Dew point {}", degrees(h.dew_point)),
        ),
        (
            "Rain today",
            today
                .and_then(|d| d.rain_chance)
                .map_or("-".to_owned(), |r| format!("{r}%")),
            "",
            today.map_or(String::new(), |d| amount(f, d.precipitation)),
        ),
        (
            "UV index",
            format!("{}", h.uv.round() as i32),
            "",
            match today {
                Some(d) if h.uv < 0.5 && d.uv_max >= 0.5 => {
                    format!("Max {} today", d.uv_max.round() as i32)
                }
                _ => uv_category(h.uv).to_owned(),
            },
        ),
    ];
    let (top, gap) = (240., 8.);
    let w = (WIDTH - 2. * SIDE - 3. * gap) / 4.;
    for (i, (name, value, unit, sub)) in chips.iter().enumerate() {
        let x = SIDE + i as f32 * (w + gap);
        card(c, x, top, w, BOTTOM - top);
        label(c, x + 12., top + 18., w - 20., name);
        let room = w - 24. - text::width(Face::R13, unit);
        let face = fitting(&[Face::S20, Face::S17, Face::S15], value, room);
        c.text_mid(x + 12., top + 45., room, value, face, color::TEXT);
        if !unit.is_empty() {
            let vx = x + 12. + text::width(face, value) + 4.;
            c.text_mid(vx, top + 48., w, unit, Face::R13, color::DIM);
        }
        c.text_mid(x + 12., top + 71., w - 20., sub, Face::R11, color::DIM);
    }
}

/// Text for pages whose part of the forecast has run out.
fn no_hours(c: &mut Canvas) {
    c.text_mid_center(
        WIDTH / 2.,
        190.,
        "This part of the forecast comes with the next update",
        Face::R13,
        color::MUTED,
    );
}

/// A cloud filling the box (x, y, s, s * 0.6).
fn cloud(c: &mut Canvas, x: f32, y: f32, s: f32, tint: u32) {
    c.disc(x + s * 0.30, y + s * 0.38, s * 0.18, tint);
    c.disc(x + s * 0.55, y + s * 0.23, s * 0.23, tint);
    c.disc(x + s * 0.74, y + s * 0.32, s * 0.18, tint);
    c.rrect(
        x + s * 0.06,
        y + s * 0.30,
        s * 0.88,
        s * 0.28,
        s * 0.14,
        tint,
    );
}

/// A drawn weather symbol in an s x s box.
fn icon(c: &mut Canvas, kind: Condition, day: bool, x: f32, y: f32, s: f32) {
    let sky = |c: &mut Canvas, cx: f32, cy: f32, r: f32| {
        let tint = if day { SUN } else { MOON };
        c.glow(cx, cy, r * 1.6, r * 1.6, alpha(tint, 0x50));
        c.disc(cx, cy, r, tint);
        if !day {
            // A crescent: the night sky bites the moon.
            c.disc(cx + r * 0.45, cy - r * 0.35, r * 0.8, color::BG);
        }
    };
    match kind {
        Condition::Clear => {
            if day {
                // Rays.
                let (cx, cy, r) = (x + s * 0.5, y + s * 0.5, s * 0.3);
                for i in 0..8 {
                    let (sn, cs) = (i as f32 * 45.).to_radians().sin_cos();
                    let (r0, r1) = (r * 1.32, r * 1.62);
                    line(
                        c,
                        (cx + sn * r0, cy - cs * r0),
                        (cx + sn * r1, cy - cs * r1),
                        (s * 0.05).max(1.5),
                        alpha(SUN, 0xC0),
                    );
                }
                sky(c, cx, cy, r);
            } else {
                sky(c, x + s * 0.5, y + s * 0.5, s * 0.32);
            }
        }
        Condition::PartlyCloudy => {
            sky(c, x + s * 0.6, y + s * 0.33, s * 0.25);
            cloud(c, x, y + s * 0.34, s * 0.82, CLOUD);
        }
        Condition::Cloudy => {
            cloud(c, x + s * 0.2, y + s * 0.12, s * 0.7, STORM_CLOUD);
            cloud(c, x, y + s * 0.3, s * 0.86, CLOUD);
        }
        Condition::Fog => {
            for (i, inset) in [0.1, 0., 0.18].into_iter().enumerate() {
                c.rrect(
                    x + s * inset,
                    y + s * (0.26 + 0.18 * i as f32),
                    s * (0.9 - inset),
                    s * 0.1,
                    s * 0.05,
                    CLOUD,
                );
            }
        }
        Condition::Drizzle | Condition::Rain | Condition::Snow | Condition::Storm => {
            let tint = if kind == Condition::Storm {
                STORM_CLOUD
            } else {
                CLOUD
            };
            cloud(c, x + s * 0.04, y + s * 0.1, s * 0.9, tint);
            let below = y + s * 0.7;
            match kind {
                Condition::Snow => {
                    for i in 0..3 {
                        c.disc(
                            x + s * (0.26 + 0.24 * i as f32),
                            below + s * 0.06 + (i % 2) as f32 * s * 0.1,
                            s * 0.06,
                            SNOW,
                        );
                    }
                }
                Condition::Storm => {
                    let (bx, by) = (x + s * 0.5, below - s * 0.06);
                    shape(
                        c,
                        [
                            [bx + s * 0.02, by],
                            [bx + s * 0.14, by],
                            [bx - s * 0.08, by + s * 0.17],
                            [bx + s * 0.04, by + s * 0.17],
                        ],
                        BOLT,
                        BOLT,
                    );
                    shape(
                        c,
                        [
                            [bx - s * 0.06, by + s * 0.15],
                            [bx + s * 0.08, by + s * 0.15],
                            [bx - s * 0.12, by + s * 0.32],
                            [bx - s * 0.11, by + s * 0.32],
                        ],
                        BOLT,
                        BOLT,
                    );
                }
                _ => {
                    let drops = if kind == Condition::Rain { 3 } else { 2 };
                    for i in 0..drops {
                        let dx = x + s * (0.28 + 0.2 * i as f32);
                        line(
                            c,
                            (dx + s * 0.05, below),
                            (dx - s * 0.02, below + s * 0.18),
                            (s * 0.06).max(1.5),
                            DROP,
                        );
                    }
                }
            }
        }
    }
}

/// The first hour shown on Hourly, keeping the cursor in view.
fn window(len: usize, cursor: usize, span: usize) -> usize {
    if len <= span {
        return 0;
    }
    cursor.saturating_sub(span / 3).min(len - span)
}

/// Draw `h`'s condition icon for a chart column centred on (cx, cy).
fn hour_icon(c: &mut Canvas, h: &Hour, cx: f32, cy: f32, s: f32) {
    icon(c, h.condition, h.is_day, cx - s / 2., cy - s / 2., s);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 13 October 2026, 14:20 at a place an hour east of UTC.
    pub(super) const NOW: i64 = 1_791_897_600;

    fn hour(i: i64) -> Hour {
        let t = NOW - 3 * 3600 + i * 3600;
        let local = (i + 11) % 24;
        let day = (7..19).contains(&local);
        let wet = (7..12).contains(&i);
        Hour {
            time: t - t.rem_euclid(3600),
            temperature: 12. + 5. * ((local as f32 - 9.) / 24. * std::f32::consts::TAU).sin(),
            feels_like: 10.,
            rain_chance: if wet { 80 } else { 10 },
            precipitation: if wet { 1.6 } else { 0. },
            condition: if wet {
                Condition::Rain
            } else if i % 5 == 0 {
                Condition::Clear
            } else {
                Condition::PartlyCloudy
            },
            wind: 14. + (i % 7) as f32,
            wind_direction: (250 + 3 * i as u16) % 360,
            gusts: 26. + (i % 5) as f32,
            humidity: 70,
            uv: if day { 2. } else { 0. },
            cloud_cover: 40,
            visibility: 24.,
            is_day: day,
            pressure: 1013. - 0.3 * i as f32,
            dew_point: 8.,
        }
    }

    /// A forecast of invented, ordinary weather, read at `NOW`.
    pub(super) fn sample() -> Forecast {
        let midnight = NOW - (14 * 3600 + 20 * 60);
        let days = (0..10)
            .map(|i| reborn_core::weather::Day {
                date: format!("2026-10-{:02}", 13 + i),
                high: 17. - i as f32 * 0.5,
                low: 9. - (i % 3) as f32,
                condition: [Condition::Rain, Condition::Cloudy, Condition::Clear][i % 3],
                rain_chance: Some(20 + 10 * (i % 4) as u8),
                sunrise: midnight + i as i64 * 86_400 + 7 * 3600 + 20 * 60,
                sunset: midnight + i as i64 * 86_400 + 18 * 3600,
                precipitation: if i % 3 == 0 { 6.4 } else { 0. },
                uv_max: 3.,
                wind_max: 24.,
                gust_max: 45.,
                wind_direction: 240,
                daylight: 38_400. - 220. * i as f32,
            })
            .collect();
        let hours: Vec<Hour> = (0..51).map(hour).collect();
        Forecast {
            current: Hour {
                time: NOW,
                ..hours[3].clone()
            },
            hours,
            days,
            imperial: false,
            timezone: "Europe/London".into(),
            utc_offset: 3600,
            fetched_at: NOW - 240,
            yesterday_daylight: Some(38_620.),
            air: Some(reborn_core::weather::Air {
                us_aqi: Some(42),
                pm2_5: Some(8.5),
                pm10: Some(14.),
                ozone: Some(61.),
                nitrogen_dioxide: Some(17.),
                pollen: vec![reborn_core::weather::Pollen {
                    name: "Grass".into(),
                    grains: 3.,
                }],
            }),
        }
    }

    fn view() -> WeatherView {
        WeatherView {
            place: Some(Place {
                name: "Llanfairpwllgwyngyllgogerychwyrndrobwll".into(),
                region: "Wales, United Kingdom".into(),
                country_code: "GB".into(),
                latitude: 53.2,
                longitude: -4.2,
            }),
            forecast: Some(sample()),
            fetched: Some(Instant::now()),
            clock: Some(NOW),
            ..Default::default()
        }
    }

    #[test]
    fn weather_draws_inside_the_display_on_every_page_and_state() {
        let full = view();
        let mut states = vec![
            WeatherView {
                clock: Some(NOW),
                ..Default::default()
            },
            full.clone(),
        ];
        let mut no_forecast = full.clone();
        no_forecast.forecast = None;
        no_forecast.busy = true;
        states.push(no_forecast);
        let mut failed = full.clone();
        failed.error = Some("No internet connection".into());
        states.push(failed);
        let mut imperial = full.clone();
        if let Some(f) = imperial.forecast.as_mut() {
            f.imperial = true;
            f.timezone = "America/Chicago".into();
            f.air = None;
        }
        states.push(imperial);
        for w in &states {
            for page in PAGES {
                // Now, the cursor at either end, tomorrow, and long after the
                // forecast ran out.
                for (cursor, later) in [(0, 0), (47, 0), (5, 30 * 3600), (0, 90 * 3600)] {
                    let mut w = w.clone();
                    w.page = page;
                    w.cursor = cursor;
                    w.clock = Some(NOW + later);
                    let mut c = Canvas::new();
                    draw(&mut c, &w, color::ACCENT);
                    for q in c.finish() {
                        assert!(q.x >= -32. && q.y >= -32., "{page:?} {later} {q:?}");
                        assert!(
                            q.x + q.w <= 512. && q.y + q.h <= 392.,
                            "{page:?} {later} {q:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn forecasts_go_stale_after_fifteen_minutes() {
        let now = Instant::now();
        let mut w = WeatherView::default();
        assert!(w.stale(now));
        w.fetched = Some(now);
        assert!(!w.stale(now + Duration::from_secs(60)));
        assert!(w.stale(now + STALE_AFTER));
    }

    #[test]
    fn pages_turn_and_the_hour_cursor_spills_over() {
        let mut w = WeatherView::default();
        w.flip(-1);
        assert_eq!(w.page, Page::Details);
        w.flip(1);
        w.turn(1, 48);
        assert_eq!(w.page, Page::Hourly);
        // On Hourly the wheel moves the cursor...
        w.turn(5, 48);
        w.turn(3, 48);
        assert_eq!((w.page, w.cursor), (Page::Hourly, 8));
        w.turn(-2, 48);
        assert_eq!(w.cursor, 6);
        // ...and past the last hour goes on to Rain, cursor back on now.
        w.turn(41, 48);
        assert_eq!((w.page, w.cursor), (Page::Hourly, 47));
        w.turn(1, 48);
        assert_eq!((w.page, w.cursor), (Page::Rain, 0));
        w.turn(-1, 48);
        w.turn(-1, 48);
        assert_eq!(w.page, Page::Now);
        // Without hours the wheel only turns pages.
        w.flip(1);
        w.turn(3, 0);
        assert_eq!(w.page, Page::Rain);
        w.cursor = 9;
        w.open();
        assert_eq!((w.page, w.cursor), (Page::Now, 0));
    }

    #[test]
    fn the_hourly_window_keeps_the_cursor_in_view() {
        assert_eq!(window(48, 0, 24), 0);
        assert_eq!(window(48, 8, 24), 0);
        assert_eq!(window(48, 20, 24), 12);
        assert_eq!(window(48, 47, 24), 24);
        assert_eq!(window(10, 9, 24), 0);
        for cursor in 0..48 {
            let start = window(48, cursor, 24);
            assert!((start..start + 24).contains(&cursor));
        }
    }

    #[test]
    fn values_read_plainly() {
        assert_eq!(age_text(1000, 1000 + 59), "Updated just now");
        assert_eq!(age_text(1000, 1000 + 12 * 60), "Updated 12 min ago");
        assert_eq!(age_text(1000, 1000 + 2 * 3600 + 900), "Updated 2 h ago");
        assert_eq!(age_text(1000, 1000 + 3 * 86_400), "Updated 3 days ago");
        assert_eq!(temp_color(-40.), 0x8C7CFFFF);
        assert_eq!(temp_color(20.), 0xF2D04BFF);
        assert_eq!(temp_color(60.), 0xFF5148FF);
        assert_eq!(Page::Now.step(-1), Page::Details);
        assert_eq!(Page::Details.step(2), Page::Hourly);
        let f = sample();
        assert_eq!(amount(&f, 0.01), "0 mm");
        assert_eq!(amount(&f, 6.14), "6.1 mm");
        assert_eq!(speed(&f, 14.4), "14 km/h");
    }

    #[test]
    fn select_and_next_turn_pages_and_hold_select_chooses_the_city() {
        let mut ui = Ui {
            weather: view(),
            ..Default::default()
        };
        let mut m = AppModel {
            screen: Screen::Weather,
            ..Default::default()
        };
        assert_eq!(ui.model_action(&mut m, Action::Select), Effect::None);
        assert_eq!(ui.weather.page, Page::Hourly);
        assert_eq!(ui.model_action(&mut m, Action::NextTrack), Effect::None);
        assert_eq!(ui.weather.page, Page::Rain);
        ui.model_action(&mut m, Action::PreviousTrack);
        ui.model_action(&mut m, Action::WheelClockwise(2));
        assert_eq!((ui.weather.page, ui.weather.cursor), (Page::Hourly, 2));
        // An old forecast is refreshed as the pages turn.
        ui.weather.fetched = None;
        assert_eq!(
            ui.model_action(&mut m, Action::NextTrack),
            Effect::WeatherRefresh
        );
        ui.weather.busy = true;
        assert_eq!(ui.model_action(&mut m, Action::NextTrack), Effect::None);
        ui.model_action(&mut m, Action::ContextMenu);
        assert_eq!(
            (m.screen, m.navigation.filter.as_str()),
            (Screen::WeatherCity, "")
        );
    }

    #[test]
    fn a_city_is_chosen_country_first_and_the_forecast_follows() {
        let mut ui = Ui::default();
        let mut m = AppModel::default();
        // Home > Extras > Weather without a city: Select opens the countries.
        let home = ui.rows(&m, &[]);
        m.navigation.focus = home.iter().position(|r| r.key == "extras").unwrap();
        ui.model_action(&mut m, Action::Select);
        let extras = ui.rows(&m, &[]);
        m.navigation.focus = extras.iter().position(|r| r.key == "weather").unwrap();
        assert_eq!(ui.model_action(&mut m, Action::Select), Effect::None);
        assert_eq!(m.screen, Screen::Weather);
        ui.model_action(&mut m, Action::Select);
        assert_eq!(m.screen, Screen::WeatherCity);
        let countries = ui.rows(&m, &[]);
        assert_eq!(countries.len(), reborn_core::cities::countries().len());
        m.navigation.focus = countries.iter().position(|r| r.label == "France").unwrap();
        ui.model_action(&mut m, Action::Select);
        assert_eq!(m.navigation.filter, "France");
        let cities = ui.rows(&m, &[]);
        assert_eq!(cities[0].label, "Paris");
        let paris = CITIES.iter().position(|c| c.0 == "Paris").unwrap();
        assert_eq!(
            ui.model_action(&mut m, Action::Select),
            Effect::WeatherCity(paris)
        );
        // Back on the forecast screen; Back from there goes to Extras.
        assert_eq!(m.screen, Screen::Weather);
        ui.model_action(&mut m, Action::Back);
        assert_eq!(m.screen, Screen::Extras);
    }
}

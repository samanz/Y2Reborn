//! Weather data shared by the platform fetcher, the app and the UI, and the
//! plain derivations the Weather pages show: what the next hours bring in one
//! sentence, the moon's phase, the change in daylight and the categories of
//! the UV index and air quality.
use serde::{Deserialize, Serialize};

/// A city found by Open-Meteo's geocoding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    /// Region and country, e.g. "England, United Kingdom".
    pub region: String,
    pub country_code: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Place {
    /// Fahrenheit and mph where those are the everyday units.
    pub fn imperial(&self) -> bool {
        matches!(self.country_code.as_str(), "US" | "LR" | "MM")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    /// The conditions when fetched.
    pub current: Hour,
    /// Hour by hour from three hours before the fetch to two days after.
    pub hours: Vec<Hour>,
    /// Today first (yesterday is only kept as `yesterday_daylight`).
    pub days: Vec<Day>,
    /// °F, mph and inches rather than °C, km/h and mm (visibility in miles
    /// rather than km).
    pub imperial: bool,
    /// The place's tz database zone, e.g. "Europe/Paris".
    pub timezone: String,
    /// The place's offset from UTC when fetched, seconds.
    pub utc_offset: i32,
    /// When the forecast was fetched (Unix seconds).
    pub fetched_at: i64,
    /// Yesterday's daylight in seconds, for today's change.
    pub yesterday_daylight: Option<f32>,
    pub air: Option<Air>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// ISO date, "YYYY-MM-DD".
    pub date: String,
    pub high: f32,
    pub low: f32,
    pub condition: Condition,
    pub rain_chance: Option<u8>,
    /// Unix seconds (0 when unknown, as at the poles).
    pub sunrise: i64,
    pub sunset: i64,
    /// Total precipitation, mm or inches.
    pub precipitation: f32,
    pub uv_max: f32,
    pub wind_max: f32,
    pub gust_max: f32,
    /// Dominant wind direction, degrees the wind comes from.
    pub wind_direction: u16,
    /// Seconds of daylight.
    pub daylight: f32,
}

/// One hour of the forecast (or the current conditions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hour {
    /// Unix seconds at the start of the hour (the observation time for the
    /// current conditions).
    pub time: i64,
    pub temperature: f32,
    pub feels_like: f32,
    pub rain_chance: u8,
    /// mm or inches in the hour.
    pub precipitation: f32,
    pub condition: Condition,
    pub wind: f32,
    /// Degrees the wind comes from.
    pub wind_direction: u16,
    pub gusts: f32,
    pub humidity: u8,
    pub uv: f32,
    pub cloud_cover: u8,
    /// km, or miles when imperial.
    pub visibility: f32,
    pub is_day: bool,
    /// Sea-level pressure, hPa.
    pub pressure: f32,
    pub dew_point: f32,
}

/// Air quality from Open-Meteo's air quality service.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Air {
    pub us_aqi: Option<u16>,
    /// Micrograms per cubic metre.
    pub pm2_5: Option<f32>,
    pub pm10: Option<f32>,
    pub ozone: Option<f32>,
    pub nitrogen_dioxide: Option<f32>,
    /// Pollen where it is measured (Europe), grains per cubic metre.
    pub pollen: Vec<Pollen>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pollen {
    /// "Grass", "Birch", ...
    pub name: String,
    pub grains: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Storm,
}

impl Condition {
    /// From a WMO weather interpretation code.
    pub fn from_wmo(code: u32) -> Self {
        match code {
            0 | 1 => Self::Clear,
            2 => Self::PartlyCloudy,
            3 => Self::Cloudy,
            45 | 48 => Self::Fog,
            51..=57 => Self::Drizzle,
            61..=67 | 80..=82 => Self::Rain,
            71..=77 | 85 | 86 => Self::Snow,
            95..=99 => Self::Storm,
            _ => Self::Cloudy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::PartlyCloudy => "Partly cloudy",
            Self::Cloudy => "Cloudy",
            Self::Fog => "Fog",
            Self::Drizzle => "Drizzle",
            Self::Rain => "Rain",
            Self::Snow => "Snow",
            Self::Storm => "Thunderstorm",
        }
    }

    /// Something falls from the sky.
    pub fn wet(self) -> bool {
        matches!(self, Self::Drizzle | Self::Rain | Self::Snow | Self::Storm)
    }
}

/// A moment in a place's local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    /// 0 = Sunday.
    pub weekday: u8,
    /// Days since 1970-01-01 in local time; equal means the same date.
    pub days: i64,
}

/// Calendar fields of a count of seconds since 1970 (no zone applied).
pub fn civil(seconds: i64) -> Local {
    // Howard Hinnant's civil-from-days.
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    Local {
        year: (yoe + era * 400 + i64::from(month <= 2)) as i32,
        month,
        day,
        hour: (rest / 3600) as u8,
        minute: (rest % 3600 / 60) as u8,
        weekday: (days + 4).rem_euclid(7) as u8,
        days,
    }
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The ISO date of a local time, "YYYY-MM-DD".
pub fn iso_date(t: &Local) -> String {
    format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
}

/// "Fri".
pub fn day_name(t: &Local) -> &'static str {
    WEEKDAYS[usize::from(t.weekday % 7)]
}

/// "Fri 17 Oct".
pub fn date_label(t: &Local) -> String {
    format!(
        "{} {} {}",
        day_name(t),
        t.day,
        MONTHS[usize::from(t.month.clamp(1, 12) - 1)]
    )
}

/// Short weekday name for an ISO date ("2026-10-11" -> "Sun").
pub fn weekday(date: &str) -> &'static str {
    let mut parts = date.splitn(3, '-').map(|p| p.parse::<i64>().unwrap_or(0));
    let (y, m, d) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return "";
    }
    // Days since 1970-01-01 (days-from-civil), then 1970-01-01 was a Thursday.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize]
}

/// Zones whose people mostly read a 12-hour clock.
fn twelve_hour_zone(zone: &str) -> bool {
    const TWELVE: &[&str] = &[
        "America/",
        "Pacific/Honolulu",
        "Asia/Kolkata",
        "Asia/Manila",
        "Asia/Karachi",
        "Asia/Dhaka",
        "Australia/",
        "Pacific/Auckland",
        "Africa/Cairo",
    ];
    const LATIN: &[&str] = &[
        "America/Sao_Paulo",
        "America/Argentina",
        "America/Bogota",
        "America/Lima",
        "America/Santiago",
        "America/Caracas",
        "America/Montevideo",
        "America/Asuncion",
        "America/La_Paz",
        "America/Havana",
        "America/Cayenne",
        "America/Paramaribo",
    ];
    TWELVE.iter().any(|p| zone.starts_with(p)) && !LATIN.iter().any(|p| zone.starts_with(p))
}

impl Forecast {
    /// Local time at the place. The offset is the one at fetch time; a
    /// daylight-saving change inside the two days shown moves labels by an
    /// hour, which is accepted to keep a zone database off the player.
    pub fn local(&self, utc: i64) -> Local {
        civil(utc + i64::from(self.utc_offset))
    }

    /// The place reads a 12-hour clock.
    pub fn twelve_hour(&self) -> bool {
        twelve_hour_zone(&self.timezone)
    }

    /// An hour on the clock: "3 pm" or "15:00".
    pub fn hour_label(&self, utc: i64) -> String {
        hour_text(self.local(utc).hour, self.twelve_hour())
    }

    /// A time of day: "6:42 am" or "06:42".
    pub fn time_label(&self, utc: i64) -> String {
        let t = self.local(utc);
        if self.twelve_hour() {
            let h = match t.hour % 12 {
                0 => 12,
                h => h,
            };
            format!(
                "{h}:{:02} {}",
                t.minute,
                if t.hour < 12 { "am" } else { "pm" }
            )
        } else {
            format!("{:02}:{:02}", t.hour, t.minute)
        }
    }

    /// The hour that contains `now`.
    pub fn hour_index(&self, now: i64) -> Option<usize> {
        self.hours
            .iter()
            .position(|h| h.time <= now && now < h.time + 3600)
    }

    /// The hours from the one containing `now` on (all of them when `now`
    /// is before the first; none when the forecast has run out).
    pub fn hours_from(&self, now: i64) -> &[Hour] {
        match self.hour_index(now) {
            Some(i) => &self.hours[i..],
            None if self.hours.first().is_some_and(|h| now < h.time) => &self.hours,
            None => &[],
        }
    }

    /// Conditions at `now`: the measured ones while they are recent, else
    /// the forecast for this hour (a saved forecast read hours later), else
    /// the last measured ones.
    pub fn conditions(&self, now: i64) -> &Hour {
        if (now - self.current.time).abs() < 90 * 60 {
            return &self.current;
        }
        self.hour_index(now)
            .map_or(&self.current, |i| &self.hours[i])
    }

    /// Today's entry in `days` (the local date at the place).
    pub fn today(&self, now: i64) -> Option<usize> {
        let date = iso_date(&self.local(now));
        self.days.iter().position(|d| d.date == date)
    }

    /// The days from today on (all of them for a forecast read early).
    pub fn days_from(&self, now: i64) -> &[Day] {
        match self.today(now) {
            Some(i) => &self.days[i..],
            None if self
                .days
                .first()
                .is_some_and(|d| d.date > iso_date(&self.local(now))) =>
            {
                &self.days
            }
            None => &[],
        }
    }

    /// Change in sea-level pressure over the last three hours, hPa.
    pub fn pressure_trend(&self, now: i64) -> Option<f32> {
        let at = |t: i64| {
            self.hours
                .iter()
                .find(|h| h.time <= t && t < h.time + 3600)
                .map(|h| h.pressure)
                .filter(|p| *p > 0.)
        };
        Some(at(now)? - at(now - 3 * 3600)?)
    }

    /// Today's daylight against the day before's, seconds (positive when
    /// longer).
    pub fn daylight_change(&self, now: i64) -> Option<f32> {
        let i = self.today(now)?;
        let today = self.days[i].daylight;
        let before = match i {
            0 => self.yesterday_daylight?,
            _ => self.days[i - 1].daylight,
        };
        (today > 0. && before > 0.).then_some(today - before)
    }

    /// One line on what the next day brings, e.g. "Rain from 3 pm, clearing
    /// tonight" or "Clear skies, cooling to 3° by 6 am".
    pub fn summary(&self, now: i64) -> String {
        // Rain is looked for in the next day; its end in all that is known.
        let ahead = self.hours_from(now);
        let n = ahead.len().min(25);
        if n < 2 {
            return self.conditions(now).condition.label().to_owned();
        }
        let current = &self.current;
        let wet_at = |i: usize| {
            let h = &ahead[i];
            let threshold = if self.imperial { 0.004 } else { 0.1 };
            let now_wet = i == 0 && current.condition.wet() && current.precipitation > 0.;
            now_wet || (h.rain_chance >= 40 && (h.condition.wet() || h.precipitation >= threshold))
        };
        let kind = |from: usize, to: usize| {
            let run = &ahead[from..to.max(from + 1).min(ahead.len())];
            let has = |c: Condition| run.iter().any(|h| h.condition == c);
            if has(Condition::Storm) {
                "Storms"
            } else if has(Condition::Snow) {
                "Snow"
            } else if run.iter().all(|h| h.condition == Condition::Drizzle) {
                "Drizzle"
            } else {
                "Rain"
            }
        };
        let today = self.local(now).days;
        let period = |t: i64| {
            let l = self.local(t);
            let tomorrow = l.days > today;
            match (tomorrow, l.hour) {
                (false, 5..=11) => "this morning",
                (false, 12..=16) => "this afternoon",
                (false, 17..=20) => "this evening",
                (false, _) => "tonight",
                (true, 0..=4) => "overnight",
                (true, 5..=11) => "tomorrow morning",
                (true, 12..=16) => "tomorrow afternoon",
                (true, 17..=20) => "tomorrow evening",
                (true, _) => "tomorrow night",
            }
        };
        // "3 pm", "midnight" or "noon".
        let spoken = |t: i64| match self.local(t).hour {
            0 => "midnight".to_owned(),
            12 if self.twelve_hour() => "noon".to_owned(),
            _ => self.hour_label(t),
        };
        // ...or "7 am tomorrow" when that is not just overnight.
        let when = |t: i64| {
            if self.local(t).days > today && t - now > 12 * 3600 {
                format!("{} tomorrow", spoken(t))
            } else {
                spoken(t)
            }
        };
        let all = ahead.len();
        if wet_at(0) {
            let end = (0..all).find(|&i| !wet_at(i));
            let what = kind(0, end.unwrap_or(all));
            return match end {
                None => format!("{what} for the next day"),
                Some(end) => {
                    let after = ahead[end].condition;
                    let sky = if after == Condition::Clear {
                        "clearing".to_owned()
                    } else {
                        after.label().to_lowercase()
                    };
                    format!("{what} until {}, then {sky}", when(ahead[end].time))
                }
            };
        }
        if let Some(start) = (0..n).find(|&i| wet_at(i)) {
            let end = (start..all).find(|&i| !wet_at(i));
            let what = kind(start, end.unwrap_or(all));
            let when = when(ahead[start].time);
            return match end {
                Some(end) => format!("{what} from {when}, clearing {}", period(ahead[end].time)),
                None => format!("{what} from {when}"),
            };
        }
        // Dry: the sky, then wind or the temperature to come.
        let next = &ahead[..n.min(13)];
        let count = |c: Condition| next.iter().filter(|h| h.condition == c).count();
        let sky_now = self.conditions(now).condition;
        let sky = if count(Condition::Fog) * 3 >= next.len() {
            "Foggy"
        } else if count(Condition::Clear) * 2 >= next.len() {
            if matches!(sky_now, Condition::Cloudy | Condition::Fog) {
                "Clearing"
            } else {
                "Clear skies"
            }
        } else if count(Condition::Cloudy) * 2 >= next.len() {
            if sky_now == Condition::Clear {
                "Clouding over"
            } else {
                "Cloudy"
            }
        } else {
            "Partly cloudy"
        };
        let unit = |speed: f32| {
            format!(
                "{} {}",
                speed.round() as i32,
                if self.imperial { "mph" } else { "km/h" }
            )
        };
        let windy = if self.imperial { 31. } else { 50. };
        if let Some(gust) = next
            .iter()
            .max_by(|a, b| a.gusts.total_cmp(&b.gusts))
            .filter(|h| h.gusts >= windy)
        {
            return format!("{sky} and windy, gusts to {}", unit(gust.gusts));
        }
        let first = &next[0];
        let warmest = next
            .iter()
            .max_by(|a, b| a.temperature.total_cmp(&b.temperature))
            .unwrap_or(first);
        let coldest = next
            .iter()
            .min_by(|a, b| a.temperature.total_cmp(&b.temperature))
            .unwrap_or(first);
        let deg = |t: f32| format!("{}°", t.round() as i32);
        if warmest.temperature - first.temperature >= 2. {
            format!(
                "{sky}, warming to {} by {}",
                deg(warmest.temperature),
                spoken(warmest.time)
            )
        } else if first.temperature - coldest.temperature >= 2. {
            format!(
                "{sky}, cooling to {} by {}",
                deg(coldest.temperature),
                spoken(coldest.time)
            )
        } else {
            format!("{sky}, steady around {}", deg(first.temperature))
        }
    }
}

/// "3 pm", "12 am" or "15:00".
pub fn hour_text(hour: u8, twelve_hour: bool) -> String {
    if twelve_hour {
        let h = match hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{h} {}", if hour < 12 { "am" } else { "pm" })
    } else {
        format!("{hour:02}:00")
    }
}

/// The 8-point compass name of a direction in degrees ("NW").
pub fn compass(degrees: u16) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    POINTS[((f32::from(degrees % 360) + 22.5) / 45.) as usize % 8]
}

/// "11 h 6 m".
pub fn duration_text(seconds: f32) -> String {
    let m = (seconds / 60.).round() as i64;
    format!("{} h {} m", m / 60, m % 60)
}

/// "2 m 31 s longer than yesterday".
pub fn daylight_change_text(change: f32) -> String {
    let s = change.abs().round() as i64;
    let size = if s >= 60 {
        format!("{} m {} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    };
    match change {
        c if c.abs() < 0.5 => "Same length as yesterday".to_owned(),
        c if c > 0. => format!("{size} longer than yesterday"),
        _ => format!("{size} shorter than yesterday"),
    }
}

/// The Beaufort scale's name for a wind speed in km/h.
pub fn beaufort(kmh: f32) -> &'static str {
    const SCALE: [(f32, &str); 12] = [
        (1., "Calm"),
        (6., "Light air"),
        (12., "Light breeze"),
        (20., "Gentle breeze"),
        (29., "Moderate breeze"),
        (39., "Fresh breeze"),
        (50., "Strong breeze"),
        (62., "Near gale"),
        (75., "Gale"),
        (89., "Strong gale"),
        (103., "Storm"),
        (118., "Violent storm"),
    ];
    SCALE
        .iter()
        .find(|(below, _)| kmh < *below)
        .map_or("Hurricane force", |(_, name)| name)
}

/// The UV index's category.
pub fn uv_category(uv: f32) -> &'static str {
    match uv.round() as i32 {
        ..=2 => "Low",
        3..=5 => "Moderate",
        6..=7 => "High",
        8..=10 => "Very high",
        _ => "Extreme",
    }
}

/// The US Air Quality Index's category.
pub fn aqi_category(aqi: u16) -> &'static str {
    match aqi {
        0..=50 => "Good",
        51..=100 => "Moderate",
        101..=150 => "Unhealthy for some",
        151..=200 => "Unhealthy",
        201..=300 => "Very unhealthy",
        _ => "Hazardous",
    }
}

/// Mean length of the lunar month, days.
const SYNODIC_MONTH: f64 = 29.530_588_853;
/// A new moon: 2000-01-06 18:14 UTC.
const NEW_MOON_2000: f64 = 947_182_440.;

/// The moon's age as a fraction of its month: 0 new, 0.25 first quarter,
/// 0.5 full, 0.75 last quarter (mean motion; within about a day).
pub fn moon_phase(utc: i64) -> f32 {
    let days = (utc as f64 - NEW_MOON_2000) / 86_400.;
    (days / SYNODIC_MONTH).rem_euclid(1.) as f32
}

/// The lit fraction of the disc.
pub fn moon_illumination(phase: f32) -> f32 {
    (1. - (std::f32::consts::TAU * phase).cos()) / 2.
}

pub fn moon_name(phase: f32) -> &'static str {
    match (phase * 8.).round() as i32 % 8 {
        0 => "New moon",
        1 => "Waxing crescent",
        2 => "First quarter",
        3 => "Waxing gibbous",
        4 => "Full moon",
        5 => "Waning gibbous",
        6 => "Last quarter",
        _ => "Waning crescent",
    }
}

/// The next time after `utc` the moon reaches `phase` (0 new, 0.5 full).
pub fn next_moon(utc: i64, phase: f32) -> i64 {
    let ahead = (f64::from(phase) - f64::from(moon_phase(utc))).rem_euclid(1.);
    let ahead = if ahead < 1e-6 { 1. } else { ahead };
    utc + (ahead * SYNODIC_MONTH * 86_400.).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays_and_civil_dates() {
        assert_eq!(weekday("1970-01-01"), "Thu");
        assert_eq!(weekday("2026-10-11"), "Sun");
        assert_eq!(weekday("2024-02-29"), "Thu");
        assert_eq!(weekday("2000-03-01"), "Wed");
        assert_eq!(weekday("nonsense"), "");
        let t = civil(1_709_164_800); // 2024-02-29 00:00 UTC
        assert_eq!((t.year, t.month, t.day, t.weekday), (2024, 2, 29, 4));
        assert_eq!(iso_date(&civil(-1)), "1969-12-31");
    }

    #[test]
    fn wmo_codes() {
        assert_eq!(Condition::from_wmo(0), Condition::Clear);
        assert_eq!(Condition::from_wmo(63), Condition::Rain);
        assert_eq!(Condition::from_wmo(81), Condition::Rain);
        assert_eq!(Condition::from_wmo(75), Condition::Snow);
        assert_eq!(Condition::from_wmo(96), Condition::Storm);
    }

    /// 2026-10-12 00:00 at a place three hours behind UTC.
    const MIDNIGHT: i64 = 1_791_774_000;

    fn hour(time: i64, temperature: f32, condition: Condition, rain_chance: u8) -> Hour {
        Hour {
            time,
            temperature,
            feels_like: temperature - 2.,
            rain_chance,
            precipitation: if condition.wet() { 0.8 } else { 0. },
            condition,
            wind: 12.,
            wind_direction: 300,
            gusts: 25.,
            humidity: 70,
            uv: 2.,
            cloud_cover: 50,
            visibility: 20.,
            is_day: true,
            pressure: 1012.,
            dew_point: 5.,
        }
    }

    /// 36 hours from midnight, `wet` hours (from midnight) raining, read at
    /// 9 am.
    fn forecast(wet: std::ops::Range<i64>) -> Forecast {
        let hours: Vec<Hour> = (0..36)
            .map(|i| {
                let (c, p) = if wet.contains(&i) {
                    (Condition::Rain, 80)
                } else {
                    (Condition::PartlyCloudy, 10)
                };
                hour(MIDNIGHT + i * 3600, 8. + (i % 24) as f32 / 3., c, p)
            })
            .collect();
        Forecast {
            current: hours[9].clone(),
            hours,
            days: vec![],
            imperial: false,
            timezone: "America/Sao_Paulo".into(),
            utc_offset: -3 * 3600,
            fetched_at: MIDNIGHT + 9 * 3600,
            yesterday_daylight: None,
            air: None,
        }
    }

    #[test]
    fn local_time_and_labels_follow_the_place() {
        let mut f = forecast(0..0);
        let t = f.local(MIDNIGHT + 15 * 3600);
        assert_eq!((t.hour, t.day, t.month), (15, 12, 10));
        assert_eq!(f.hour_label(MIDNIGHT + 15 * 3600), "15:00");
        f.timezone = "America/Chicago".into();
        assert_eq!(f.hour_label(MIDNIGHT + 15 * 3600), "3 pm");
        assert_eq!(f.hour_label(MIDNIGHT), "12 am");
        assert_eq!(f.time_label(MIDNIGHT + 7 * 3600 + 12 * 60), "7:12 am");
        f.timezone = "Europe/Berlin".into();
        assert_eq!(f.time_label(MIDNIGHT + 7 * 3600 + 12 * 60), "07:12");
        assert_eq!(date_label(&f.local(MIDNIGHT)), "Mon 12 Oct");
    }

    #[test]
    fn the_hour_cursor_starts_at_the_hour_holding_now() {
        let f = forecast(0..0);
        assert_eq!(f.hour_index(MIDNIGHT + 9 * 3600 + 1799), Some(9));
        assert_eq!(f.hours_from(MIDNIGHT + 9 * 3600 + 1799).len(), 27);
        assert_eq!(f.hours_from(MIDNIGHT - 7200).len(), 36);
        assert!(f.hours_from(MIDNIGHT + 40 * 3600).is_empty());
        // Hours later, a saved forecast shows that hour, not the old reading.
        let mut saved = f.clone();
        saved.current = hour(MIDNIGHT + 9 * 3600, 99., Condition::Clear, 0);
        assert_eq!(saved.conditions(MIDNIGHT + 9 * 3600 + 600).temperature, 99.);
        assert_eq!(
            saved.conditions(MIDNIGHT + 20 * 3600).temperature,
            f.hours[20].temperature
        );
        // Past the end of the forecast, the last reading.
        assert_eq!(saved.conditions(MIDNIGHT + 60 * 3600).temperature, 99.);
    }

    #[test]
    fn summaries_say_when_the_rain_comes_and_goes() {
        let nine = MIDNIGHT + 9 * 3600;
        let twelve = |mut f: Forecast| {
            f.timezone = "America/New_York".into();
            f
        };
        assert_eq!(
            twelve(forecast(15..22)).summary(nine),
            "Rain from 3 pm, clearing tonight"
        );
        assert_eq!(
            twelve(forecast(15..18)).summary(nine),
            "Rain from 3 pm, clearing this evening"
        );
        assert_eq!(
            twelve(forecast(8..12)).summary(nine),
            "Rain until noon, then partly cloudy"
        );
        assert_eq!(
            twelve(forecast(31..34)).summary(nine),
            "Rain from 7 am tomorrow, clearing tomorrow morning"
        );
        assert_eq!(
            twelve(forecast(5..34)).summary(nine),
            "Rain until 10 am tomorrow, then partly cloudy"
        );
        assert_eq!(twelve(forecast(15..40)).summary(nine), "Rain from 3 pm");
        assert_eq!(
            forecast(15..22).summary(nine),
            "Rain from 15:00, clearing tonight"
        );
        let mut snow = twelve(forecast(15..22));
        for h in &mut snow.hours[15..22] {
            h.condition = Condition::Snow;
        }
        assert_eq!(snow.summary(nine), "Snow from 3 pm, clearing tonight");
        // Dry: sky and temperature, or the wind when it gusts.
        assert_eq!(
            twelve(forecast(0..0)).summary(nine),
            "Partly cloudy, warming to 15° by 9 pm"
        );
        let mut clearing = twelve(forecast(0..0));
        for (i, h) in clearing.hours.iter_mut().enumerate() {
            h.condition = Condition::Clear;
            h.temperature = 30. - i.min(24) as f32;
        }
        let two = MIDNIGHT + 14 * 3600;
        clearing.current = hour(two, 16., Condition::Cloudy, 0);
        assert_eq!(clearing.summary(two), "Clearing, cooling to 6° by midnight");
        let mut windy = forecast(0..0);
        windy.hours[12].gusts = 71.;
        assert_eq!(
            windy.summary(nine),
            "Partly cloudy and windy, gusts to 71 km/h"
        );
    }

    #[test]
    fn daylight_change_against_yesterday() {
        let mut f = forecast(0..0);
        f.days = ["2026-10-12", "2026-10-13"]
            .iter()
            .enumerate()
            .map(|(i, date)| Day {
                date: (*date).into(),
                high: 12.,
                low: 4.,
                condition: Condition::Clear,
                rain_chance: None,
                sunrise: 0,
                sunset: 0,
                precipitation: 0.,
                uv_max: 3.,
                wind_max: 10.,
                gust_max: 20.,
                wind_direction: 0,
                daylight: 39_984. - 181. * i as f32,
            })
            .collect();
        f.yesterday_daylight = Some(40_165.12);
        let change = f.daylight_change(MIDNIGHT + 3600).unwrap();
        assert!((change + 181.12).abs() < 0.01);
        assert_eq!(
            daylight_change_text(change),
            "3 m 1 s shorter than yesterday"
        );
        assert_eq!(
            f.daylight_change(MIDNIGHT + 30 * 3600).map(|c| c.round()),
            Some(-181.)
        );
        assert_eq!(daylight_change_text(42.4), "42 s longer than yesterday");
        assert_eq!(daylight_change_text(0.2), "Same length as yesterday");
        assert_eq!(duration_text(39_984.), "11 h 6 m");
        assert_eq!(f.days_from(MIDNIGHT + 30 * 3600).len(), 1);
    }

    #[test]
    fn pressure_trend_over_three_hours() {
        let mut f = forecast(0..0);
        for (i, h) in f.hours.iter_mut().enumerate() {
            h.pressure = 1000. + i as f32 * 0.5;
        }
        assert_eq!(f.pressure_trend(MIDNIGHT + 9 * 3600 + 60), Some(1.5));
        assert_eq!(f.pressure_trend(MIDNIGHT + 3600), None);
    }

    #[test]
    fn moon_phases() {
        // New moon 2024-10-02 18:49 UTC, full moon 2024-10-17 11:26 UTC,
        // new moon 2024-11-01 12:47 UTC.
        let new = 1_727_894_940;
        let full = 1_729_164_360;
        let next_new = 1_730_465_220;
        let near = |a: f32, b: f32| (a - b).abs() < 0.04 || (a - b).abs() > 0.96;
        assert!(near(moon_phase(full), 0.5), "{}", moon_phase(full));
        assert!(near(moon_phase(new), 0.), "{}", moon_phase(new));
        assert_eq!(moon_name(moon_phase(full)), "Full moon");
        assert_eq!(moon_name(0.27), "First quarter");
        assert_eq!(moon_name(0.97), "New moon");
        assert!(moon_illumination(0.5) > 0.99 && moon_illumination(0.) < 0.01);
        assert!((moon_illumination(0.25) - 0.5).abs() < 0.01);
        // Within a day of the real times.
        assert!((next_moon(new + 86_400, 0.5) - full).abs() < 86_400);
        assert!((next_moon(full + 86_400, 0.) - next_new).abs() < 86_400);
        assert!(next_moon(full + 2 * 86_400, 0.5) > full + 20 * 86_400);
    }

    #[test]
    fn categories_and_compass() {
        assert_eq!(compass(0), "N");
        assert_eq!(compass(350), "N");
        assert_eq!(compass(23), "NE");
        assert_eq!(compass(300), "NW");
        assert_eq!(compass(180), "S");
        assert_eq!(uv_category(0.4), "Low");
        assert_eq!(uv_category(5.4), "Moderate");
        assert_eq!(uv_category(11.), "Extreme");
        assert_eq!(aqi_category(37), "Good");
        assert_eq!(aqi_category(120), "Unhealthy for some");
        assert_eq!(aqi_category(420), "Hazardous");
        assert_eq!(beaufort(0.4), "Calm");
        assert_eq!(beaufort(16.4), "Gentle breeze");
        assert_eq!(beaufort(80.), "Strong gale");
        assert_eq!(beaufort(140.), "Hurricane force");
    }
}

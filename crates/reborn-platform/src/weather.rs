//! Weather over Wi-Fi from Open-Meteo (free, no account or key): the city's
//! position from its geocoding, then one forecast request (current
//! conditions, 51 hours, ten days) and one air quality request whose
//! failure is ignored.
use crate::https;
use reborn_core::weather::{civil, iso_date, Air, Condition, Day, Forecast, Hour, Place, Pollen};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

/// Replies are a few tens of kilobytes.
const MAX_BODY: usize = 512 * 1024;

fn get(url: &str) -> Result<Value, String> {
    let r = https::get(url, MAX_BODY)?;
    if r.status != 200 {
        return Err(format!("The weather service answered {}", r.status));
    }
    serde_json::from_slice(&r.body).map_err(|_| "Unexpected weather data".to_owned())
}

/// Percent-encoding for a query value.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The city `name` in `country`: the first match in that country, else the
/// best match anywhere; None when nothing matches.
pub fn geocode(name: &str, country: &str) -> Result<Option<Place>, String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=10&language=en&format=json",
        encode(name)
    );
    Ok(parse_place(&get(&url)?, country))
}

fn parse_place(v: &Value, country: &str) -> Option<Place> {
    let results = v["results"].as_array()?;
    let r = results
        .iter()
        .find(|r| r["country"].as_str() == Some(country))
        .or(results.first())?;
    let region = [r["admin1"].as_str(), r["country"].as_str()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    Some(Place {
        name: r["name"].as_str()?.to_owned(),
        region,
        country_code: r["country_code"].as_str().unwrap_or("").to_owned(),
        latitude: r["latitude"].as_f64()?,
        longitude: r["longitude"].as_f64()?,
    })
}

/// Measures asked for hour by hour and for now (the same names).
const HOURLY: &str = "temperature_2m,apparent_temperature,precipitation_probability,precipitation,\
    weather_code,wind_speed_10m,wind_direction_10m,wind_gusts_10m,relative_humidity_2m,uv_index,\
    cloud_cover,visibility,is_day,pressure_msl,dew_point_2m";
const DAILY: &str = "weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,\
    precipitation_probability_max,sunrise,sunset,uv_index_max,wind_speed_10m_max,\
    wind_gusts_10m_max,wind_direction_10m_dominant,daylight_duration";
const AIR: &str = "us_aqi,pm2_5,pm10,ozone,nitrogen_dioxide,alder_pollen,birch_pollen,\
    grass_pollen,mugwort_pollen,olive_pollen,ragweed_pollen";

/// The forecast for `place`, with its air quality when that answers.
pub fn forecast(place: &Place) -> Result<Forecast, String> {
    let imperial = place.imperial();
    let at = format!(
        "latitude={:.4}&longitude={:.4}",
        place.latitude, place.longitude
    );
    // Yesterday is asked for only for its daylight; three past hours give
    // the pressure's trend.
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?{at}&current={HOURLY}&hourly={HOURLY}\
         &daily={DAILY}&past_days=1&past_hours=3&forecast_hours=48&forecast_days=10\
         &timezone=auto&timeformat=unixtime{}",
        if imperial {
            "&temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch"
        } else {
            ""
        }
    )
    .replace(' ', "");
    let mut f = parse_forecast(&get(&url)?, imperial).ok_or("Unexpected weather data")?;
    f.fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let air = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?{at}&current={AIR}&timezone=auto"
    )
    .replace(' ', "");
    f.air = get(&air).ok().and_then(|v| parse_air(&v));
    Ok(f)
}

fn num(x: &Value) -> Option<f32> {
    x.as_f64().map(|n| n as f32)
}

fn percent(x: &Value) -> Option<u8> {
    x.as_f64().map(|n| n.round().clamp(0., 100.) as u8)
}

/// One hour (or the current conditions) from the `i`th entry of each
/// series in `s` (`i` None: `s` holds single values).
fn parse_hour(s: &Value, i: Option<usize>, imperial: bool) -> Option<Hour> {
    let at = |key: &str| match i {
        Some(i) => &s[key][i],
        None => &s[key],
    };
    let temperature = num(at("temperature_2m"))?;
    // Visibility comes in metres, or feet with imperial units.
    let visibility =
        num(at("visibility")).map_or(0., |v| if imperial { v / 5280. } else { v / 1000. });
    Some(Hour {
        time: at("time").as_i64()?,
        temperature,
        feels_like: num(at("apparent_temperature")).unwrap_or(temperature),
        rain_chance: percent(at("precipitation_probability")).unwrap_or(0),
        precipitation: num(at("precipitation")).unwrap_or(0.),
        condition: Condition::from_wmo(at("weather_code").as_u64()? as u32),
        wind: num(at("wind_speed_10m")).unwrap_or(0.),
        wind_direction: at("wind_direction_10m").as_f64().unwrap_or(0.).round() as u16 % 360,
        gusts: num(at("wind_gusts_10m")).unwrap_or(0.),
        humidity: percent(at("relative_humidity_2m")).unwrap_or(0),
        uv: num(at("uv_index")).unwrap_or(0.),
        cloud_cover: percent(at("cloud_cover")).unwrap_or(0),
        visibility,
        is_day: at("is_day").as_u64() != Some(0),
        pressure: num(at("pressure_msl")).unwrap_or(0.),
        dew_point: num(at("dew_point_2m")).unwrap_or(temperature),
    })
}

/// A forecast reply; `pub` for the preview's fixture.
pub fn parse_forecast_json(body: &[u8], imperial: bool) -> Option<Forecast> {
    parse_forecast(&serde_json::from_slice(body).ok()?, imperial)
}

/// An air quality reply; `pub` for the preview's fixture.
pub fn parse_air_json(body: &[u8]) -> Option<Air> {
    parse_air(&serde_json::from_slice(body).ok()?)
}

fn parse_forecast(v: &Value, imperial: bool) -> Option<Forecast> {
    let utc_offset = v["utc_offset_seconds"].as_i64().unwrap_or(0) as i32;
    let current = parse_hour(&v["current"], None, imperial)?;
    let h = &v["hourly"];
    let hours: Vec<Hour> = (0..h["time"].as_array().map_or(0, Vec::len))
        .filter_map(|i| parse_hour(h, Some(i), imperial))
        .collect();
    let d = &v["daily"];
    let mut days: Vec<Day> = (0..d["time"].as_array().map_or(0, Vec::len))
        .filter_map(|i| {
            // Days start at local midnight.
            let start = d["time"][i].as_i64()?;
            Some(Day {
                date: iso_date(&civil(start + i64::from(utc_offset))),
                high: num(&d["temperature_2m_max"][i])?,
                low: num(&d["temperature_2m_min"][i])?,
                condition: Condition::from_wmo(d["weather_code"][i].as_u64()? as u32),
                rain_chance: percent(&d["precipitation_probability_max"][i]),
                sunrise: d["sunrise"][i].as_i64().unwrap_or(0),
                sunset: d["sunset"][i].as_i64().unwrap_or(0),
                precipitation: num(&d["precipitation_sum"][i]).unwrap_or(0.),
                uv_max: num(&d["uv_index_max"][i]).unwrap_or(0.),
                wind_max: num(&d["wind_speed_10m_max"][i]).unwrap_or(0.),
                gust_max: num(&d["wind_gusts_10m_max"][i]).unwrap_or(0.),
                wind_direction: d["wind_direction_10m_dominant"][i]
                    .as_f64()
                    .unwrap_or(0.)
                    .round() as u16
                    % 360,
                daylight: num(&d["daylight_duration"][i]).unwrap_or(0.),
            })
        })
        .collect();
    // The first day is yesterday (past_days=1): keep only its daylight.
    let today = iso_date(&civil(current.time + i64::from(utc_offset)));
    let mut yesterday_daylight = None;
    while days.first().is_some_and(|day| day.date < today) {
        yesterday_daylight = Some(days.remove(0).daylight).filter(|d| *d > 0.);
    }
    Some(Forecast {
        current,
        hours,
        days,
        imperial,
        timezone: v["timezone"].as_str().unwrap_or("").to_owned(),
        utc_offset,
        fetched_at: 0,
        yesterday_daylight,
        air: None,
    })
}

fn parse_air(v: &Value) -> Option<Air> {
    let c = &v["current"];
    let pollen = [
        ("Alder", "alder_pollen"),
        ("Birch", "birch_pollen"),
        ("Grass", "grass_pollen"),
        ("Mugwort", "mugwort_pollen"),
        ("Olive", "olive_pollen"),
        ("Ragweed", "ragweed_pollen"),
    ]
    .into_iter()
    .filter_map(|(name, key)| {
        Some(Pollen {
            name: name.to_owned(),
            grains: num(&c[key])?,
        })
    })
    .collect();
    let air = Air {
        us_aqi: c["us_aqi"].as_u64().map(|a| a.min(999) as u16),
        pm2_5: num(&c["pm2_5"]),
        pm10: num(&c["pm10"]),
        ozone: num(&c["ozone"]),
        nitrogen_dioxide: num(&c["nitrogen_dioxide"]),
        pollen,
    };
    (air.us_aqi.is_some() || air.pm2_5.is_some()).then_some(air)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn geocoding_prefers_the_city_in_the_chosen_country() {
        let v = json!({"results":[
            {"name":"Hyderabad","latitude":17.38,"longitude":78.46,"country_code":"IN",
             "country":"India","admin1":"Telangana"},
            {"name":"Hyderabad","latitude":25.39,"longitude":68.37,"country_code":"PK",
             "country":"Pakistan","admin1":"Sindh"}]});
        let p = parse_place(&v, "Pakistan").unwrap();
        assert_eq!(
            (p.region.as_str(), p.country_code.as_str()),
            ("Sindh, Pakistan", "PK")
        );
        // A country spelled differently falls back to the best match.
        assert_eq!(parse_place(&v, "Elsewhere").unwrap().country_code, "IN");
        assert!(!p.imperial());
        assert_eq!(
            parse_place(&json!({"generationtime_ms":0.1}), "India"),
            None
        );
        assert_eq!(encode("Sao Paulo & co"), "Sao%20Paulo%20%26%20co");
    }

    /// Synthetic replies in Open-Meteo's shape: a city an hour east of UTC
    /// read at 14:20 local on 13 October 2026, and its air with pollen.
    const FORECAST: &[u8] = include_bytes!("weather_fixtures/forecast.json");
    const FORECAST_US: &[u8] = include_bytes!("weather_fixtures/forecast-us.json");
    const AIR_REPLY: &[u8] = include_bytes!("weather_fixtures/air.json");

    #[test]
    fn parses_a_full_forecast() {
        let f = parse_forecast_json(FORECAST, false).unwrap();
        assert_eq!(f.utc_offset, 3600);
        let now = &f.current;
        assert_eq!(f.local(now.time).hour, 14);
        assert_eq!(now.wind_direction, 250);
        assert!((now.visibility - 24.).abs() < 0.01);
        // Three past hours and 48 to come.
        assert_eq!(f.hours.len(), 51);
        assert_eq!(f.hour_index(now.time), Some(3));
        // Ten days from today; yesterday kept as its daylight.
        assert_eq!(f.days.len(), 10);
        assert_eq!(f.days[0].date, "2026-10-13");
        assert!(f.yesterday_daylight.is_some());
        assert!(f.daylight_change(now.time).unwrap() < 0.);
        assert!(f.days[0].sunrise < f.days[0].sunset);
        assert_eq!(f.today(now.time), Some(0));
        assert!(f.summary(now.time).starts_with("Rain from"));
    }

    #[test]
    fn imperial_units_are_converted_where_needed() {
        let f = parse_forecast_json(FORECAST_US, true).unwrap();
        assert!(f.imperial);
        // 78740 feet is 14.9 miles.
        assert!((f.current.visibility - 14.91).abs() < 0.01);
        assert!(f.current.temperature > 40.);
    }

    #[test]
    fn parses_air_quality_with_and_without_pollen() {
        let air = parse_air_json(AIR_REPLY).unwrap();
        assert_eq!(air.us_aqi, Some(42));
        assert_eq!(air.pollen.len(), 6);
        assert_eq!(air.pm2_5, Some(8.5));
        let bare = json!({"current":{"us_aqi":37,"pm2_5":4.2,"pm10":7.0}});
        assert!(parse_air(&bare).unwrap().pollen.is_empty());
        assert_eq!(parse_air(&json!({"error": true})), None);
    }
}

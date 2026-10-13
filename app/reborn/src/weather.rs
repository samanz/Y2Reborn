//! The weather worker: geocoding and forecasts go over the network on their
//! own thread; the UI thread sends requests and drains replies each pass.
//! The chosen city and its last forecast are kept in `weather.json`.
use reborn_core::{
    cities::CITIES,
    weather::{Forecast, Place},
};
use reborn_ui::weather::WeatherView;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::mpsc::{sync_channel, Receiver, SyncSender},
};

const FILE: &str = "weather.json";

pub enum Request {
    /// Find `CITIES[i]`.
    City(usize),
    Forecast(Place),
}

pub enum Reply {
    City(Result<Option<Place>, String>),
    /// Boxed: a full forecast is much larger than the other reply.
    Forecast(Result<Box<Forecast>, String>),
}

pub struct Worker {
    requests: SyncSender<Request>,
    pub replies: Receiver<Reply>,
}

impl Worker {
    pub fn spawn() -> Result<Self, std::io::Error> {
        let (requests, rx) = sync_channel::<Request>(2);
        let (tx, replies) = sync_channel(2);
        std::thread::Builder::new()
            .name("weather".into())
            .spawn(move || {
                while let Ok(request) = rx.recv() {
                    let reply = match request {
                        Request::City(i) => {
                            let (name, country) = CITIES[i];
                            Reply::City(reborn_platform::weather::geocode(name, country))
                        }
                        Request::Forecast(place) => Reply::Forecast(
                            reborn_platform::weather::forecast(&place).map(Box::new),
                        ),
                    };
                    if tx.send(reply).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { requests, replies })
    }

    /// False when the worker is still busy with earlier requests.
    pub fn send(&self, request: Request) -> bool {
        self.requests.try_send(request).is_ok()
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    place: Option<Place>,
    forecast: Option<Forecast>,
}

/// The saved city and forecast, if any (a file that does not parse is
/// ignored).
pub fn load(root: &Path, view: &mut WeatherView) {
    if let Some(saved) = fs::read(root.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
    {
        view.place = saved.place;
        view.forecast = saved.forecast;
    }
}

pub fn save(root: &Path, view: &WeatherView) -> Result<(), String> {
    let saved = Saved {
        place: view.place.clone(),
        forecast: view.forecast.clone(),
    };
    let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
    reborn_core::atomic_write(&root.join(FILE), &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_city_and_forecast_survive_a_restart() {
        let root = std::env::temp_dir().join(format!("reborn-weather-{}", std::process::id()));
        let mut view = WeatherView {
            place: Some(Place {
                name: "Lisbon".into(),
                region: "Lisbon, Portugal".into(),
                country_code: "PT".into(),
                latitude: 38.72,
                longitude: -9.14,
            }),
            ..Default::default()
        };
        save(&root, &view).unwrap();
        view.place = None;
        load(&root, &mut view);
        assert_eq!(view.place.as_ref().map(|p| p.name.as_str()), Some("Lisbon"));
        // A damaged file leaves the view alone.
        fs::write(root.join(FILE), b"{not json").unwrap();
        load(&root, &mut view);
        assert!(view.place.is_some());
        fs::remove_dir_all(root).unwrap();
    }
}

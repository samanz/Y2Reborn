//! Retro games in the app: finding systems and games, starting and
//! stopping the frontend (`reborn-retro`, which loads the libretro core),
//! its pictures into the GPU's picture texture, its sound through the
//! audio output (music pauses and lets go of the device first, and comes
//! back after), and the buttons straight to it while the game is on
//! screen. The game pauses whenever its screen is not showing (the game
//! menu, the screen off) and ends when the player leaves its screens.
use crate::{retro_audio::GameAudio, Runtime};
use reborn_core::{
    retro::{fit, layout_for, save_stem, Command, Controls, Input, Launch, StatusKind, System},
    AudioOutput, PlaybackState, Screen,
};
use reborn_observability::Level;
use reborn_platform::{
    input::InputEvent,
    retro::{self, Engine, Game},
};
use reborn_ui::retro::{Scan, SystemRow};
use serde_json::json;
use std::{
    io::Write,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Folder under the data root: one per system, for battery saves and
/// states.
const SAVES: &str = "retro";

pub struct Retro {
    engine: Option<Engine>,
    pub controls: Controls,
    paused: bool,
    /// The picture being uploaded (swapped with the reader's).
    frame: Vec<u8>,
    /// How the picture texture is sampled now (None: as Reborn left it).
    nearest: Option<bool>,
    /// Wakes the main loop when a picture arrives.
    pub wake: Option<UnixStream>,
    audio: Option<GameAudio>,
    /// Music was playing when the game started: it plays again after.
    music_resume: bool,
    /// The time base for the controls' holds and turns.
    epoch: Instant,
}

impl Default for Retro {
    fn default() -> Self {
        Self {
            engine: None,
            controls: Controls::default(),
            paused: false,
            frame: vec![],
            nearest: None,
            wake: None,
            audio: None,
            music_resume: false,
            epoch: Instant::now(),
        }
    }
}

impl Retro {
    /// A game is running and showing: pictures are due 60 times a second.
    pub fn playing(&self) -> bool {
        self.engine.is_some() && !self.paused
    }

    /// The game has the audio output (music must not start).
    pub fn holds_audio(&self) -> bool {
        self.audio.is_some()
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

impl Runtime {
    /// `Games` on every card present.
    fn retro_games_dirs(&self) -> Vec<PathBuf> {
        self.model
            .sources
            .iter()
            .filter(|s| s.online)
            .map(|s| s.root.join(reborn_core::retro::CARD_FOLDER))
            .collect()
    }

    fn retro_saves(&self, system: System) -> PathBuf {
        self.root.join(SAVES).join(system.key())
    }

    /// Music lets go of the output; the game's sound gets a sink planned
    /// like music's (the wired output, or the Bluetooth A2DP PCM).
    fn retro_audio(&mut self) -> Option<GameAudio> {
        let id = self.log.correlation();
        if let Err(error) = self.playback.stop_and_wait(self.model.generation) {
            self.log.emit(
                Level::Warn,
                "retro",
                "audio_busy",
                &error,
                Some(id),
                json!({}),
            );
            return None;
        }
        let output = self.model.output.clone();
        let planned = match &output {
            AudioOutput::Wired => {
                reborn_audio::AlsaSink::plan(&output, 48_000, self.log.clone(), id).or_else(|_| {
                    reborn_audio::AlsaSink::plan(&output, 44_100, self.log.clone(), id)
                })
            }
            AudioOutput::Bluetooth(address) => {
                self.bt_state.playback_pcm(address).and_then(|pcm| {
                    reborn_audio::AlsaSink::plan_bluetooth(&output, &pcm, self.log.clone(), id)
                })
            }
        };
        match planned {
            Ok((spec, _)) => Some(GameAudio::start(
                spec,
                self.model.settings.volume,
                self.log.clone(),
                id,
            )),
            Err(error) => {
                self.log.emit(
                    Level::Warn,
                    "retro",
                    "no_audio",
                    &error,
                    Some(id),
                    json!({}),
                );
                None
            }
        }
    }

    pub(crate) fn retro_start(&mut self, launch: Launch) {
        self.retro_stop();
        let Some(binary) = retro::binary() else {
            self.ui.retro_ended(
                &mut self.model,
                Some("the game player (reborn-retro) is not installed".into()),
            );
            return;
        };
        let Some(core) = retro::core(launch.system) else {
            let why = format!("the {} emulator is not installed", launch.system.core());
            self.ui.retro_ended(&mut self.model, Some(why));
            return;
        };
        // The game's sound takes the output from music.
        let was_playing = matches!(
            self.model.playback,
            PlaybackState::Playing | PlaybackState::Buffering
        );
        if was_playing {
            self.pause();
        }
        let audio = self.retro_audio();
        let saves = self.retro_saves(launch.system);
        let bios = self
            .retro_games_dirs()
            .into_iter()
            .map(|g| g.join(reborn_core::retro::BIOS_FOLDER))
            .find(|b| b.is_dir())
            .unwrap_or_else(|| saves.clone());
        let wake = self
            .retro
            .wake
            .as_ref()
            .and_then(|w| w.try_clone().ok())
            .map(|w| Arc::new(Mutex::new(w)));
        let notify = move || {
            if let Some(w) = &wake {
                if let Ok(mut w) = w.lock() {
                    let _ = w.write(&[1]);
                }
            }
        };
        let feed = audio.as_ref().map(|a| a.feeder());
        let sound = move |s: &[i16]| {
            if let Some(f) = &feed {
                f(s);
            }
        };
        let game = Game {
            core: &core,
            rom: &launch.rom,
            system_dir: &bios,
            saves: &saves,
            rate: audio.as_ref().map_or(0, |a| a.rate),
        };
        match Engine::start(&binary, &game, notify, sound) {
            Ok(engine) => {
                self.log.emit(
                    Level::Info,
                    "retro",
                    "started",
                    "Retro game started",
                    None,
                    json!({"core":core,"rom":launch.rom,"rate":game.rate}),
                );
                self.ui.retro.has_state = saves
                    .join(format!("{}.state", save_stem(&launch.rom)))
                    .is_file();
                self.retro.engine = Some(engine);
                self.retro.audio = audio;
                self.retro.paused = false;
                self.retro.controls = Controls::new(layout_for(launch.system, &launch.rom));
                self.retro.music_resume = was_playing;
            }
            Err(error) => {
                drop(audio);
                self.ui.retro_ended(
                    &mut self.model,
                    Some(format!("it could not start ({error})")),
                );
                if was_playing {
                    self.retro_resume_music();
                }
            }
        }
    }

    fn retro_resume_music(&mut self) {
        if self.model.playback == PlaybackState::Paused {
            if let Err(error) = self.toggle() {
                self.log.emit(
                    Level::Warn,
                    "retro",
                    "music_resume",
                    &error,
                    None,
                    json!({}),
                );
            }
        }
    }

    pub(crate) fn retro_stop(&mut self) {
        if let Some(mut engine) = self.retro.engine.take() {
            let _ = engine.send(&self.retro.controls.release_all());
            engine.stop();
            self.log.emit(
                Level::Info,
                "retro",
                "stopped",
                "Retro game asked to save and quit",
                None,
                json!({}),
            );
        }
        // Dropping the worker closes the sink: the output is free again.
        self.retro.audio = None;
        self.retro.paused = false;
        if self.retro.nearest.take().is_some() {
            if let Some(g) = self.graphics.as_mut() {
                let _ = g.picture_filter(false); // Doom's smooth sampling
            }
        }
        if std::mem::take(&mut self.retro.music_resume) {
            self.retro_resume_music();
        }
    }

    pub(crate) fn retro_command(&mut self, command: Command) {
        let input = match command {
            Command::SaveState => Input::SaveState,
            Command::LoadState => Input::LoadState,
            Command::Tap(pad) => Input::Tap(pad),
        };
        if let Some(engine) = self.retro.engine.as_mut() {
            let _ = engine.send(&[input]);
        }
    }

    /// Every main-loop pass: look for games when asked, follow the game
    /// (stopped by itself, paused, left), and show its newest picture.
    pub(crate) fn pump_retro(&mut self) {
        if let Some(scan) = self.ui.retro.scan.take() {
            let dirs = self.retro_games_dirs();
            match scan {
                Scan::Systems => {
                    self.ui.retro.systems = System::ALL
                        .into_iter()
                        .map(|system| SystemRow {
                            system,
                            games: retro::find_roms(system, &dirs).len(),
                            core: retro::core(system).is_some(),
                        })
                        .collect();
                }
                Scan::Roms(system) => {
                    self.ui.retro.roms = retro::find_roms(system, &dirs);
                    if self.ui.retro.systems.is_empty() {
                        self.ui.retro.scan = Some(Scan::Systems);
                    }
                }
            }
            self.dirty.mark_render();
        }
        let Some(engine) = self.retro.engine.as_mut() else {
            return;
        };
        if let Some(failure) = engine.ended() {
            self.retro.engine = None;
            if let Some(why) = &failure {
                self.log
                    .emit(Level::Warn, "retro", "failed", why, None, json!({}));
            }
            self.ui.retro_ended(&mut self.model, failure);
            self.retro_stop();
            self.dirty.mark_render();
            return;
        }
        if !self.ui.retro.in_session(&self.model) {
            // Left by any route (Quit, held Back from the menu, an alert).
            self.ui.retro.running = None;
            self.ui.retro.frame = None;
            self.retro_stop();
            self.dirty.mark_render();
            return;
        }
        if self.model.screen_off && self.model.screen == Screen::RetroGame {
            // On waking, the game menu: nothing moves before the player is
            // ready.
            self.ui.retro_open_menu(&mut self.model);
        }
        let pause = self.model.screen_off
            || self.sleep_prepared
            || self.model.screen != Screen::RetroGame
            || self.model.navigation.modal.is_some();
        if pause != self.retro.paused {
            let mut inputs = vec![];
            if pause {
                inputs = self.retro.controls.release_all();
                inputs.push(Input::Pause);
            } else {
                inputs.push(Input::Resume);
            }
            self.retro.paused = pause;
            if engine.send(&inputs).is_err() {
                return; // it has gone; `ended` reports it next pass
            }
        }
        let now = self.retro.epoch.elapsed().as_millis() as u64;
        let held = self.retro.controls.tick(now);
        let _ = engine.send(&held);
        if let Some(audio) = self.retro.audio.as_mut() {
            audio.follow(
                !pause,
                !self.model.screen_off && !self.sleep_prepared,
                self.model.settings.volume,
            );
        }
        for (kind, text) in engine.take_statuses() {
            if kind == StatusKind::StateSaved {
                self.ui.retro.has_state = true;
            }
            self.ui.retro.status = Some(text);
            self.dirty.mark_render();
        }
        if let Some((w, h)) = engine.take_frame(&mut self.retro.frame) {
            if let Some(g) = self.graphics.as_mut() {
                let nearest = fit(w, h).nearest;
                if self.retro.nearest != Some(nearest) && g.picture_filter(nearest).is_ok() {
                    self.retro.nearest = Some(nearest);
                }
                if g.picture(&self.retro.frame, w as usize, h as usize).is_ok() {
                    self.ui.retro.frame = Some((w, h));
                }
            }
            self.dirty.mark_render();
        }
    }

    /// One physical input event: true when the game took it (it is on
    /// screen), or when it is the rest of a press made during play.
    pub(crate) fn retro_input(&mut self, event: &InputEvent) -> bool {
        if self.retro.controls.swallows(event.input) {
            return true;
        }
        if self.retro.engine.is_none()
            || !self.ui.retro.playing(&self.model)
            || self.model.screen_off
            || self.model.navigation.modal.is_some()
        {
            return false;
        }
        let now = self.retro.now_ms();
        let Some(inputs) = self.retro.controls.event(event.input, now) else {
            return false; // Power: the game menu
        };
        self.last_activity = Instant::now();
        if let Some(engine) = self.retro.engine.as_mut() {
            let _ = engine.send(&inputs);
        }
        true
    }
}

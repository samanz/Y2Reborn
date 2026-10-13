//! Doom in the app: finding the games, starting and stopping the engine
//! (`reborn-doom`, a separate GPL program the system installs, spoken to
//! over pipes), its frames into the GPU's picture texture, and the buttons
//! straight to it while the game is on screen. The game pauses while the
//! screen is off and ends when the player leaves the Doom screen. There is no
//! sound: the engine never opens the audio device, so music carries on.
use crate::Runtime;
use reborn_core::{
    doom::{Controls, Input, Out, CARD_FOLDER},
    Screen,
};
use reborn_observability::Level;
use reborn_platform::{
    doom::{self, Engine},
    input::InputEvent,
};
use serde_json::json;
use std::{
    io::Write,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Folder under the data root for each game's saves.
const SAVES: &str = "doom";

#[derive(Default)]
pub struct Doom {
    engine: Option<Engine>,
    controls: Controls,
    paused: bool,
    /// The frame being uploaded (swapped with the reader's, never copied).
    frame: Vec<u8>,
    /// Wakes the main loop when a frame arrives.
    pub wake: Option<UnixStream>,
}

impl Doom {
    /// A game is running and showing: frames are due about 35 times a
    /// second.
    pub fn playing(&self) -> bool {
        self.engine.is_some() && !self.paused
    }
}

impl Runtime {
    fn doom_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self
            .model
            .sources
            .iter()
            .filter(|s| s.online)
            .map(|s| s.root.join(CARD_FOLDER))
            .collect();
        dirs.push(self.root.join(SAVES));
        dirs
    }

    pub(crate) fn doom_start(&mut self, iwad: PathBuf) {
        self.doom_stop();
        let Some(binary) = doom::binary() else {
            self.ui
                .doom_ended(Some("the Doom engine is not installed".into()));
            return;
        };
        let Some(wad) = self.ui.doom.wads.iter().find(|w| w.path == iwad) else {
            self.ui
                .doom_ended(Some("that game is no longer there".into()));
            return;
        };
        let home = self
            .root
            .join(SAVES)
            .join(reborn_core::doom::save_folder(wad));
        // The reader thread wakes the main loop for each frame.
        let wake = self
            .doom
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
        match Engine::start(&binary, &iwad, &home, notify) {
            Ok(engine) => {
                self.log.emit(
                    Level::Info,
                    "doom",
                    "started",
                    "Doom engine started",
                    None,
                    json!({"iwad":iwad,"home":home}),
                );
                self.doom.engine = Some(engine);
                self.doom.paused = false;
                self.doom.controls = Controls::default();
            }
            Err(error) => {
                self.ui
                    .doom_ended(Some(format!("it could not start ({error})")));
            }
        }
    }

    pub(crate) fn doom_stop(&mut self) {
        if let Some(mut engine) = self.doom.engine.take() {
            let _ = engine.send(&self.doom.controls.release_all());
            engine.stop();
            self.log.emit(
                Level::Info,
                "doom",
                "stopped",
                "Doom engine asked to quit",
                None,
                json!({}),
            );
        }
        self.doom.paused = false;
    }

    /// Every main-loop pass: look for games when asked, follow the engine
    /// (stopped by itself, screen off, left), and show its newest frame.
    pub(crate) fn pump_doom(&mut self) {
        if std::mem::take(&mut self.ui.doom.scan) {
            self.ui.doom.wads = doom::find_games(&self.doom_dirs());
            self.dirty.mark_render();
        }
        let Some(engine) = self.doom.engine.as_mut() else {
            return;
        };
        if let Some(failure) = engine.ended() {
            self.doom.engine = None;
            if let Some(why) = &failure {
                self.log
                    .emit(Level::Warn, "doom", "failed", why, None, json!({}));
            }
            self.ui.doom_ended(failure);
            self.dirty.mark_render();
            return;
        }
        if self.model.screen != Screen::Doom {
            // Left by any route (an alert, Now Playing from the remote).
            self.ui.doom_ended(None);
            self.doom_stop();
            return;
        }
        let pause = self.model.screen_off || self.sleep_prepared;
        if pause != self.doom.paused {
            let mut inputs = self.doom.controls.release_all();
            inputs.push(if pause { Input::Pause } else { Input::Resume });
            self.doom.paused = pause;
            if engine.send(&inputs).is_err() {
                return; // it has gone; `ended` reports it next pass
            }
        }
        if engine.take_frame(&mut self.doom.frame) {
            if let Some(g) = self.graphics.as_mut() {
                if g.picture(
                    &self.doom.frame,
                    reborn_core::doom::WIDTH,
                    reborn_core::doom::HEIGHT,
                )
                .is_ok()
                {
                    self.ui.doom.frame = true;
                }
            }
            self.dirty.mark_render();
        }
    }

    /// One physical input event: true when Doom took it (the game is on
    /// screen), or when it is the end of the press that left the game.
    pub(crate) fn doom_input(&mut self, event: &InputEvent) -> bool {
        if self.doom.controls.swallows(event.input) {
            return true;
        }
        if self.doom.engine.is_none()
            || self.doom.paused
            || !self.ui.doom.playing(&self.model)
            || self.model.navigation.modal.is_some()
            || self.ui.alert.is_some()
        {
            return false;
        }
        let Some(outs) = self.doom.controls.event(event.input) else {
            return false; // Power keeps its meaning
        };
        self.last_activity = Instant::now();
        let mut inputs = vec![];
        let mut leave = false;
        for out in outs {
            match out {
                Out::Send(input) => inputs.push(input),
                Out::Leave => leave = true,
            }
        }
        if let Some(engine) = self.doom.engine.as_mut() {
            let _ = engine.send(&inputs);
        }
        if leave {
            let effect = self.ui.doom_leave(&mut self.model);
            if let Err(error) = self.effect(effect) {
                self.fail("doom", error);
            }
            self.dirty.mark_render();
        }
        true
    }
}

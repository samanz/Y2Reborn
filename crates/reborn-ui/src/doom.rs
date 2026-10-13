//! Doom (Extras > Games): a picker of the games found (Freedoom or any IWAD in
//! the card's Doom folder), then the game itself, drawn from the engine's
//! frames. While a game runs the app sends the buttons straight to the
//! engine (`reborn_core::doom::Controls`); this screen only shows it.
use crate::{
    canvas::Canvas,
    pages::{Hero, Page},
    text::Face,
    theme::color,
    Item, RowKind, Ui,
};
use reborn_core::{doom::Wad, AppModel, Effect, Screen};
use reborn_graphics::{Quad, Tex};

/// The engine's 320x200 frame at 1.5x, centred: 480x300 between two
/// 30-pixel black bands.
pub const FRAME_RECT: (f32, f32, f32, f32) = (0., 30., 480., 300.);

/// How to play, in full (the Controls row).
const CONTROLS: &str = "Wheel: turn. Next: walk forward. Previous: back up. Select: fire. \
Play/Pause: open doors and use switches; hold it for the map. Volume up and down: change weapon. \
Back: Doom's menu, where the wheel moves and Select chooses; save your game there. \
Hold Back to leave Doom. There is no sound; your music keeps playing.";

const ABOUT: &str = "The Doom engine is doomgeneric (Chocolate Doom), free software under the \
GNU GPL version 2 or later, installed with the system and running as its own program. Freedoom \
is by the Freedoom project (BSD licence). Doom and Doom II need your own doom.wad or doom2.wad \
in the Doom folder on the card.";

#[derive(Debug, Default, Clone)]
pub struct DoomView {
    /// The system has the engine (`reborn-doom`); without it Doom is not
    /// offered at all.
    pub installed: bool,
    /// The games found (set by the app when the screen opens).
    pub wads: Vec<Wad>,
    /// Asked the app to look for games.
    pub scan: bool,
    /// A game is running (the engine process is alive).
    pub running: bool,
    /// The engine's latest frame is resident in `Tex::Picture`.
    pub frame: bool,
    /// Why the last game stopped on its own, in plain words.
    pub error: Option<String>,
}

impl DoomView {
    /// The game is on screen: draw it, and send the buttons to it.
    pub fn playing(&self, m: &AppModel) -> bool {
        self.running && m.screen == Screen::Doom
    }
}

/// The Games row, when the engine is installed.
pub(crate) fn games_row(v: &DoomView) -> Option<Item> {
    v.installed.then(|| {
        Item::new("Doom", "doom")
            .with_secondary("Freedoom, or your own WADs")
            .with_kind(RowKind::Chevron)
    })
}

/// The picker: one row per game, then how to play and the credits.
pub(crate) fn page(v: &DoomView) -> Page {
    let mut rows: Vec<Item> = v
        .wads
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let file = w
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Item::new(w.title.clone(), format!("doom_play:{i}")).with_secondary(file)
        })
        .collect();
    rows.push(
        Item::new("Controls", format!("value:Controls\u{1f}{CONTROLS}"))
            .with_kind(RowKind::Chevron),
    );
    rows.push(
        Item::new("About Doom", format!("value:About Doom\u{1f}{ABOUT}"))
            .with_kind(RowKind::Chevron),
    );
    let mut page = Page {
        rows,
        status: Some(match &v.error {
            Some(e) => format!("Doom stopped: {e}"),
            None => "Select fires, Play opens doors, hold Back to leave".into(),
        }),
        ..Default::default()
    };
    if v.wads.is_empty() && !v.scan {
        page.hero = Some(Hero {
            title: "No games found".into(),
            body: "Put freedoom1.wad, freedoom2.wad or your own doom.wad in a folder called Doom on the card."
                .into(),
        });
    }
    page
}

impl Ui {
    /// Row keys owned by Doom.
    pub(crate) fn doom_key(&mut self, m: &mut AppModel, key: &str) -> Option<Effect> {
        if key == "doom" {
            Self::go(m, Screen::Doom, "");
            self.doom.scan = true;
            self.doom.error = None;
            return Some(Effect::None);
        }
        let i: usize = key.strip_prefix("doom_play:")?.parse().ok()?;
        let wad = self.doom.wads.get(i)?;
        self.doom.running = true;
        self.doom.frame = false;
        self.doom.error = None;
        Some(Effect::DoomStart(wad.path.clone()))
    }

    /// Back held during a game: leave it for the list of games.
    pub fn doom_leave(&mut self, m: &mut AppModel) -> Effect {
        self.doom.running = false;
        self.doom.frame = false;
        if m.screen == Screen::Doom {
            Self::back(m);
        }
        Effect::DoomStop
    }

    /// The engine stopped by itself (Quit Game, or a failure): back to the
    /// picker, saying why when it failed.
    pub fn doom_ended(&mut self, error: Option<String>) {
        self.doom.running = false;
        self.doom.frame = false;
        self.doom.error = error;
    }
}

/// The game, full width: black bands, the frame (or a word while the
/// engine starts).
pub(crate) fn draw(c: &mut Canvas, v: &DoomView) {
    c.rect(0., 0., 480., 360., color::BLACK);
    let (x, y, w, h) = FRAME_RECT;
    if v.frame {
        c.draw.push(Quad {
            tex: Tex::Picture,
            ..Quad::rect(x, y, w, h, 0xFFFFFFFF)
        });
    } else {
        c.text_mid_center(240., 180., "Starting Doom…", Face::R15, color::MUTED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::Action;
    use std::path::PathBuf;

    fn wad(file: &str, title: &str) -> Wad {
        Wad {
            title: title.into(),
            path: PathBuf::from(format!("/sd/Doom/{file}")),
        }
    }

    fn installed() -> Ui {
        let mut ui = Ui::default();
        ui.doom.installed = true;
        ui
    }

    fn on_doom(ui: &mut Ui) -> AppModel {
        let mut m = AppModel {
            screen: Screen::Games,
            ..Default::default()
        };
        assert_eq!(ui.doom_key(&mut m, "doom"), Some(Effect::None));
        assert_eq!(m.screen, Screen::Doom);
        assert!(ui.doom.scan, "the app looks for games");
        m
    }

    #[test]
    fn games_lists_doom_after_the_built_in_games_only_when_the_engine_is_installed() {
        let mut m = AppModel {
            screen: Screen::Games,
            ..Default::default()
        };
        let labels = |ui: &Ui, m: &AppModel| -> Vec<String> {
            crate::pages::page(ui, m, &[])
                .rows
                .into_iter()
                .map(|r| r.label)
                .collect()
        };
        assert_eq!(labels(&Ui::default(), &m), ["Shatterfield", "Pellet Rush"]);
        let mut ui = installed();
        assert_eq!(labels(&ui, &m), ["Shatterfield", "Pellet Rush", "Doom"]);
        // Home is the same either way.
        let home = AppModel::default();
        assert_eq!(
            labels(&ui, &home),
            labels(&Ui::default(), &home),
            "no Home row of its own"
        );
        let rows = crate::pages::page(&ui, &m, &[]).rows;
        m.navigation.focus = rows
            .iter()
            .position(|r| r.key == "doom")
            .expect("a Doom row");
        assert_eq!(ui.action(&mut m, &[], Action::Select), Effect::None);
        assert_eq!(m.screen, Screen::Doom);
    }

    #[test]
    fn the_picker_lists_games_and_starts_one() {
        let mut ui = installed();
        let mut m = on_doom(&mut ui);
        ui.doom.scan = false;
        ui.doom.wads = vec![
            wad("freedoom1.wad", "Freedoom: Phase 1"),
            wad("doom2.wad", "Doom II"),
        ];
        let page = page(&ui.doom);
        assert!(page.hero.is_none());
        let labels: Vec<&str> = page.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Freedoom: Phase 1", "Doom II", "Controls", "About Doom"]
        );
        assert_eq!(page.rows[1].secondary, "doom2.wad");
        assert!(page.status.unwrap().contains("hold Back"));
        m.navigation.focus = 1;
        assert_eq!(
            ui.action(&mut m, &[], Action::Select),
            Effect::DoomStart(PathBuf::from("/sd/Doom/doom2.wad"))
        );
        assert!(ui.doom.playing(&m));
        // Held Back leaves for the list of games and stops the engine.
        assert_eq!(ui.doom_leave(&mut m), Effect::DoomStop);
        assert_eq!(m.screen, Screen::Games);
        assert!(!ui.doom.playing(&m));
    }

    #[test]
    fn no_games_says_where_to_put_them_and_failures_are_shown() {
        let mut ui = installed();
        let _m = on_doom(&mut ui);
        assert!(page(&ui.doom).hero.is_none(), "not while still looking");
        ui.doom.scan = false;
        let empty = page(&ui.doom);
        assert!(empty.hero.unwrap().body.contains("Doom"));
        assert_eq!(empty.rows.len(), 2, "Controls and About still there");
        ui.doom.running = true;
        ui.doom_ended(Some("W_InitFiles: no files found".into()));
        assert!(!ui.doom.running);
        assert_eq!(
            page(&ui.doom).status.as_deref(),
            Some("Doom stopped: W_InitFiles: no files found")
        );
    }

    #[test]
    fn the_game_fills_the_width_at_one_and_a_half_times() {
        let mut v = DoomView::default();
        let mut c = Canvas::new();
        draw(&mut c, &v);
        let quads = c.finish();
        assert!(!quads.iter().any(|q| q.tex == Tex::Picture), "no frame yet");
        assert!(quads.len() > 1, "a word while it starts");
        v.frame = true;
        let mut c = Canvas::new();
        draw(&mut c, &v);
        let quads = c.finish();
        let picture = quads.iter().find(|q| q.tex == Tex::Picture).unwrap();
        assert_eq!(
            (picture.x, picture.y, picture.w, picture.h),
            (0., 30., 480., 300.)
        );
        assert_eq!(picture.w / 320., 1.5);
        assert_eq!(picture.h / 200., 1.5);
    }

    #[test]
    fn a_running_game_draws_the_frame_not_the_picker() {
        let mut ui = installed();
        let mut m = on_doom(&mut ui);
        ui.doom.scan = false;
        ui.doom.wads = vec![wad("freedoom1.wad", "Freedoom: Phase 1")];
        let picker = ui.draw(&m, &[], false);
        assert!(!picker.iter().any(|q| q.tex == Tex::Picture));
        m.navigation.focus = 0;
        ui.action(&mut m, &[], Action::Select);
        ui.doom.frame = true;
        let game = ui.draw(&m, &[], false);
        assert!(game.iter().any(|q| q.tex == Tex::Picture));
        assert!(game
            .iter()
            .all(|q| q.x >= 0. && q.y >= 0. && q.x + q.w <= 480. && q.y + q.h <= 360.));
        assert_eq!(
            crate::focus_target_count(&game),
            0,
            "no list focus over the game"
        );
        // Power during a game turns the screen off (the game pauses) rather
        // than putting the player to sleep.
        assert_eq!(
            ui.action(&mut m, &[], Action::PowerShort),
            Effect::ScreenSleep
        );
        assert_eq!(m.screen, Screen::Doom);
    }
}

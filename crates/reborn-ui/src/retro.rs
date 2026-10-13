//! Retro games (Extras > Games): the systems, each system's games on the
//! card, its controls, then the game itself, drawn from the frontend's
//! pictures, and the game menu Power opens over it. While a game is on
//! screen the app sends the buttons straight to it
//! (`reborn_core::retro::Controls`); this module only shows it.
use crate::{
    canvas::Canvas,
    components::{self, SheetRow},
    pages::{Hero, Page},
    text::Face,
    theme::color,
    Item, RowKind, Ui,
};
use reborn_core::{
    retro::{self, fit, game_layout, layout_for, Command, Launch, Layout, Rom, System},
    AppModel, Effect, Screen,
};
use reborn_graphics::{Quad, Tex};

/// A system as the list shows it (found by the app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRow {
    pub system: System,
    pub games: usize,
    /// Its emulator is installed.
    pub core: bool,
}

/// What the app should look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scan {
    Systems,
    Roms(System),
}

#[derive(Debug, Default, Clone)]
pub struct RetroView {
    /// The system has the frontend (`reborn-retro`) and at least one core;
    /// without them retro games are not offered at all.
    pub installed: bool,
    pub systems: Vec<SystemRow>,
    /// The games of the system open (set by the app).
    pub roms: Vec<Rom>,
    /// Asked the app to look.
    pub scan: Option<Scan>,
    /// The game running (the frontend process is alive).
    pub running: Option<Launch>,
    /// The size of its latest picture, resident in `Tex::Picture`.
    pub frame: Option<(u32, u32)>,
    /// Why the last game stopped on its own, in plain words.
    pub error: Option<String>,
    /// The game's last word (state saved, ...), for the game menu.
    pub status: Option<String>,
    /// A saved state exists for the running game.
    pub has_state: bool,
}

impl RetroView {
    /// The game is on screen: draw it and send it the buttons.
    pub fn playing(&self, m: &AppModel) -> bool {
        self.running.is_some() && m.screen == Screen::RetroGame
    }

    /// A game is running and one of its screens shows (the game, its
    /// menu, the controls opened from the menu).
    pub fn in_session(&self, m: &AppModel) -> bool {
        self.running.is_some()
            && match m.screen {
                Screen::RetroGame | Screen::RetroMenu => true,
                Screen::RetroControls => m
                    .navigation
                    .history
                    .iter()
                    .any(|f| f.screen == Screen::RetroGame),
                _ => false,
            }
    }

    /// The running game's controls.
    pub fn layout(&self) -> Option<&'static Layout> {
        self.running.as_ref().map(|l| layout_for(l.system, &l.rom))
    }
}

/// The Games row, when the frontend and a core are installed.
pub(crate) fn games_row(v: &RetroView) -> Option<Item> {
    v.installed.then(|| {
        Item::new("Retro games", "retro")
            .with_secondary("Classic consoles and arcade")
            .with_kind(RowKind::Chevron)
    })
}

fn count(n: usize) -> String {
    match n {
        0 => "No games yet".into(),
        1 => "1 game".into(),
        n => format!("{n} games"),
    }
}

/// The systems.
pub(crate) fn systems_page(v: &RetroView) -> Page {
    let rows = v
        .systems
        .iter()
        .map(|s| {
            let detail = if s.core {
                count(s.games)
            } else {
                "Emulator not installed".into()
            };
            Item::new(s.system.title(), format!("retro_sys:{}", s.system.key()))
                .with_secondary(detail)
                .with_kind(RowKind::Chevron)
        })
        .collect();
    Page {
        rows,
        status: Some("Games go in a Games folder on the card".into()),
        ..Default::default()
    }
}

/// One system's games, then its controls.
pub(crate) fn roms_page(v: &RetroView, m: &AppModel) -> Page {
    let Some(system) = System::from_key(&m.navigation.filter) else {
        return Page::default();
    };
    let core = v
        .systems
        .iter()
        .find(|s| s.system == system)
        .is_none_or(|s| s.core);
    let mut rows: Vec<Item> = v
        .roms
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut item = Item::new(r.title.clone(), format!("retro_play:{i}"))
                .with_secondary(r.tags.clone());
            item.enabled = core;
            item
        })
        .collect();
    rows.push(
        Item::new("Controls", format!("retro_controls:{}", system.key()))
            .with_kind(RowKind::Chevron),
    );
    let status = match (&v.error, core) {
        (Some(e), _) => format!("The game stopped: {e}"),
        (None, false) => format!("The {} emulator is not installed", system.core()),
        (None, true) => "Power opens the game menu while you play".into(),
    };
    let mut page = Page {
        rows,
        status: Some(status),
        ..Default::default()
    };
    if v.roms.is_empty() && v.scan.is_none() {
        page.hero = Some(Hero {
            title: "No games found".into(),
            body: system.where_to_put(),
        });
    }
    page
}

/// The layout a Controls page shows: filter "<system>" (the running game's
/// own when it is that system's), or "<system>:<n>" for a game in
/// [`retro::GAME_LAYOUTS`].
fn controls_layout(v: &RetroView, filter: &str) -> Option<(System, &'static Layout)> {
    let (key, game) = match filter.split_once(':') {
        Some((k, n)) => (k, n.parse::<usize>().ok()),
        None => (filter, None),
    };
    let system = System::from_key(key)?;
    if let Some(n) = game {
        return retro::GAME_LAYOUTS.get(n).map(|(_, _, _, l)| (system, *l));
    }
    match &v.running {
        Some(l) if l.system == system => Some((system, layout_for(system, &l.rom))),
        _ => Some((system, system.layout())),
    }
}

/// A layout, one row per button.
pub(crate) fn controls_page(v: &RetroView, m: &AppModel) -> Page {
    let filter = m.navigation.filter.as_str();
    let Some((system, layout)) = controls_layout(v, filter) else {
        return Page::default();
    };
    use reborn_core::PhysicalControl as C;
    let dpad = [
        (C::Back, retro::Pad::Up),
        (C::PlayPause, retro::Pad::Down),
        (C::Previous, retro::Pad::Left),
        (C::Next, retro::Pad::Right),
    ];
    let standard = |b: &retro::Binding| {
        dpad.iter()
            .any(|(c, p)| b.trigger == retro::Trigger::Held(*c) && b.pad == *p)
    };
    // The usual D-pad is one row, after the buttons.
    let whole_dpad = dpad.iter().all(|(c, p)| {
        layout
            .bindings
            .iter()
            .any(|b| b.trigger == retro::Trigger::Held(*c) && b.pad == *p)
    });
    let mut rows: Vec<Item> = vec![];
    for b in layout.bindings {
        if whole_dpad && standard(b) {
            continue;
        }
        rows.push(Item::new(b.name, "retro_none").with_secondary(retro::trigger_words(b.trigger)));
    }
    for (_, name) in layout.menu_buttons {
        rows.push(Item::new(*name, "retro_none").with_secondary("From the game menu"));
    }
    if whole_dpad {
        rows.push(
            Item::new("D-pad", "retro_none").with_secondary("Menu, Play, Previous, Next buttons"),
        );
    }
    rows.push(Item::new("Game menu", "retro_none").with_secondary("Power"));
    // The arcade page lists the games with controls of their own.
    if !filter.contains(':') {
        for (i, (s, _, name, _)) in retro::GAME_LAYOUTS.iter().enumerate() {
            if *s == system {
                rows.push(
                    Item::new(
                        format!("{name} controls"),
                        format!("retro_controls:{}:{i}", s.key()),
                    )
                    .with_kind(RowKind::Chevron),
                );
            }
        }
    }
    Page {
        rows,
        status: Some("Hold means half a second".into()),
        ..Default::default()
    }
}

/// The game menu: resume, states, the buttons the controls cannot reach,
/// controls, quit.
pub(crate) fn menu_page(v: &RetroView) -> Page {
    let Some(launch) = &v.running else {
        return Page::default();
    };
    let mut rows = vec![
        Item::new("Resume", "retro_resume"),
        Item::new("Save state", "retro_save").with_secondary(if v.has_state {
            "Replaces the saved one"
        } else {
            ""
        }),
    ];
    let mut load = Item::new("Load state", "retro_load");
    if !v.has_state {
        load = load.with_secondary("None saved yet");
        load.enabled = false;
    }
    rows.push(load);
    for (pad, name) in layout_for(launch.system, &launch.rom).menu_buttons {
        rows.push(Item::new(
            format!("Press {name}"),
            format!("retro_press:{}", *pad as u8),
        ));
    }
    rows.push(
        Item::new(
            "Controls",
            format!("retro_controls:{}", launch.system.key()),
        )
        .with_kind(RowKind::Chevron),
    );
    rows.push(Item::new("Quit game", "retro_quit"));
    Page {
        rows,
        ..Default::default()
    }
}

/// The status bar title of the retro screens.
pub(crate) fn title(v: &RetroView, m: &AppModel) -> String {
    match m.screen {
        Screen::Retro => "Retro games".into(),
        Screen::RetroRoms => System::from_key(&m.navigation.filter)
            .map_or("Retro games", |s| s.title())
            .into(),
        Screen::RetroControls => {
            let game = m
                .navigation
                .filter
                .split_once(':')
                .and_then(|(_, n)| n.parse::<usize>().ok())
                .and_then(|n| retro::GAME_LAYOUTS.get(n))
                .map(|(_, _, name, _)| *name)
                .or_else(|| {
                    v.running
                        .as_ref()
                        .and_then(|l| game_layout(l.system, &l.rom))
                        .map(|(name, _)| name)
                });
            match game {
                Some(name) => format!("{name} controls"),
                None => "Controls".into(),
            }
        }
        _ => v
            .running
            .as_ref()
            .map_or("Retro games".into(), |l| l.title.clone()),
    }
}

impl Ui {
    /// Row keys owned by retro games.
    pub(crate) fn retro_key(&mut self, m: &mut AppModel, key: &str) -> Option<Effect> {
        let v = &mut self.retro;
        match key {
            "retro" => {
                Self::go(m, Screen::Retro, "");
                v.scan = Some(Scan::Systems);
                return Some(Effect::None);
            }
            "retro_none" => return Some(Effect::None),
            "retro_resume" => {
                if m.screen == Screen::RetroMenu {
                    Self::back(m);
                }
                return Some(Effect::None);
            }
            "retro_save" => {
                v.status = Some("Saving…".into());
                return Some(Effect::RetroCommand(Command::SaveState));
            }
            "retro_load" => {
                if m.screen == Screen::RetroMenu {
                    Self::back(m);
                }
                return Some(Effect::RetroCommand(Command::LoadState));
            }
            "retro_quit" => {
                v.running = None;
                v.frame = None;
                Self::leave_game(m);
                return Some(Effect::RetroStop);
            }
            _ => {}
        }
        if let Some(k) = key.strip_prefix("retro_sys:") {
            let system = System::from_key(k)?;
            Self::go(m, Screen::RetroRoms, k);
            v.roms.clear();
            v.scan = Some(Scan::Roms(system));
            v.error = None;
            return Some(Effect::None);
        }
        if let Some(f) = key.strip_prefix("retro_controls:") {
            Self::go(m, Screen::RetroControls, f);
            return Some(Effect::None);
        }
        if let Some(id) = key.strip_prefix("retro_press:") {
            let pad = retro::Pad::from_id(id.parse().ok()?)?;
            if m.screen == Screen::RetroMenu {
                Self::back(m);
            }
            return Some(Effect::RetroCommand(Command::Tap(pad)));
        }
        let i: usize = key.strip_prefix("retro_play:")?.parse().ok()?;
        if v.running.is_some() {
            return Some(Effect::None);
        }
        let rom = v.roms.get(i)?;
        let launch = Launch {
            system: rom.system,
            rom: rom.path.clone(),
            title: rom.title.clone(),
        };
        v.running = Some(launch.clone());
        v.frame = None;
        v.error = None;
        v.status = None;
        Self::go(m, Screen::RetroGame, rom.system.key());
        Some(Effect::RetroStart(launch))
    }

    /// Back down to the game list from the game's screens.
    fn leave_game(m: &mut AppModel) {
        let in_game = m.screen == Screen::RetroGame
            || m.navigation
                .history
                .iter()
                .any(|f| f.screen == Screen::RetroGame);
        if !in_game {
            return;
        }
        while m.screen != Screen::RetroRoms && !m.navigation.history.is_empty() {
            Self::back(m);
        }
    }

    /// Power during play, or the screen going off: the game menu.
    pub fn retro_open_menu(&mut self, m: &mut AppModel) {
        if self.retro.playing(m) {
            self.retro.status = None;
            Self::go(m, Screen::RetroMenu, "");
        }
    }

    /// The frontend stopped by itself (a failure, or the core shut down):
    /// back to the game list, saying why when it failed.
    pub fn retro_ended(&mut self, m: &mut AppModel, error: Option<String>) {
        let was = self.retro.running.take().is_some();
        self.retro.frame = None;
        self.retro.error = error;
        if was {
            Self::leave_game(m);
        }
    }

    /// Power during a game: the menu (the game pauses), or with the menu
    /// already open, the screen goes off. `None` when no game is running.
    pub(crate) fn retro_power(&mut self, m: &mut AppModel) -> Option<Effect> {
        if self.retro.playing(m) && !m.screen_off {
            self.retro_open_menu(m);
            return Some(Effect::None);
        }
        self.retro.in_session(m).then_some(Effect::ScreenSleep)
    }
}

fn picture(c: &mut Canvas, v: &RetroView) {
    c.rect(0., 0., 480., 360., color::BLACK);
    match v.frame {
        Some((w, h)) => {
            let f = fit(w, h);
            c.draw.push(Quad {
                tex: Tex::Picture,
                ..Quad::rect(f.x, f.y, f.w, f.h, 0xFFFFFFFF)
            });
        }
        None => c.text_mid_center(240., 180., "Starting the game…", Face::R15, color::MUTED),
    }
}

/// The game, full screen.
pub(crate) fn draw_game(c: &mut Canvas, v: &RetroView) {
    picture(c, v);
}

/// The game menu over the paused picture.
pub(crate) fn draw_menu(c: &mut Canvas, ui: &Ui, m: &AppModel) {
    let v = &ui.retro;
    picture(c, v);
    let page = menu_page(v);
    let rows: Vec<SheetRow> = page
        .rows
        .iter()
        .map(|r| SheetRow {
            label: &r.label,
            detail: &r.secondary,
            enabled: r.enabled,
        })
        .collect();
    let subtitle = v
        .status
        .clone()
        .unwrap_or_else(|| "Paused. Volume buttons change the sound level".into());
    let focus = m.navigation.focus.min(rows.len().saturating_sub(1));
    components::action_sheet(c, &title(v, m), &subtitle, &rows, focus);
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::{
        retro::{Command, Pad},
        Action,
    };
    use std::path::PathBuf;

    fn rom(system: System, file: &str) -> Rom {
        retro::rom(system, PathBuf::from(format!("/sd/Games/X/{file}"))).unwrap()
    }

    fn select_key(ui: &mut Ui, m: &mut AppModel, key: &str) -> Effect {
        let rows = crate::pages::page(ui, m, &[]).rows;
        m.navigation.focus = rows
            .iter()
            .position(|r| r.key == key)
            .unwrap_or_else(|| panic!("a {key} row in {:?}", rows));
        ui.action(m, &[], Action::Select)
    }

    fn at_roms(ui: &mut Ui) -> AppModel {
        let mut m = AppModel {
            screen: Screen::Games,
            ..Default::default()
        };
        assert!(
            !crate::pages::page(ui, &m, &[])
                .rows
                .iter()
                .any(|r| r.key == "retro"),
            "not offered without the frontend and a core"
        );
        ui.retro.installed = true;
        assert_eq!(select_key(ui, &mut m, "retro"), Effect::None);
        assert_eq!(m.screen, Screen::Retro);
        assert_eq!(ui.retro.scan, Some(Scan::Systems));
        ui.retro.scan = None;
        ui.retro.systems = System::ALL
            .into_iter()
            .map(|s| SystemRow {
                system: s,
                games: 2,
                core: s != System::Arcade,
            })
            .collect();
        let rows = crate::pages::page(ui, &m, &[]).rows;
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].secondary, "2 games");
        assert_eq!(rows[5].secondary, "Emulator not installed");
        select_key(ui, &mut m, "retro_sys:nes");
        assert_eq!(
            (m.screen, m.navigation.filter.as_str()),
            (Screen::RetroRoms, "nes")
        );
        assert_eq!(ui.retro.scan, Some(Scan::Roms(System::Nes)));
        ui.retro.scan = None;
        ui.retro.roms = vec![
            rom(System::Nes, "Alpha (USA).nes"),
            rom(System::Nes, "beta.nes"),
        ];
        m
    }

    #[test]
    fn games_to_a_game_and_back_through_the_menu() {
        let mut ui = Ui::default();
        let mut m = at_roms(&mut ui);
        let page = crate::pages::page(&ui, &m, &[]);
        let labels: Vec<&str> = page.rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Alpha", "beta", "Controls"]);
        assert_eq!(page.rows[0].secondary, "USA");
        let effect = select_key(&mut ui, &mut m, "retro_play:0");
        let Effect::RetroStart(launch) = effect else {
            panic!("{effect:?}")
        };
        assert_eq!(launch.system, System::Nes);
        assert_eq!(launch.title, "Alpha");
        assert_eq!(m.screen, Screen::RetroGame);
        assert!(ui.retro.playing(&m) && ui.retro.in_session(&m));
        // Power: the menu.
        assert_eq!(ui.action(&mut m, &[], Action::PowerShort), Effect::None);
        assert_eq!(m.screen, Screen::RetroMenu);
        assert!(!ui.retro.playing(&m) && ui.retro.in_session(&m));
        let labels: Vec<String> = crate::pages::page(&ui, &m, &[])
            .rows
            .iter()
            .map(|r| r.label.clone())
            .collect();
        assert_eq!(
            labels,
            [
                "Resume",
                "Save state",
                "Load state",
                "Controls",
                "Quit game"
            ]
        );
        // Power again with the menu open: the screen goes off.
        assert_eq!(
            ui.action(&mut m, &[], Action::PowerShort),
            Effect::ScreenSleep
        );
        // Save stays in the menu; Back resumes.
        assert_eq!(
            select_key(&mut ui, &mut m, "retro_save"),
            Effect::RetroCommand(Command::SaveState)
        );
        assert_eq!(m.screen, Screen::RetroMenu);
        ui.action(&mut m, &[], Action::Back);
        assert_eq!(m.screen, Screen::RetroGame);
        // Controls from the menu keep the game.
        ui.retro_open_menu(&mut m);
        select_key(&mut ui, &mut m, "retro_controls:nes");
        assert_eq!(m.screen, Screen::RetroControls);
        assert!(ui.retro.in_session(&m));
        ui.action(&mut m, &[], Action::Back);
        assert_eq!(m.screen, Screen::RetroMenu);
        // Quit: back to the list, the frontend stops.
        assert_eq!(select_key(&mut ui, &mut m, "retro_quit"), Effect::RetroStop);
        assert_eq!(m.screen, Screen::RetroRoms);
        assert!(ui.retro.running.is_none());
    }

    #[test]
    fn a_failed_game_says_why_on_the_list() {
        let mut ui = Ui::default();
        let mut m = at_roms(&mut ui);
        select_key(&mut ui, &mut m, "retro_play:1");
        ui.retro_open_menu(&mut m);
        ui.retro_ended(&mut m, Some("the core could not load this game".into()));
        assert_eq!(m.screen, Screen::RetroRoms);
        assert_eq!(
            crate::pages::page(&ui, &m, &[]).status.as_deref(),
            Some("The game stopped: the core could not load this game")
        );
        // An empty system says where games go.
        ui.retro.roms.clear();
        let page = crate::pages::page(&ui, &m, &[]);
        assert!(page.hero.unwrap().body.contains("Games/NES"));
        assert_eq!(page.rows.len(), 1, "Controls still there");
    }

    #[test]
    fn menu_rows_press_buttons_the_controls_lack_and_load_needs_a_state() {
        let mut ui = Ui::default();
        let mut m = AppModel::default();
        ui.retro.running = Some(Launch {
            system: System::Snes,
            rom: PathBuf::from("/sd/Games/SNES/x.sfc"),
            title: "X".into(),
        });
        m.screen = Screen::RetroGame;
        ui.retro_open_menu(&mut m);
        let rows = crate::pages::page(&ui, &m, &[]).rows;
        assert!(!rows[2].enabled, "no state yet");
        assert_eq!(rows[3].label, "Press Select");
        ui.retro.has_state = true;
        assert_eq!(
            select_key(&mut ui, &mut m, "retro_load"),
            Effect::RetroCommand(Command::LoadState)
        );
        assert_eq!(m.screen, Screen::RetroGame, "loading resumes");
        ui.retro_open_menu(&mut m);
        assert_eq!(
            select_key(
                &mut ui,
                &mut m,
                &format!("retro_press:{}", Pad::Select as u8)
            ),
            Effect::RetroCommand(Command::Tap(Pad::Select))
        );
        assert_eq!(m.screen, Screen::RetroGame);
    }

    #[test]
    fn controls_pages_list_every_button_and_the_games_with_their_own() {
        let ui = Ui::default();
        let mut m = AppModel {
            screen: Screen::RetroControls,
            ..Default::default()
        };
        m.navigation.filter = "nes".into();
        let rows = crate::pages::page(&ui, &m, &[]).rows;
        let pairs: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.label.as_str(), r.secondary.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("A", "Volume up"),
                ("B", "Volume down"),
                ("Start", "Tap centre button"),
                ("Select", "Hold centre button"),
                ("D-pad", "Menu, Play, Previous, Next buttons"),
                ("Game menu", "Power"),
            ]
        );
        m.navigation.filter = "arcade".into();
        let rows = crate::pages::page(&ui, &m, &[]).rows;
        assert!(rows.iter().any(|r| r.label == "Asteroids controls"));
        m.navigation.filter = "arcade:0".into();
        let rows = crate::pages::page(&ui, &m, &[]).rows;
        let turns: Vec<&str> = rows
            .iter()
            .filter(|r| r.label == "Turn left")
            .map(|r| r.secondary.as_str())
            .collect();
        assert_eq!(turns, ["Turn the ring anticlockwise", "Previous (left)"]);
        assert_eq!(title(&ui.retro, &m), "Asteroids controls");
        m.navigation.filter = "snes".into();
        let rows = crate::pages::page(&ui, &m, &[]).rows;
        assert!(rows
            .iter()
            .any(|r| r.label == "Select" && r.secondary == "From the game menu"));
    }

    #[test]
    fn the_game_is_drawn_at_its_fit_and_the_menu_over_it() {
        let mut ui = Ui::default();
        let mut m = AppModel::default();
        ui.retro.running = Some(Launch {
            system: System::Gba,
            rom: PathBuf::from("/sd/Games/GBA/x.gba"),
            title: "X".into(),
        });
        m.screen = Screen::RetroGame;
        let starting = ui.draw(&m, &[], false);
        assert!(!starting.iter().any(|q| q.tex == Tex::Picture));
        ui.retro.frame = Some((240, 160));
        let game = ui.draw(&m, &[], false);
        let p = game.iter().find(|q| q.tex == Tex::Picture).unwrap();
        assert_eq!((p.x, p.y, p.w, p.h), (0., 20., 480., 320.));
        ui.retro_open_menu(&mut m);
        let menu = ui.draw(&m, &[], false);
        assert!(menu.iter().any(|q| q.tex == Tex::Picture), "paused picture");
        assert_eq!(crate::focus_target_count(&menu), 1);
    }
}

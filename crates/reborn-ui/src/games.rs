//! Extras > Games: two arcade games played with the wheel and the buttons,
//! Shatterfield (`rocks`) and Pellet Rush (`maze`). Each game is plain state
//! advanced in fixed steps (`Game::advance`), so the rules are deterministic
//! for a seed and testable. Both are played with buttons held and released,
//! so the app offers them the raw input events first (`Ui::game_input`).
use crate::{
    canvas::Canvas,
    maze::{self, Maze},
    rocks::{self, Rocks},
    screens::screen_title,
    text::{self, Face},
    theme::{alpha, color, WIDTH},
    Item, RowKind, Ui,
};
use reborn_core::{Action, AppModel, Effect, GameBest, NormalizedInput, PhysicalControl, Screen};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Shatterfield,
    PelletRush,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Shatterfield, Kind::PelletRush];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Shatterfield => "shatterfield",
            Kind::PelletRush => "pellet_rush",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Shatterfield => "Shatterfield",
            Kind::PelletRush => "Pellet Rush",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Kind::Shatterfield => "Spin to aim and blast rocks",
            Kind::PelletRush => "Steer with the four buttons",
        }
    }

    pub fn best(self, best: &GameBest) -> u32 {
        match self {
            Kind::Shatterfield => best.shatterfield,
            Kind::PelletRush => best.pellet_rush,
        }
    }

    fn best_mut(self, best: &mut GameBest) -> &mut u32 {
        match self {
            Kind::Shatterfield => &mut best.shatterfield,
            Kind::PelletRush => &mut best.pellet_rush,
        }
    }
}

/// What a game made of one raw button or wheel event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Not the game's: it goes on to the usual handling.
    No,
    Yes,
    /// The player held Back to leave.
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The start card, waiting for Select.
    Ready,
    Playing,
    Paused,
    Over,
}

/// The score line below the status bar.
const HUD_Y: f32 = 52.;
const HUD_LEFT: f32 = 16.;
const HUD_RIGHT: f32 = 464.;
/// Logic runs at 60 steps a second whatever the frame rate.
const STEP_MS: f32 = 1000. / 60.;
/// Longest frame gap simulated at once (a stall must not teleport anything).
const MAX_FRAME_MS: f32 = 100.;

pub struct Game {
    pub kind: Kind,
    pub phase: Phase,
    pub score: u32,
    play: Play,
    rng: u64,
    clock_ms: f32,
}

enum Play {
    Shatterfield(Box<Rocks>),
    PelletRush(Box<Maze>),
}

impl Game {
    pub fn new(kind: Kind, seed: u64) -> Self {
        let mut rng = seed | 1;
        let play = match kind {
            Kind::Shatterfield => Play::Shatterfield(Box::new(Rocks::new(&mut rng))),
            Kind::PelletRush => Play::PelletRush(Box::default()),
        };
        Self {
            kind,
            phase: Phase::Ready,
            score: 0,
            play,
            rng,
            clock_ms: 0.,
        }
    }

    /// Select: start, pause, resume, or a new game after one ends.
    pub fn select(&mut self) {
        self.phase = match self.phase {
            Phase::Ready => Phase::Playing,
            Phase::Playing => Phase::Paused,
            Phase::Paused => Phase::Playing,
            Phase::Over => {
                *self = Game::new(self.kind, next(&mut self.rng));
                Phase::Ready
            }
        };
        self.clock_ms = 0.;
    }

    pub fn pause(&mut self) {
        if self.phase == Phase::Playing {
            self.phase = Phase::Paused;
        }
        if let Play::Shatterfield(r) = &mut self.play {
            r.release();
        }
    }

    /// One raw button or wheel event. Presses only count in play; the rest
    /// of a press the game took (release, long press) comes here too.
    ///
    /// Shatterfield: the wheel turns, Volume Up thrusts while held, Volume
    /// Down fires (held, it repeats), Select jumps to hyperspace, Play/Pause
    /// pauses and resumes. Pellet Rush: Back, Play/Pause, Previous and Next
    /// steer up, down, left and right (a turn waits for its opening), Select
    /// pauses and holding Back leaves.
    pub fn input(&mut self, input: NormalizedInput) -> Taken {
        use NormalizedInput as N;
        use PhysicalControl as C;
        let playing = self.phase == Phase::Playing;
        let mut pause = false;
        let taken = match &mut self.play {
            Play::Shatterfield(r) => match input {
                N::WheelClockwise(n) | N::WheelCounterClockwise(n)
                    if matches!(self.phase, Phase::Playing | Phase::Ready) =>
                {
                    let n = n.min(8) as i8;
                    r.turn(if matches!(input, N::WheelClockwise(_)) {
                        n
                    } else {
                        -n
                    });
                    Taken::Yes
                }
                N::Press(C::VolumeUp) if playing => {
                    r.set_thrust(true);
                    Taken::Yes
                }
                N::Press(C::VolumeDown) if playing => {
                    r.set_fire(true);
                    Taken::Yes
                }
                N::Release(C::VolumeUp) | N::Cancel(C::VolumeUp) => {
                    r.set_thrust(false);
                    Taken::Yes
                }
                N::Release(C::VolumeDown) | N::Cancel(C::VolumeDown) => {
                    r.set_fire(false);
                    Taken::Yes
                }
                N::Press(C::Select) if playing => {
                    r.hyperspace();
                    Taken::Yes
                }
                N::Press(C::PlayPause) if playing => {
                    pause = true;
                    Taken::Yes
                }
                N::Press(C::PlayPause) if self.phase == Phase::Paused => {
                    self.phase = Phase::Playing;
                    self.clock_ms = 0.;
                    Taken::Yes
                }
                // Skipping tracks mid-game would only surprise.
                N::Press(C::Previous | C::Next) if playing => Taken::Yes,
                _ => Taken::No,
            },
            Play::PelletRush(z) => {
                let steer = |c| match c {
                    C::Back => Some(maze::Dir::Up),
                    C::PlayPause => Some(maze::Dir::Down),
                    C::Previous => Some(maze::Dir::Left),
                    C::Next => Some(maze::Dir::Right),
                    _ => None,
                };
                match input {
                    N::Press(c) if playing => match steer(c) {
                        Some(dir) => {
                            z.steer(dir, true);
                            Taken::Yes
                        }
                        None if c == C::Select => {
                            pause = true;
                            Taken::Yes
                        }
                        None => Taken::No,
                    },
                    N::Release(c) | N::Cancel(c) => match steer(c) {
                        Some(dir) => {
                            z.steer(dir, false);
                            Taken::Yes
                        }
                        None => Taken::No,
                    },
                    N::LongPress(C::Back) => Taken::Leave,
                    _ => Taken::No,
                }
            }
        };
        if pause || taken == Taken::Leave {
            self.pause();
        }
        taken
    }

    /// Advances the game by `ms` of wall time; true when the picture changed.
    pub fn advance(&mut self, ms: f32) -> bool {
        if self.phase != Phase::Playing {
            return false;
        }
        self.clock_ms += ms.clamp(0., MAX_FRAME_MS);
        while self.clock_ms >= STEP_MS {
            self.clock_ms -= STEP_MS;
            let over = match &mut self.play {
                Play::Shatterfield(r) => {
                    r.step(STEP_MS / 1000., &mut self.rng);
                    self.score = r.score;
                    r.over()
                }
                Play::PelletRush(z) => {
                    z.step(STEP_MS / 1000., &mut self.rng);
                    self.score = z.score;
                    z.over()
                }
            };
            if over {
                self.phase = Phase::Over;
                break;
            }
        }
        true
    }
}

/// A uniform number in 0..1.
pub(crate) fn rand01(rng: &mut u64) -> f32 {
    (next(rng) >> 40) as f32 / (1u64 << 24) as f32
}

pub(crate) fn next(rng: &mut u64) -> u64 {
    // xorshift64
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    *rng
}

/// Extras > Games: each game with its best score, or Paused for one left
/// mid-play.
pub(crate) fn rows(ui: &Ui, m: &AppModel) -> Vec<Item> {
    Kind::ALL
        .into_iter()
        .map(|kind| {
            let best = kind.best(&m.game_best);
            let paused = ui
                .game
                .as_ref()
                .is_some_and(|g| g.kind == kind && g.phase == Phase::Paused);
            let detail = if paused {
                "Paused".to_owned()
            } else if best > 0 {
                format!("Best {best}")
            } else {
                kind.hint().to_owned()
            };
            Item::new(kind.title(), format!("game:{}", kind.key()))
                .with_secondary(detail)
                .with_kind(RowKind::Chevron)
        })
        .collect()
}

impl Ui {
    /// Opens a game from Extras > Games: the one left mid-play resumes (paused),
    /// anything else starts afresh.
    pub(crate) fn open_game(&mut self, m: &mut AppModel, kind: Kind) {
        let resume = self
            .game
            .as_ref()
            .is_some_and(|g| g.kind == kind && g.phase != Phase::Over);
        if !resume {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as u64);
            self.game = Some(Game::new(kind, seed));
        }
        self.game_clock = None;
        Self::go(m, Screen::Game, kind.key());
    }

    /// Actions on the game screen: Select starts, pauses and resumes; the
    /// wheel and the menu button do nothing else here. `None` lets the action
    /// continue (Back, Home), pausing the game first.
    pub(crate) fn game_action(&mut self, action: Action) -> Option<Effect> {
        let game = self.game.as_mut()?;
        match action {
            Action::Select => game.select(),
            Action::WheelClockwise(_)
            | Action::WheelCounterClockwise(_)
            | Action::NavigateUp
            | Action::NavigateDown
            | Action::NavigateLeft
            | Action::NavigateRight
            | Action::ContextMenu => {}
            _ => {
                game.pause();
                return None;
            }
        }
        Some(Effect::None)
    }

    /// One raw input event, before the usual button handling: true when the
    /// game took it. Presses and the wheel count while the game is on
    /// screen; the rest of a press it took is always its own, so a release
    /// never acts on the screen behind after the game is left.
    pub fn game_input(&mut self, m: &mut AppModel, input: NormalizedInput) -> bool {
        use NormalizedInput as N;
        let showing = m.screen == Screen::Game && !m.screen_off && m.navigation.modal.is_none();
        let held = match input {
            N::Release(c) | N::Cancel(c) | N::Repeat(c) | N::LongPress(c) => {
                if !self.game_held.contains(&c) {
                    return false;
                }
                if matches!(input, N::Release(_) | N::Cancel(_)) {
                    self.game_held.retain(|&h| h != c);
                }
                true
            }
            N::Press(_) | N::WheelClockwise(_) | N::WheelCounterClockwise(_) => false,
        };
        let Some(game) = self.game.as_mut() else {
            return held;
        };
        if !showing && !held {
            return false;
        }
        match game.input(input) {
            Taken::No => held,
            Taken::Yes => {
                if let N::Press(c) = input {
                    self.game_held.push(c);
                }
                true
            }
            Taken::Leave => {
                if showing {
                    Self::back(m);
                }
                true
            }
        }
    }

    /// Advances the game while its screen shows and pauses it otherwise
    /// (another screen, a sheet, the screen off). Records a new best score.
    /// True when a new frame is due; called every main-loop pass.
    pub fn tick_games(&mut self, m: &mut AppModel, now: Instant) -> bool {
        let Some(game) = self.game.as_mut() else {
            return false;
        };
        if m.screen != Screen::Game || m.screen_off || m.navigation.modal.is_some() {
            game.pause();
            self.game_clock = None;
            return false;
        }
        let elapsed = self.game_clock.map_or(0., |t| {
            now.saturating_duration_since(t).as_secs_f32() * 1000.
        });
        self.game_clock = Some(now);
        let live = game.phase != Phase::Over;
        let frame = game.advance(elapsed);
        let best = game.kind.best_mut(&mut m.game_best);
        if live && game.phase == Phase::Over && game.score > *best {
            *best = game.score;
        }
        frame
    }

    /// A game in play asks for frames at the panel's rate.
    pub(crate) fn game_running(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|g| g.phase == Phase::Playing)
    }
}

/// The game screen: status bar, score line, field and any phase card.
pub(crate) fn draw(c: &mut Canvas, ui: &Ui, m: &AppModel) {
    crate::components::status_bar(c, ui, m, &screen_title(m).to_uppercase(), true);
    let Some(g) = ui.game.as_ref() else {
        return;
    };
    let accent = ui.accent();
    let best = g.kind.best(&m.game_best);
    c.text_mid(
        HUD_LEFT,
        HUD_Y,
        140.,
        &format!("Score {}", g.score),
        Face::S13,
        color::TEXT,
    );
    c.text_mid_right(
        HUD_RIGHT,
        HUD_Y,
        &format!("Best {}", best.max(g.score)),
        Face::R11,
        color::MUTED,
    );
    let (status, mid) = match &g.play {
        Play::Shatterfield(r) => {
            rocks::draw(c, r, accent);
            (rocks::status(r), rocks::FIELD_Y + rocks::H / 2.)
        }
        Play::PelletRush(z) => {
            maze::draw(c, z, accent);
            (
                maze::status(z),
                maze::TOP + maze::ROWS as f32 * maze::TILE / 2.,
            )
        }
    };
    c.text_mid_center(WIDTH / 2., HUD_Y, &status, Face::R11, color::DIM);
    // The start card explains the buttons.
    let notes: &[&str] = match (&g.play, g.phase) {
        (Play::Shatterfield(_), Phase::Ready) => &[
            "Wheel aims · Vol + thrusts · Vol - fires",
            "Select jumps to hyperspace · Play pauses",
        ],
        (Play::PelletRush(_), Phase::Ready) => &[
            "Back up · Play down · Previous left · Next right",
            "Select pauses · hold Back to leave",
        ],
        _ => &[],
    };
    let (head, hint) = match g.phase {
        Phase::Playing => return,
        Phase::Ready => (g.kind.title().to_owned(), "Select to start"),
        Phase::Paused => (
            "Paused".to_owned(),
            match g.kind {
                Kind::Shatterfield => "Play or Select to resume",
                Kind::PelletRush => "Select to resume",
            },
        ),
        Phase::Over => (format!("Game over · {}", g.score), "Select to play again"),
    };
    let line_h = 17.;
    let w = if notes.is_empty() { 300. } else { 340. };
    let h = 78. + line_h * notes.len() as f32;
    let x = (WIDTH - w) / 2.;
    let top = mid - h / 2.;
    c.rrect(x, top, w, h, 16., color::SHEET);
    c.rrect_outline(x, top, w, h, 16., 1., alpha(accent, 90));
    c.text_mid_center(
        WIDTH / 2.,
        top + 26.,
        &text::fit(Face::S17, &head, w - 24.),
        Face::S17,
        color::TEXT,
    );
    c.text_mid_center(WIDTH / 2., top + 54., hint, Face::R13, color::MUTED);
    for (i, note) in notes.iter().enumerate() {
        c.text_mid_center(
            WIDTH / 2.,
            top + 78. + line_h * i as f32,
            &text::fit(Face::R11, note, w - 24.),
            Face::R11,
            color::DIM,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use NormalizedInput as N;
    use PhysicalControl as C;

    fn playing(kind: Kind) -> Game {
        let mut g = Game::new(kind, 7);
        g.select();
        assert_eq!(g.phase, Phase::Playing);
        g
    }

    #[test]
    fn games_run_the_same_at_any_frame_rate() {
        for kind in Kind::ALL {
            let play = |frame_ms: f32| {
                let mut g = playing(kind);
                g.input(N::Press(C::VolumeDown));
                g.input(N::Press(C::Previous));
                for _ in 0..(3000. / frame_ms).round() as usize {
                    g.advance(frame_ms);
                }
                let snapshot = match &g.play {
                    Play::Shatterfield(r) => format!("{:?}", (r.lives, r.fingerprint())),
                    Play::PelletRush(z) => format!("{:?}", (z.lives, z.fingerprint())),
                };
                (g.score, snapshot)
            };
            // 3 s at 60 fps, 30 fps and uneven frames: the same 180 steps.
            assert_eq!(play(1000. / 60.), play(1000. / 30.), "{kind:?}");
            assert_eq!(play(1000. / 60.), play(25.), "{kind:?}");
        }
    }

    #[test]
    fn the_same_seed_plays_the_same_game() {
        for kind in Kind::ALL {
            let play = |seed: u64| {
                let mut g = Game::new(kind, seed);
                g.select();
                g.input(N::Press(C::VolumeDown));
                g.input(N::Press(C::Next));
                for k in 0..300 {
                    if k % 40 == 0 {
                        g.input(N::WheelClockwise(3));
                    }
                    g.advance(STEP_MS);
                }
                (g.score, g.phase)
            };
            assert_eq!(play(11), play(11), "{kind:?}");
        }
    }

    #[test]
    fn shatterfield_takes_held_volume_keys_only_in_play() {
        let mut g = Game::new(Kind::Shatterfield, 4);
        assert_eq!(
            g.input(N::Press(C::VolumeUp)),
            Taken::No,
            "volume before play"
        );
        assert_eq!(
            g.input(N::WheelClockwise(1)),
            Taken::Yes,
            "aim on the start card"
        );
        g.select();
        assert_eq!(g.input(N::Press(C::VolumeUp)), Taken::Yes);
        let Play::Shatterfield(r) = &g.play else {
            unreachable!()
        };
        assert!(r.thrust);
        assert_eq!(g.input(N::Release(C::VolumeUp)), Taken::Yes);
        let Play::Shatterfield(r) = &g.play else {
            unreachable!()
        };
        assert!(!r.thrust);
        assert_eq!(
            g.input(N::Press(C::Back)),
            Taken::No,
            "Back leaves as usual"
        );
        assert_eq!(g.input(N::Press(C::Power)), Taken::No);
        g.input(N::Press(C::VolumeDown));
        assert_eq!(g.input(N::Press(C::PlayPause)), Taken::Yes);
        assert_eq!(g.phase, Phase::Paused);
        let Play::Shatterfield(r) = &g.play else {
            unreachable!()
        };
        assert!(!r.fire, "pausing lets go");
        assert_eq!(g.input(N::Press(C::PlayPause)), Taken::Yes);
        assert_eq!(g.phase, Phase::Playing);
    }

    #[test]
    fn pellet_rush_steers_with_the_compass_and_held_back_leaves() {
        let mut g = Game::new(Kind::PelletRush, 4);
        assert_eq!(g.input(N::Press(C::Next)), Taken::No, "not before play");
        g.select();
        assert_eq!(
            g.input(N::Press(C::PlayPause)),
            Taken::Yes,
            "down, not music"
        );
        assert_eq!(g.input(N::Press(C::Back)), Taken::Yes, "up");
        assert_eq!(g.input(N::LongPress(C::Back)), Taken::Leave);
        assert_eq!(g.phase, Phase::Paused);
        assert_eq!(
            g.input(N::Press(C::VolumeUp)),
            Taken::No,
            "volume stays volume"
        );
        assert_eq!(
            g.input(N::WheelClockwise(1)),
            Taken::No,
            "the wheel does not steer"
        );
        g.select();
        assert_eq!(g.input(N::Press(C::Select)), Taken::Yes);
        assert_eq!(g.phase, Phase::Paused, "Select pauses");
    }

    #[test]
    fn paused_games_do_not_advance_and_a_finished_one_starts_over() {
        let mut g = playing(Kind::PelletRush);
        g.advance(500.);
        let score = g.score;
        g.select();
        assert_eq!(g.phase, Phase::Paused);
        assert!(!g.advance(1000.));
        assert_eq!(g.score, score);
        g.phase = Phase::Over;
        g.select();
        assert_eq!(g.phase, Phase::Ready);
        assert_eq!(g.score, 0);
    }

    #[test]
    fn game_screens_draw_inside_the_display() {
        for kind in Kind::ALL {
            for phase in [Phase::Ready, Phase::Playing, Phase::Paused, Phase::Over] {
                let mut g = Game::new(kind, 9);
                g.phase = phase;
                let ui = Ui {
                    game: Some(g),
                    ..Default::default()
                };
                let m = AppModel {
                    screen: Screen::Game,
                    navigation: reborn_core::NavigationState {
                        filter: kind.key().into(),
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut c = Canvas::new();
                draw(&mut c, &ui, &m);
                for q in c.finish() {
                    assert!(q.x >= 0. && q.y >= 0., "{kind:?} {phase:?} {q:?}");
                    assert!(
                        q.x + q.w <= 480. && q.y + q.h <= 360.,
                        "{kind:?} {phase:?} {q:?}"
                    );
                }
            }
        }
    }
}

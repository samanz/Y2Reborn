//! Pellet Rush (Extras > Games): a maze chase steered with the four compass
//! buttons. Plain state advanced in fixed steps by `games::Game`, so the
//! rules are deterministic and testable.
//!
//! Four chasers, each with a temper of its own: the tracker follows the
//! player's path a few tiles behind and closes in when near, the cutter
//! makes for the next crossing ahead of the player, the guard keeps to the
//! power pellet nearest the player, and the pouncer wanders until it sees
//! the player down a corridor, then charges, faster, to where it saw them.
//! Each hunts for 14 seconds then wanders for a few (fewer on later
//! levels) on its own staggered rhythm, so they never all ease off at
//! once. A power pellet frightens them; eaten, their eyes go home.
//!
//! Pellets are 5 and power pellets 25; caught chasers 100, 200, 300 and
//! 400 in one fright; bonus items 250 times the level. A cleared maze is
//! worth 100 times the level and gives a life back (five at most).
//!
//! The maze is our own: 33 x 21 tiles, mirrored left to right, with a
//! tunnel through the middle row and the chasers' house in the centre.
use crate::{
    canvas::Canvas,
    games::next,
    text::Face,
    theme::{alpha, color, mix},
};

pub const COLS: i32 = 33;
pub const ROWS: i32 = 21;
pub const TILE: f32 = 13.;
pub const LEFT: f32 = 26.;
pub const TOP: f32 = 70.;
const WIDTH_PX: f32 = COLS as f32 * TILE;
const HEIGHT_PX: f32 = ROWS as f32 * TILE;

/// The left half of the maze and its middle column; the right half mirrors
/// it. `#` wall, `.` pellet, `o` power pellet, `-` the chasers' door, `H`
/// their house, space an empty corridor (the tunnel, round the house).
const HALF: [&str; ROWS as usize] = [
    "#################",
    "#o......#........",
    "#.#####.#.######.",
    "#.#####.#.######.",
    "#................",
    "#.##.#.#####.####",
    "#.##.#.#####.####",
    "#....#......     ",
    "####.####.## ###-",
    "####.####.## #HHH",
    "    ........ #HHH",
    "##.######.## #HHH",
    "##.######.## ####",
    "#...........     ",
    "#.#####.####.###.",
    "#.#####.####.###.",
    "#o......#........",
    "#.##.####.####.##",
    "#.##.####.####.##",
    "#................",
    "#################",
];

/// The row that runs off both sides and comes back on the other.
const TUNNEL_ROW: i32 = 10;
/// Where the player starts and bonus items appear.
const START: (i32, i32) = (16, 13);
/// The tile just outside the door: where chasers leave to and eyes return.
const DOOR_OUT: (i32, i32) = (16, 7);
/// The middle of the house.
const HOUSE_Y: f32 = 10.;

/// Tiles per second at full speed; the player runs at 80% of it on level 1.
const BASE_SPEED: f32 = 9.5;
/// Seconds each chaser hunts before it wanders for a while, and where in
/// that rhythm each one starts.
const HUNT_S: f32 = 14.;
const STAGGER_S: [f32; 4] = [0., 3.5, 7., 10.5];
/// Seconds after a life starts before each chaser leaves the house.
const RELEASE_S: [f32; 4] = [0., 2.5, 5., 7.5];
/// A tapped direction waits this long for an opening (a held one waits
/// while held).
const WANT_S: f32 = 0.8;
/// Pellets eaten (in a level) when a bonus item appears, and for how long.
const FRUIT_AT: [u32; 2] = [60, 150];
const FRUIT_S: f32 = 8.;
const PELLET_POINTS: u32 = 5;
const POWER_POINTS: u32 = 25;
/// The nth chaser caught in one fright is worth n times this.
const CHASER_POINTS: u32 = 100;
/// Bonus items and a cleared maze, times the level (bonus items up to 10).
const BONUS_POINTS: u32 = 250;
const CLEAR_POINTS: u32 = 100;
/// A cleared maze gives a life back, keeping at most this many.
const MAX_LIVES: u8 = 5;
/// The tracker follows the player's path this many tiles behind, and goes
/// straight for them once this close.
const TRAIL: usize = 6;
/// The cutter looks this many tiles ahead of the player for a crossing.
const CUT_AHEAD: i32 = 10;
/// The pouncer's charge, as a share of its usual speed.
const POUNCE: f32 = 1.2;
/// Closer than this (tiles) and a chaser has caught the player, or been eaten.
const CATCH: f32 = 0.6;

const STARTING_S: f32 = 1.6;
const DYING_S: f32 = 1.6;
const CLEARED_S: f32 = 2.0;
/// The pause after eating a chaser, with its points shown.
const CAUGHT_S: f32 = 0.6;

/// Tracker gold, cutter violet, guard lime, pouncer blue.
const CHASER_COLORS: [u32; 4] = [0xF2D14BFF, 0xB27CF2FF, 0xB4E04AFF, 0x4F86F7FF];
const FRIGHTENED_COLOR: u32 = 0x56607AFF;
const PLAYER_COLOR: u32 = 0x8DF7C4FF;
const FIELD_BG: u32 = 0x06080DFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tile {
    Wall,
    Pellet,
    Power,
    Empty,
    Door,
    House,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Left,
    Down,
    Right,
}

impl Dir {
    /// The tie-break order when two ways are equally good.
    const ALL: [Dir; 4] = [Dir::Up, Dir::Left, Dir::Down, Dir::Right];

    fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Left => (-1, 0),
            Dir::Down => (0, 1),
            Dir::Right => (1, 0),
        }
    }

    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Left => Dir::Right,
            Dir::Down => Dir::Up,
            Dir::Right => Dir::Left,
        }
    }
}

type Pos = (f32, f32);
type Cell = (i32, i32);

fn tile_of(p: Pos) -> Cell {
    ((p.0.round() as i32).rem_euclid(COLS), p.1.round() as i32)
}

fn neighbour((x, y): Cell, d: Dir) -> Cell {
    let (dx, dy) = d.delta();
    ((x + dx).rem_euclid(COLS), y + dy)
}

fn dist2(a: Cell, b: Cell) -> i32 {
    (a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)
}

/// Distance between two positions, the short way through the tunnel.
fn gap(a: Pos, b: Pos) -> f32 {
    let w = COLS as f32;
    let dx = (a.0 - b.0).rem_euclid(w);
    let dx = dx.min(w - dx);
    (dx * dx + (a.1 - b.1).powi(2)).sqrt()
}

struct Board {
    tiles: Vec<Tile>,
    pellets: u32,
}

impl Board {
    fn new() -> Self {
        let mut tiles = Vec::with_capacity((COLS * ROWS) as usize);
        for half in HALF {
            let left: Vec<u8> = half.bytes().collect();
            let row = left.iter().chain(left[..left.len() - 1].iter().rev());
            tiles.extend(row.map(|b| match b {
                b'#' => Tile::Wall,
                b'.' => Tile::Pellet,
                b'o' => Tile::Power,
                b'-' => Tile::Door,
                b'H' => Tile::House,
                _ => Tile::Empty,
            }));
        }
        let pellets = tiles
            .iter()
            .filter(|t| matches!(t, Tile::Pellet | Tile::Power))
            .count() as u32;
        Self { tiles, pellets }
    }

    fn at(&self, (x, y): Cell) -> Tile {
        if !(0..ROWS).contains(&y) {
            return Tile::Wall;
        }
        self.tiles[(y * COLS + x.rem_euclid(COLS)) as usize]
    }

    fn set(&mut self, (x, y): Cell, tile: Tile) {
        self.tiles[(y * COLS + x.rem_euclid(COLS)) as usize] = tile;
    }

    /// Corridors: everything but walls, the door and the house.
    fn open(&self, cell: Cell) -> bool {
        matches!(self.at(cell), Tile::Pellet | Tile::Power | Tile::Empty)
    }

    fn tunnel((x, y): Cell) -> bool {
        y == TUNNEL_ROW && (x <= 3 || x >= COLS - 4)
    }

    /// A crossing: more than two ways out.
    fn crossing(&self, at: Cell) -> bool {
        Dir::ALL
            .iter()
            .filter(|&&d| self.open(neighbour(at, d)))
            .count()
            > 2
    }

    /// The power pellet left nearest `to`.
    fn nearest_power(&self, to: Cell) -> Option<Cell> {
        (0..ROWS)
            .flat_map(|y| (0..COLS).map(move |x| (x, y)))
            .filter(|&at| self.at(at) == Tile::Power)
            .min_by_key(|&at| dist2(at, to))
    }

    /// `a` and `b` share a row or column with open corridor all the way.
    fn sees(&self, a: Cell, b: Cell) -> bool {
        if a.0 == b.0 {
            (a.1.min(b.1)..=a.1.max(b.1)).all(|y| self.open((a.0, y)))
        } else if a.1 == b.1 {
            (a.0.min(b.0)..=a.0.max(b.0)).all(|x| self.open((x, a.1)))
        } else {
            false
        }
    }
}

/// Moves `pos` along `dir` by `dist` tiles. At every tile centre reached,
/// `at_centre(tile, heading)` says the way on, or `None` to stop there.
fn travel(
    pos: &mut Pos,
    dir: &mut Option<Dir>,
    mut dist: f32,
    mut at_centre: impl FnMut(Cell, Dir) -> Option<Dir>,
) {
    while let Some(d) = *dir {
        let (dx, dy) = d.delta();
        let along = if dx != 0 {
            pos.0 * dx as f32
        } else {
            pos.1 * dy as f32
        };
        // The next centre strictly ahead.
        let ahead = (along + 1e-4).floor() + 1. - along;
        if dist < ahead {
            pos.0 += dx as f32 * dist;
            pos.1 += dy as f32 * dist;
            break;
        }
        dist -= ahead;
        pos.0 = (pos.0 + dx as f32 * ahead).round();
        pos.1 = (pos.1 + dy as f32 * ahead).round();
        if pos.0 < 0. {
            pos.0 += COLS as f32;
        } else if pos.0 >= COLS as f32 {
            pos.0 -= COLS as f32;
        }
        *dir = at_centre(tile_of(*pos), d);
    }
    // Half a tile past either edge is the other side of the tunnel.
    if pos.0 < -0.5 {
        pos.0 += COLS as f32;
    } else if pos.0 >= COLS as f32 - 0.5 {
        pos.0 -= COLS as f32;
    }
}

/// The way on from a tile centre: never back the way it came (unless it
/// must), the open way closest to `target` (straight on when two are as
/// close), or a random one when it has no target.
fn choose(board: &Board, at: Cell, heading: Dir, target: Option<Cell>, rng: &mut u64) -> Dir {
    let mut options = [Dir::Up; 4];
    let mut n = 0;
    for d in Dir::ALL {
        if d != heading.opposite() && board.open(neighbour(at, d)) {
            options[n] = d;
            n += 1;
        }
    }
    if n == 0 {
        return heading.opposite();
    }
    let Some(target) = target else {
        return options[(next(rng) % n as u64) as usize];
    };
    let mut best = options[0];
    let mut best_d = i32::MAX;
    for &d in &options[..n] {
        let (dx, dy) = d.delta();
        let d2 = dist2((at.0 + dx, at.1 + dy), target);
        if d2 < best_d || (d2 == best_d && d == heading) {
            best = d;
            best_d = d2;
        }
    }
    best
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Waiting in the house, bobbing.
    Home,
    /// Walking out of the door.
    Leaving,
    /// In the maze.
    Out,
    /// Eaten: eyes hurrying back to the door.
    Eyes,
    /// Eyes going down into the house to come back to life.
    Entering,
}

#[derive(Debug, Clone, Copy)]
struct Chaser {
    pos: Pos,
    dir: Dir,
    mode: Mode,
    frightened: bool,
    /// The pouncer: where it last saw the player, charging there.
    pounce: Option<Cell>,
}

#[derive(Debug, Clone, Copy)]
struct Player {
    pos: Pos,
    /// Moving this way; `None` when stopped against a wall.
    dir: Option<Dir>,
    facing: Dir,
    /// The turn asked for, taken at the first opening.
    want: Option<Dir>,
    /// The button for `want` is still held: it waits as long as that.
    want_held: bool,
    want_left: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Starting(f32),
    Running,
    /// A chaser was just eaten: a short freeze with its points shown.
    Caught(f32),
    Dying(f32),
    Cleared(f32),
    Over,
}

pub struct Maze {
    pub score: u32,
    pub lives: u8,
    pub level: u32,
    board: Board,
    player: Player,
    chasers: [Chaser; 4],
    stage: Stage,
    /// Seconds into the chasers' hunting rhythm (it stops while frightened).
    mode_clock: f32,
    /// Frightened seconds left.
    fright: f32,
    /// Chasers eaten in this fright.
    combo: u32,
    /// The player's last tiles, oldest first (the tracker follows them).
    trail: Vec<Cell>,
    /// Seconds since this life (or level) began.
    life_clock: f32,
    /// Pellets eaten this level.
    eaten: u32,
    /// A bonus item showing, with its seconds left.
    fruit: Option<f32>,
    /// Points shown where something was eaten: position, points, seconds left.
    popups: Vec<(Pos, u32, f32)>,
    /// Seconds of play, for blinking.
    anim: f32,
}

impl Maze {
    pub fn new() -> Self {
        let mut z = Self {
            score: 0,
            lives: 3,
            level: 1,
            board: Board::new(),
            player: Player {
                pos: (0., 0.),
                dir: None,
                facing: Dir::Left,
                want: None,
                want_held: false,
                want_left: 0.,
            },
            chasers: [Chaser {
                pos: (0., 0.),
                dir: Dir::Left,
                mode: Mode::Home,
                frightened: false,
                pounce: None,
            }; 4],
            stage: Stage::Starting(STARTING_S),
            mode_clock: 0.,
            fright: 0.,
            combo: 0,
            trail: Vec::new(),
            life_clock: 0.,
            eaten: 0,
            fruit: None,
            popups: Vec::new(),
            anim: 0.,
        };
        z.reset_actors();
        z
    }

    /// Everyone back to their places for a new life or level.
    fn reset_actors(&mut self) {
        self.player.pos = (START.0 as f32, START.1 as f32);
        self.player.dir = Some(Dir::Left);
        self.player.facing = Dir::Left;
        self.player.want = None;
        let at = |x: i32, y: f32, dir, mode| Chaser {
            pos: (x as f32, y),
            dir,
            mode,
            frightened: false,
            pounce: None,
        };
        self.chasers = [
            at(DOOR_OUT.0, DOOR_OUT.1 as f32, Dir::Left, Mode::Out),
            at(16, HOUSE_Y, Dir::Up, Mode::Home),
            at(14, HOUSE_Y, Dir::Down, Mode::Home),
            at(18, HOUSE_Y, Dir::Up, Mode::Home),
        ];
        self.mode_clock = 0.;
        self.fright = 0.;
        self.trail.clear();
        self.life_clock = 0.;
        self.fruit = None;
        self.popups.clear();
    }

    pub fn over(&self) -> bool {
        self.stage == Stage::Over
    }

    /// Where everyone is, to compare runs in tests.
    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> Vec<(f32, f32)> {
        let mut at = vec![self.player.pos];
        at.extend(self.chasers.iter().map(|c| c.pos));
        at
    }

    /// A direction button: pressed (`held`) or released.
    pub fn steer(&mut self, dir: Dir, held: bool) {
        let p = &mut self.player;
        if held {
            p.want = Some(dir);
            p.want_held = true;
            p.want_left = WANT_S;
        } else if p.want == Some(dir) {
            p.want_held = false;
        }
    }

    /// Chaser `i` is taking a break from the hunt, wandering.
    fn roaming(&self, i: usize) -> bool {
        let roam = match self.level {
            1 => 5.,
            2..=4 => 3.5,
            _ => 2.,
        };
        (self.mode_clock + STAGGER_S[i]).rem_euclid(HUNT_S + roam) >= HUNT_S
    }

    /// Where chaser `i` heads while it hunts, by its temper; `None` wanders.
    fn target(&self, i: usize) -> Option<Cell> {
        let player = tile_of(self.player.pos);
        let chaser = &self.chasers[i];
        match i {
            // The tracker: along the player's path, or straight at them
            // once close.
            0 => {
                let near = dist2(tile_of(chaser.pos), player) <= (TRAIL * TRAIL) as i32;
                Some(match self.trail.first() {
                    Some(&behind) if !near => behind,
                    _ => player,
                })
            }
            // The cutter: the next crossing ahead of the player.
            1 => {
                let mut at = player;
                for _ in 0..CUT_AHEAD {
                    let on = neighbour(at, self.player.facing);
                    if !self.board.open(on) {
                        break;
                    }
                    at = on;
                    if self.board.crossing(at) {
                        break;
                    }
                }
                Some(at)
            }
            // The guard: the power pellet nearest the player, or the player
            // once none are left.
            2 => Some(self.board.nearest_power(player).unwrap_or(player)),
            // The pouncer: where it last saw the player.
            _ => chaser.pounce,
        }
    }

    fn fright_seconds(&self) -> f32 {
        (6. - 0.75 * (self.level - 1) as f32).max(2.)
    }

    fn player_speed(&self) -> f32 {
        let base = if self.fright > 0. { 0.9 } else { 0.8 };
        BASE_SPEED * (base + 0.025 * (self.level - 1) as f32).min(1.)
    }

    fn chaser_speed(&self, c: &Chaser) -> f32 {
        BASE_SPEED
            * match c.mode {
                Mode::Eyes | Mode::Entering => 1.6,
                Mode::Home | Mode::Leaving => 0.45,
                Mode::Out if Board::tunnel(tile_of(c.pos)) => 0.4,
                Mode::Out if c.frightened => 0.5,
                Mode::Out => {
                    let pace = (0.75 + 0.03 * (self.level - 1) as f32).min(0.95);
                    if c.pounce.is_some() {
                        pace * POUNCE
                    } else {
                        pace
                    }
                }
            }
    }

    /// One fixed step of `dt` seconds.
    pub fn step(&mut self, dt: f32, rng: &mut u64) {
        self.anim += dt;
        for p in &mut self.popups {
            p.2 -= dt;
        }
        self.popups.retain(|p| p.2 > 0.);
        match &mut self.stage {
            Stage::Running => {}
            Stage::Over => return,
            Stage::Starting(t) | Stage::Caught(t) => {
                *t -= dt;
                if *t <= 0. {
                    self.stage = Stage::Running;
                }
                return;
            }
            Stage::Dying(t) => {
                *t -= dt;
                if *t <= 0. {
                    self.lives -= 1;
                    if self.lives == 0 {
                        self.stage = Stage::Over;
                    } else {
                        self.reset_actors();
                        self.stage = Stage::Starting(STARTING_S);
                    }
                }
                return;
            }
            Stage::Cleared(t) => {
                *t -= dt;
                if *t <= 0. {
                    self.level += 1;
                    self.board = Board::new();
                    self.eaten = 0;
                    self.reset_actors();
                    self.stage = Stage::Starting(STARTING_S);
                }
                return;
            }
        }
        self.life_clock += dt;
        if self.fright > 0. {
            self.fright -= dt;
            if self.fright <= 0. {
                self.fright = 0.;
                for c in &mut self.chasers {
                    c.frightened = false;
                }
            }
        } else {
            self.mode_clock += dt;
        }
        if let Some(t) = &mut self.fruit {
            *t -= dt;
            if *t <= 0. {
                self.fruit = None;
            }
        }
        self.move_player(dt);
        let at = tile_of(self.player.pos);
        if self.trail.last() != Some(&at) {
            if self.trail.len() == TRAIL {
                self.trail.remove(0);
            }
            self.trail.push(at);
        }
        self.eat();
        if self.stage != Stage::Running {
            return;
        }
        self.collide();
        if self.stage != Stage::Running {
            return;
        }
        self.move_chasers(dt, rng);
        self.collide();
    }

    fn move_player(&mut self, dt: f32) {
        let speed = self.player_speed();
        let board = &self.board;
        let p = &mut self.player;
        if !p.want_held && p.want.is_some() {
            p.want_left -= dt;
            if p.want_left <= 0. {
                p.want = None;
            }
        }
        // Turning back is allowed anywhere; from a standstill any open way.
        match (p.want, p.dir) {
            (Some(w), Some(d)) if w == d.opposite() => {
                p.dir = Some(w);
                p.want = None;
            }
            (Some(w), None) if board.open(neighbour(tile_of(p.pos), w)) => {
                p.dir = Some(w);
                p.want = None;
            }
            _ => {}
        }
        let want = &mut p.want;
        travel(&mut p.pos, &mut p.dir, speed * dt, |at, heading| {
            if let Some(w) = *want {
                if board.open(neighbour(at, w)) {
                    *want = None;
                    return Some(w);
                }
            }
            board.open(neighbour(at, heading)).then_some(heading)
        });
        if let Some(d) = p.dir {
            p.facing = d;
        }
    }

    fn eat(&mut self) {
        let at = tile_of(self.player.pos);
        match self.board.at(at) {
            Tile::Pellet => {
                self.board.set(at, Tile::Empty);
                self.score += PELLET_POINTS;
            }
            Tile::Power => {
                self.board.set(at, Tile::Empty);
                self.score += POWER_POINTS;
                self.frighten();
            }
            _ => {
                if self.fruit.is_some() && at == START {
                    let points = BONUS_POINTS * self.level.min(10);
                    self.fruit = None;
                    self.score += points;
                    self.popups
                        .push(((START.0 as f32, START.1 as f32), points, 1.5));
                }
                return;
            }
        }
        self.board.pellets -= 1;
        self.eaten += 1;
        if FRUIT_AT.contains(&self.eaten) {
            self.fruit = Some(FRUIT_S);
        }
        if self.board.pellets == 0 {
            self.score += CLEAR_POINTS * self.level;
            self.lives = (self.lives + 1).min(MAX_LIVES);
            self.stage = Stage::Cleared(CLEARED_S);
        }
    }

    fn frighten(&mut self) {
        self.fright = self.fright_seconds();
        self.combo = 0;
        for c in &mut self.chasers {
            if matches!(c.mode, Mode::Eyes | Mode::Entering) {
                continue;
            }
            c.frightened = true;
            if c.mode == Mode::Out {
                c.dir = c.dir.opposite();
            }
        }
    }

    fn move_chasers(&mut self, dt: f32, rng: &mut u64) {
        let player = tile_of(self.player.pos);
        // The pouncer charges at the player once it sees them while hunting,
        // and gives up where it last saw them.
        let hunting = !self.roaming(3);
        let pouncer = &mut self.chasers[3];
        let at = tile_of(pouncer.pos);
        if pouncer.mode != Mode::Out || pouncer.frightened || !hunting {
            pouncer.pounce = None;
        } else if self.board.sees(at, player) {
            pouncer.pounce = Some(player);
        } else if pouncer.pounce == Some(at) {
            pouncer.pounce = None;
        }
        for (i, release_at) in RELEASE_S.into_iter().enumerate() {
            let goal = if self.roaming(i) {
                None
            } else {
                self.target(i)
            };
            let dist = self.chaser_speed(&self.chasers[i]) * dt;
            let release = self.life_clock >= release_at;
            let board = &self.board;
            let c = &mut self.chasers[i];
            match c.mode {
                Mode::Home => {
                    if release {
                        c.mode = Mode::Leaving;
                    } else {
                        // Bob up and down.
                        let dy = if c.dir == Dir::Up { -dist } else { dist };
                        c.pos.1 += dy;
                        if c.pos.1 <= HOUSE_Y - 0.4 {
                            c.dir = Dir::Down;
                        } else if c.pos.1 >= HOUSE_Y + 0.4 {
                            c.dir = Dir::Up;
                        }
                    }
                }
                Mode::Leaving => {
                    let dx = DOOR_OUT.0 as f32 - c.pos.0;
                    if dx.abs() > 1e-3 {
                        c.pos.0 += dx.signum() * dist.min(dx.abs());
                    } else if c.pos.1 > DOOR_OUT.1 as f32 {
                        c.pos.1 = (c.pos.1 - dist).max(DOOR_OUT.1 as f32);
                    } else {
                        c.mode = Mode::Out;
                        c.dir = Dir::Left;
                    }
                }
                Mode::Entering => {
                    c.pos.1 += dist;
                    if c.pos.1 >= HOUSE_Y {
                        c.pos.1 = HOUSE_Y;
                        c.mode = Mode::Leaving;
                        c.frightened = false;
                    }
                }
                Mode::Out | Mode::Eyes => {
                    let eyes = c.mode == Mode::Eyes;
                    let goal = if eyes {
                        Some(DOOR_OUT)
                    } else if c.frightened {
                        None
                    } else {
                        goal
                    };
                    let mut dir = Some(c.dir);
                    let mut home = false;
                    travel(&mut c.pos, &mut dir, dist, |at, heading| {
                        if eyes && at == DOOR_OUT {
                            home = true;
                            return None;
                        }
                        Some(choose(board, at, heading, goal, rng))
                    });
                    if let Some(d) = dir {
                        c.dir = d;
                    }
                    if home {
                        c.mode = Mode::Entering;
                    }
                }
            }
        }
    }

    fn collide(&mut self) {
        for i in 0..self.chasers.len() {
            let c = self.chasers[i];
            if c.mode != Mode::Out || gap(c.pos, self.player.pos) >= CATCH {
                continue;
            }
            if c.frightened {
                self.combo += 1;
                let points = CHASER_POINTS * self.combo;
                self.chasers[i].frightened = false;
                self.chasers[i].mode = Mode::Eyes;
                self.score += points;
                self.popups.push((c.pos, points, 1.0));
                self.stage = Stage::Caught(CAUGHT_S);
            } else {
                self.stage = Stage::Dying(DYING_S);
                self.player.want = None;
            }
            return;
        }
    }
}

impl Default for Maze {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------ drawing

fn centre((x, y): Pos) -> (f32, f32) {
    (LEFT + (x + 0.5) * TILE, TOP + (y + 0.5) * TILE)
}

/// The score strip's middle text.
pub(crate) fn status(z: &Maze) -> String {
    let lives = if z.lives == 1 { "life" } else { "lives" };
    format!("Level {} · {} {lives}", z.level, z.lives)
}

pub(crate) fn draw(c: &mut Canvas, z: &Maze, accent: u32) {
    c.rrect(
        LEFT - 6.,
        TOP - 6.,
        WIDTH_PX + 12.,
        HEIGHT_PX + 12.,
        12.,
        FIELD_BG,
    );
    let flash = matches!(z.stage, Stage::Cleared(_)) && (z.anim * 4.) as i32 % 2 == 0;
    let edge = if flash {
        0xF4F6FFFF
    } else {
        mix(accent, 0xFFFFFFFF, 0.2)
    };
    draw_walls(c, &z.board, alpha(mix(accent, FIELD_BG, 0.78), 255), edge);
    // The door.
    let (dx, dy) = centre((16., 8.));
    c.rect(dx - TILE / 2., dy - 1., TILE, 2., 0xF0A8D0FF);
    // Pellets.
    let blink = (z.anim * 3.) as i32 % 2 == 0 || z.stage != Stage::Running;
    for y in 0..ROWS {
        for x in 0..COLS {
            let (px, py) = centre((x as f32, y as f32));
            match z.board.at((x, y)) {
                Tile::Pellet => c.rect(px - 1.5, py - 1.5, 3., 3., 0xF2E6CCFF),
                Tile::Power if blink => c.disc(px, py, 4.5, 0xF7EED8FF),
                _ => {}
            }
        }
    }
    if z.fruit.is_some() {
        let (fx, fy) = centre((START.0 as f32, START.1 as f32));
        draw_bonus(c, fx, fy, (z.level as usize - 1) % BONUS_KINDS);
    }
    let dying = match z.stage {
        Stage::Dying(t) => Some(1. - t / DYING_S),
        _ => None,
    };
    if dying.is_none() {
        let late = z.fright > 0. && z.fright < 2. && (z.anim * 6.) as i32 % 2 == 0;
        for (i, ch) in z.chasers.iter().enumerate() {
            draw_chaser(c, ch, CHASER_COLORS[i], late);
        }
    }
    draw_player(c, &z.player, dying);
    for &(pos, points, _) in &z.popups {
        let (px, py) = centre(pos);
        c.text_mid_center(px, py, &points.to_string(), Face::S10, 0x9FE8FFFF);
    }
    let message = match z.stage {
        Stage::Starting(_) if z.level > 1 && z.eaten == 0 && z.lives > 0 => {
            Some(format!("Level {}", z.level))
        }
        Stage::Starting(_) => Some("Ready".to_owned()),
        _ => None,
    };
    if let Some(text) = message {
        let (mx, my) = centre((16., HOUSE_Y));
        c.rrect(mx - 40., my - 11., 80., 22., 11., alpha(FIELD_BG, 230));
        c.text_mid_center(mx, my, &text, Face::S13, color::TEXT);
    }
}

/// Filled wall runs, then a bright line on every side that faces a corridor.
fn draw_walls(c: &mut Canvas, board: &Board, fill: u32, edge: u32) {
    let wall = |x: i32, y: i32| (0..COLS).contains(&x) && board.at((x, y)) == Tile::Wall;
    for y in 0..ROWS {
        let mut x = 0;
        while x < COLS {
            if !wall(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < COLS && wall(x, y) {
                x += 1;
            }
            c.rect(
                LEFT + start as f32 * TILE,
                TOP + y as f32 * TILE,
                (x - start) as f32 * TILE,
                TILE,
                fill,
            );
        }
    }
    // Horizontal edges, merged into runs.
    for y in 0..ROWS {
        for (side, dy) in [(0., -1), (TILE - 2., 1)] {
            let mut x = 0;
            while x < COLS {
                let facing =
                    |x: i32| wall(x, y) && (0..ROWS).contains(&(y + dy)) && !wall(x, y + dy);
                if !facing(x) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < COLS && facing(x) {
                    x += 1;
                }
                c.rect(
                    LEFT + start as f32 * TILE,
                    TOP + y as f32 * TILE + side,
                    (x - start) as f32 * TILE,
                    2.,
                    edge,
                );
            }
        }
    }
    // Vertical edges.
    for x in 0..COLS {
        for (side, dx) in [(0., -1), (TILE - 2., 1)] {
            let mut y = 0;
            while y < ROWS {
                let facing =
                    |y: i32| wall(x, y) && (0..COLS).contains(&(x + dx)) && !wall(x + dx, y);
                if !facing(y) {
                    y += 1;
                    continue;
                }
                let start = y;
                while y < ROWS && facing(y) {
                    y += 1;
                }
                c.rect(
                    LEFT + x as f32 * TILE + side,
                    TOP + start as f32 * TILE,
                    2.,
                    (y - start) as f32 * TILE,
                    edge,
                );
            }
        }
    }
}

/// On screen, or in the tunnel's mouth.
fn visible(pos: Pos) -> bool {
    pos.0 >= -0.2 && pos.0 <= COLS as f32 - 0.8
}

fn draw_player(c: &mut Canvas, p: &Player, dying: Option<f32>) {
    if !visible(p.pos) {
        return;
    }
    let (x, y) = centre(p.pos);
    if let Some(t) = dying {
        // Shrinks away inside a widening ring.
        let r = 5.5 * (1. - t).max(0.);
        if r > 0.5 {
            c.disc(x, y, r, PLAYER_COLOR);
        }
        c.ring(
            x,
            y,
            4. + 10. * t,
            1.5,
            alpha(PLAYER_COLOR, (200. * (1. - t)) as u8),
        );
        return;
    }
    c.disc(x, y, 5.5, PLAYER_COLOR);
    // A visor on the side it faces.
    let (dx, dy) = p.facing.delta();
    let (vx, vy) = (x + dx as f32 * 2.5, y + dy as f32 * 2.5);
    if dx != 0 {
        c.rect(vx - 1., vy - 3., 2., 6., 0x10302AFF);
    } else {
        c.rect(vx - 3., vy - 1., 6., 2., 0x10302AFF);
    }
}

fn draw_chaser(c: &mut Canvas, ch: &Chaser, color: u32, late: bool) {
    if !visible(ch.pos) {
        return;
    }
    let (x, y) = centre(ch.pos);
    let eyes_only = matches!(ch.mode, Mode::Eyes | Mode::Entering);
    if !eyes_only {
        let body = match (ch.frightened, late) {
            (true, false) => FRIGHTENED_COLOR,
            (true, true) => 0xE6E8F2FF,
            _ => color,
        };
        // A rounded block with a little antenna.
        c.rrect(x - 5.5, y - 5., 11., 10.5, 3., body);
        c.rect(x - 0.5, y - 7.5, 1., 2.5, body);
        if ch.frightened {
            c.rect(x - 3., y - 2.5, 2., 2., 0xF6D6A8FF);
            c.rect(x + 1., y - 2.5, 2., 2., 0xF6D6A8FF);
            return;
        }
    }
    let (dx, dy) = ch.dir.delta();
    for ex in [x - 2.5, x + 2.5] {
        c.rect(ex - 1.5, y - 4., 3., 3.5, 0xFFFFFFFF);
        c.rect(
            ex - 0.75 + dx as f32 * 0.75,
            y - 3. + dy as f32 * 0.9,
            1.5,
            1.5,
            0x1A2A6AFF,
        );
    }
}

const BONUS_KINDS: usize = 6;

/// Bonus items: a gem, an acorn, a mushroom, a crown, a star and a moon.
fn draw_bonus(c: &mut Canvas, x: f32, y: f32, kind: usize) {
    match kind {
        0 => {
            // Gem: stacked bars narrowing to a point.
            for (i, w) in [6., 10., 8., 5., 2.].iter().enumerate() {
                c.rect(x - w / 2., y - 4. + i as f32 * 2., *w, 2., 0x5FE3F0FF);
            }
            c.rect(x - 3., y - 4., 2., 2., 0xD8FAFFFF);
        }
        1 => {
            c.disc(x, y + 1.5, 4., 0xC7843EFF);
            c.rrect(x - 5., y - 4., 10., 4., 2., 0x6E4A2AFF);
            c.rect(x - 0.5, y - 6., 1., 2., 0x6E4A2AFF);
        }
        2 => {
            c.rrect(x - 6., y - 5., 12., 6., 3., 0xE24A4AFF);
            c.rect(x - 2., y + 1., 4., 4., 0xF2E6D0FF);
            c.rect(x - 3., y - 3., 2., 2., 0xFFFFFFFF);
            c.rect(x + 2., y - 4., 2., 2., 0xFFFFFFFF);
        }
        3 => {
            c.rect(x - 5., y, 10., 4., 0xF2C14CFF);
            for dx in [-5., -1., 3.] {
                c.rect(x + dx, y - 4., 2., 4., 0xF2C14CFF);
            }
            c.rect(x - 1., y + 1., 2., 2., 0xE8504AFF);
        }
        4 => {
            c.rect(x - 1.5, y - 6., 3., 12., 0xFFE36BFF);
            c.rect(x - 6., y - 1.5, 12., 3., 0xFFE36BFF);
            c.rect(x - 3., y - 3., 6., 6., 0xFFE36BFF);
        }
        _ => {
            c.disc(x, y, 5., 0xF4E7A0FF);
            c.disc(x + 2.5, y - 1.5, 4., FIELD_BG);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1. / 60.;

    fn running() -> Maze {
        let mut z = Maze::new();
        z.stage = Stage::Running;
        z
    }

    fn run(z: &mut Maze, seconds: f32, rng: &mut u64) {
        for _ in 0..(seconds / DT).round() as usize {
            z.step(DT, rng);
        }
    }

    #[test]
    fn the_maze_is_mirrored_with_four_power_pellets_and_no_dead_ends() {
        let b = Board::new();
        assert_eq!(b.tiles.len(), (COLS * ROWS) as usize);
        for y in 0..ROWS {
            for x in 0..COLS {
                assert_eq!(b.at((x, y)), b.at((COLS - 1 - x, y)), "{x},{y}");
                if b.open((x, y)) {
                    let exits = Dir::ALL
                        .iter()
                        .filter(|&&d| b.open(neighbour((x, y), d)))
                        .count();
                    assert!(exits >= 2, "dead end at {x},{y}");
                }
            }
        }
        let power = b.tiles.iter().filter(|&&t| t == Tile::Power).count();
        assert_eq!(power, 4);
        assert!(b.pellets > 200, "{}", b.pellets);
        // The tunnel goes through.
        assert!(b.open((0, TUNNEL_ROW)) && b.open((COLS - 1, TUNNEL_ROW)));
    }

    #[test]
    fn every_pellet_can_be_reached_from_the_start() {
        let b = Board::new();
        let mut seen = vec![false; b.tiles.len()];
        let mut todo = vec![START];
        while let Some(at) = todo.pop() {
            let i = (at.1 * COLS + at.0) as usize;
            if seen[i] || !b.open(at) {
                continue;
            }
            seen[i] = true;
            todo.extend(Dir::ALL.map(|d| neighbour(at, d)));
        }
        for (i, t) in b.tiles.iter().enumerate() {
            if matches!(t, Tile::Pellet | Tile::Power) {
                assert!(seen[i], "pellet {} cut off", i);
            }
        }
        // Chasers can get out of the house to the door's tile.
        assert_eq!(b.at((16, 8)), Tile::Door);
        assert!(b.open(DOOR_OUT));
    }

    #[test]
    fn a_turn_pressed_early_is_taken_at_the_corner() {
        let mut rng = 5;
        let mut z = running();
        z.chasers.iter_mut().for_each(|c| c.mode = Mode::Home);
        z.life_clock = -100.; // keep them home
        z.player.pos = (15., 4.);
        z.player.dir = Some(Dir::Left);
        // Down is walled at x 15, 14, 13; the lane down is at x 12.
        z.steer(Dir::Down, true);
        z.steer(Dir::Down, false);
        z.step(DT, &mut rng);
        assert_eq!(z.player.dir, Some(Dir::Left), "no opening yet");
        run(&mut z, 0.5, &mut rng);
        assert_eq!(z.player.pos.0, 12.);
        assert!(
            z.player.pos.1 > 4.5,
            "went down the lane: {:?}",
            z.player.pos
        );
        assert_eq!(z.player.facing, Dir::Down);
    }

    #[test]
    fn a_tapped_turn_lapses_but_a_held_one_waits() {
        let mut rng = 5;
        let mut z = running();
        z.life_clock = -100.;
        z.chasers.iter_mut().for_each(|c| c.mode = Mode::Home);
        z.player.pos = (20., 4.);
        z.player.dir = Some(Dir::Left);
        // Down first opens at x 12, eight tiles (over a second) away.
        z.steer(Dir::Down, true);
        z.steer(Dir::Down, false);
        run(&mut z, 1.0, &mut rng);
        assert_eq!(z.player.want, None, "a tap lapses");
        assert_eq!(z.player.facing, Dir::Left);
        z.steer(Dir::Down, true);
        run(&mut z, 1.5, &mut rng);
        assert_eq!(z.player.facing, Dir::Down, "held, it waits for the lane");
    }

    #[test]
    fn turning_back_is_immediate_and_walls_stop_the_player() {
        let mut rng = 5;
        let mut z = running();
        z.life_clock = -100.;
        z.chasers.iter_mut().for_each(|c| c.mode = Mode::Home);
        z.player.pos = (5.5, 4.);
        z.player.dir = Some(Dir::Left);
        z.steer(Dir::Right, true);
        z.step(DT, &mut rng);
        assert_eq!(z.player.dir, Some(Dir::Right));
        assert!(z.player.pos.0 > 5.5);
        // Run into the top-left corner and stop there.
        z.player.pos = (3., 1.);
        z.player.dir = Some(Dir::Left);
        z.player.want = None;
        run(&mut z, 1., &mut rng);
        assert_eq!(z.player.pos, (1., 1.));
        assert_eq!(z.player.dir, None);
    }

    #[test]
    fn the_tunnel_wraps_round() {
        let mut rng = 5;
        let mut z = running();
        z.life_clock = -100.;
        z.chasers.iter_mut().for_each(|c| c.mode = Mode::Home);
        z.player.pos = (1., TUNNEL_ROW as f32);
        z.player.dir = Some(Dir::Left);
        run(&mut z, 0.5, &mut rng);
        assert!(z.player.pos.0 > 28., "{:?}", z.player.pos);
        assert_eq!(z.player.dir, Some(Dir::Left));
    }

    #[test]
    fn each_chaser_hunts_in_its_own_way() {
        let mut z = running();
        z.player.pos = (12., 13.);
        z.player.facing = Dir::Right;
        z.trail = (6..12).map(|x| (x, 13)).collect();
        // The tracker follows the path from afar and goes straight in close.
        z.chasers[0].pos = (30., 4.);
        assert_eq!(z.target(0), Some((6, 13)));
        z.chasers[0].pos = (10., 13.);
        assert_eq!(z.target(0), Some((12, 13)));
        // The cutter: the next crossing ahead, whichever way the player faces,
        // or the player's own tile against a wall.
        assert_eq!(z.target(1), Some((16, 13)));
        z.player.facing = Dir::Left;
        assert_eq!(z.target(1), Some((9, 13)));
        z.player.pos = (1., 1.);
        assert_eq!(z.target(1), Some((1, 1)));
        // The guard: the power pellet nearest the player, then the player.
        z.player.pos = (12., 13.);
        assert_eq!(z.target(2), Some((1, 16)));
        z.board.set((1, 16), Tile::Empty);
        assert_eq!(z.target(2), Some((1, 1)));
        for at in [(1, 1), (COLS - 2, 1), (COLS - 2, 16)] {
            z.board.set(at, Tile::Empty);
        }
        assert_eq!(z.target(2), Some((12, 13)));
        // The pouncer has nowhere to go until it sees the player.
        assert_eq!(z.target(3), None);
    }

    #[test]
    fn the_pouncer_charges_down_a_corridor_to_where_it_saw_the_player() {
        let b = Board::new();
        assert!(b.sees((1, 4), (20, 4)));
        assert!(b.sees((12, 4), (12, 13)), "down the open column");
        assert!(!b.sees((1, 4), (6, 7)), "not diagonally");
        assert!(!b.sees((4, 7), (6, 7)), "not through a wall");
        let mut rng = 3;
        let mut z = running();
        z.player.pos = (12., 13.);
        z.chasers[3].mode = Mode::Out;
        z.chasers[3].pos = (12., 4.);
        z.chasers[3].dir = Dir::Down;
        z.move_chasers(DT, &mut rng);
        assert_eq!(z.chasers[3].pounce, Some((12, 13)));
        assert_eq!(z.target(3), Some((12, 13)));
        let mut calm = z.chasers[3];
        calm.pounce = None;
        assert!(z.chaser_speed(&z.chasers[3]) > z.chaser_speed(&calm));
        // Out of sight it keeps on to that spot, and gives up there.
        z.player.pos = (1., 1.);
        z.move_chasers(DT, &mut rng);
        assert_eq!(z.chasers[3].pounce, Some((12, 13)));
        z.chasers[3].pos = (12., 13.);
        z.move_chasers(DT, &mut rng);
        assert_eq!(z.chasers[3].pounce, None);
        // Wandering, it does not charge.
        z.player.pos = (12., 4.);
        z.chasers[3].pos = (12., 13.);
        z.mode_clock = 4.;
        assert!(z.roaming(3));
        z.move_chasers(DT, &mut rng);
        assert_eq!(z.chasers[3].pounce, None);
    }

    #[test]
    fn each_chaser_hunts_then_wanders_on_its_own_rhythm() {
        let mut z = running();
        assert!((0..4).all(|i| !z.roaming(i)), "all hunt at first");
        let mut wandered = [false; 4];
        for k in 0..240 {
            z.mode_clock = k as f32 * 0.25;
            let roaming: Vec<bool> = (0..4).map(|i| z.roaming(i)).collect();
            assert!(roaming.iter().filter(|&&r| r).count() <= 2, "{k}");
            for (w, r) in wandered.iter_mut().zip(roaming) {
                *w |= r;
            }
        }
        assert_eq!(wandered, [true; 4]);
        // Later levels wander for less.
        z.mode_clock = HUNT_S + 2.5;
        assert!(z.roaming(0));
        z.level = 5;
        assert!(!z.roaming(0));
    }

    #[test]
    fn chasers_take_the_way_nearest_their_target_and_never_reverse() {
        let b = Board::new();
        let mut rng = 1;
        // At (12, 4) heading left: up, left and down are open.
        assert_eq!(
            choose(&b, (12, 4), Dir::Left, Some((12, 20)), &mut rng),
            Dir::Down
        );
        assert_eq!(
            choose(&b, (12, 4), Dir::Left, Some((0, 4)), &mut rng),
            Dir::Left
        );
        // Behind it is closest, but it may not turn back.
        assert_ne!(
            choose(&b, (12, 4), Dir::Left, Some((20, 4)), &mut rng),
            Dir::Right
        );
        // On a tie (up and right are as close) it keeps going straight.
        assert_eq!(
            choose(&b, (16, 4), Dir::Right, Some((16, 4)), &mut rng),
            Dir::Right
        );
        assert_eq!(
            choose(&b, (16, 4), Dir::Left, Some((16, 4)), &mut rng),
            Dir::Left
        );
        // No target (frightened, wandering): random, but still never back.
        for seed in 1..40 {
            let mut r = seed;
            assert_ne!(choose(&b, (12, 4), Dir::Left, None, &mut r), Dir::Right);
        }
    }

    #[test]
    fn chasers_leave_the_house_in_turn() {
        let mut rng = 9;
        let mut z = running();
        // Out of everyone's reach, so no one is caught meanwhile.
        z.player.pos = (0., 0.);
        z.player.dir = None;
        run(&mut z, 1.0, &mut rng);
        let out = |z: &Maze| z.chasers.iter().filter(|c| c.mode == Mode::Out).count();
        assert_eq!(out(&z), 1);
        run(&mut z, 3.5, &mut rng);
        assert_eq!(out(&z), 2);
        run(&mut z, 9., &mut rng);
        assert_eq!(out(&z), 4);
    }

    #[test]
    fn a_power_pellet_frightens_and_eaten_chasers_go_home() {
        let mut rng = 3;
        let mut z = running();
        z.player.pos = (2., 1.);
        z.player.dir = Some(Dir::Left);
        run(&mut z, 0.2, &mut rng);
        assert_eq!(z.board.at((1, 1)), Tile::Empty);
        assert!(z.fright > 0.);
        assert!(z.chasers[0].frightened);
        assert!(z.chasers[1].frightened, "even at home");
        z.chasers[0].dir = Dir::Left;
        z.frighten();
        assert_eq!(z.chasers[0].dir, Dir::Right, "turned round");
        let before = z.score;
        // Walk a frightened chaser into the player.
        z.chasers[0].pos = z.player.pos;
        z.collide();
        assert_eq!(z.score, before + CHASER_POINTS);
        assert_eq!(z.chasers[0].mode, Mode::Eyes);
        z.chasers[1].mode = Mode::Out;
        z.chasers[1].pos = z.player.pos;
        z.chasers[1].dir = Dir::Right;
        z.collide();
        assert_eq!(
            z.score,
            before + CHASER_POINTS * 3,
            "the second is worth twice the first"
        );
        // The eyes find their way home and come back out.
        z.stage = Stage::Running;
        z.player.pos = (0., 0.);
        z.player.dir = None;
        z.life_clock = -100.;
        let mut entered = false;
        for _ in 0..(12. / DT) as usize {
            z.step(DT, &mut rng);
            entered |= z.chasers[0].mode == Mode::Entering;
            if entered && z.chasers[0].mode == Mode::Out {
                break;
            }
        }
        assert!(entered);
        assert_eq!(z.chasers[0].mode, Mode::Out);
        assert!(!z.chasers[0].frightened);
    }

    #[test]
    fn being_caught_costs_a_life_and_three_end_the_game() {
        let mut rng = 3;
        let mut z = running();
        for life in (0..3).rev() {
            z.stage = Stage::Running;
            z.chasers[0].pos = z.player.pos;
            z.chasers[0].mode = Mode::Out;
            z.collide();
            assert!(matches!(z.stage, Stage::Dying(_)));
            run(&mut z, DYING_S + 0.1, &mut rng);
            assert_eq!(z.lives, life);
        }
        assert!(z.over());
    }

    #[test]
    fn clearing_the_maze_gives_a_life_back_and_starts_a_faster_level() {
        let mut rng = 3;
        let mut z = running();
        let speed = z.chaser_speed(&z.chasers[0]);
        for t in z.board.tiles.iter_mut() {
            if matches!(t, Tile::Pellet | Tile::Power) {
                *t = Tile::Empty;
            }
        }
        z.board.set((14, 13), Tile::Pellet);
        z.board.pellets = 1;
        z.chasers.iter_mut().for_each(|c| c.mode = Mode::Home);
        z.life_clock = -100.;
        run(&mut z, 0.3, &mut rng);
        assert!(matches!(z.stage, Stage::Cleared(_)));
        assert_eq!(z.score, PELLET_POINTS + CLEAR_POINTS);
        assert_eq!(z.lives, 4);
        run(&mut z, CLEARED_S + 0.1, &mut rng);
        assert_eq!(z.level, 2);
        assert!(z.board.pellets > 200);
        assert!(z.chaser_speed(&z.chasers[0]) > speed);
        // Five lives at most.
        z.lives = MAX_LIVES;
        z.board.pellets = 1;
        z.board.set(tile_of(z.player.pos), Tile::Pellet);
        z.eat();
        assert_eq!(z.lives, MAX_LIVES);
    }

    #[test]
    fn bonus_items_are_worth_more_each_level() {
        let mut z = running();
        z.eaten = FRUIT_AT[0] - 1;
        z.player.pos = (14., 13.);
        z.player.dir = None;
        z.board.set((14, 13), Tile::Pellet);
        z.eat();
        assert!(z.fruit.is_some());
        z.player.pos = (START.0 as f32, START.1 as f32);
        z.eat();
        assert!(z.fruit.is_none());
        assert_eq!(z.score, PELLET_POINTS + BONUS_POINTS);
        z.level = 3;
        z.fruit = Some(FRUIT_S);
        z.eat();
        assert_eq!(z.score, PELLET_POINTS + BONUS_POINTS * 4);
    }
}

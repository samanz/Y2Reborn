//! Shatterfield (Extras > Games): a ship in a field of drifting rocks, drawn
//! as glowing vector outlines. The wheel turns the ship like a rotary knob,
//! Volume Up (held) thrusts, Volume Down fires (held, it keeps firing at a
//! limited rate), Select jumps through hyperspace and Play/Pause pauses.
//! Plain state advanced in fixed steps by `games::Game`, so the rules are
//! deterministic for a seed and testable.
//!
//! Scoring rewards accuracy: rocks are worth 10, 20 and 30 (big to small)
//! times the streak, which grows by one with every hit up to x5 and drops
//! back to x1 when a shot flies out without hitting anything or a ship is
//! lost. A cleared wave adds 100 per wave number; saucers are 300 and 750.
use crate::{
    canvas::Canvas,
    games::{next, rand01},
    theme::{alpha, mix},
};
use reborn_graphics::{Quad, Warp};
use std::f32::consts::{PI, TAU};

/// The field on screen: below the score line, nearly the full width.
pub const FIELD_X: f32 = 8.;
pub const FIELD_Y: f32 = 64.;
pub const W: f32 = 464.;
pub const H: f32 = 288.;

/// One wheel detent turns the ship this far; a fast spin (the input layer
/// reports up to 4 detents at once) turns progressively further per detent.
const TURN_STEP: f32 = 6. * PI / 180.;
const TURN_ACCEL: [f32; 4] = [1., 2.5, 4.5, 7.];
/// How quickly the ship's heading catches up with the wheel (per second):
/// a detent is a smooth turn over a few frames rather than a jump.
const AIM_RATE: f32 = 28.;
/// Thrust (px/s²), drag (fraction of speed lost per second, roughly) and
/// the top speed.
const THRUST: f32 = 300.;
const DRAG: f32 = 0.6;
const MAX_SPEED: f32 = 300.;
const SHIP_R: f32 = 8.;
const SHOT_SPEED: f32 = 360.;
const SHOT_LIFE: f32 = 0.85;
const MAX_SHOTS: usize = 4;
/// Held fire repeats this often (seconds).
const REFIRE: f32 = 0.22;
/// Seconds in hyperspace, and the 1-in-N chance of not coming back.
const HYPER_S: f32 = 0.6;
const HYPER_RISK: u64 = 8;
const RESPAWN_S: f32 = 2.;
/// A new ship waits until no rock is this close to the centre.
const SAFE_R: f32 = 70.;
const WAVE_GAP_S: f32 = 2.;
const OVER_S: f32 = 2.;
/// Rock radius and points by size (1 small, 2 medium, 3 big).
const ROCK_R: [f32; 4] = [0., 8., 15., 28.];
const ROCK_POINTS: [u32; 4] = [0, 30, 20, 10];
/// Hits in a row multiply rock points up to this.
const MAX_STREAK: u32 = 5;
/// A cleared wave is worth this much per wave number.
const WAVE_BONUS: u32 = 100;
/// The big and the small saucer.
const SAUCER_POINTS: [u32; 2] = [300, 750];
/// One more ship every this many points, keeping at most `MAX_SHIPS`.
const SHIP_EVERY: u32 = 5_000;
const MAX_SHIPS: u8 = 5;
const SAUCER_FROM_WAVE: u32 = 2;
/// From this wave on the saucer is the small one, which aims.
const SMALL_SAUCER_FROM_WAVE: u32 = 4;
const SAUCER_SHOT_SPEED: f32 = 230.;
const SAUCER_SHOT_LIFE: f32 = 1.3;
const MAX_DEBRIS: usize = 64;

const ROCK_COLOR: u32 = 0xCFDEFFFF;
const SAUCER_COLOR: u32 = 0xFF7AD9FF;
const SHOT_COLOR: u32 = 0xFFFFFFFF;
const FIELD_BG: u32 = 0x04060AFF;

/// Rock outlines: the radius at each of 11 evenly spaced points.
const SHAPES: [[f32; 11]; 4] = [
    [
        1.0, 0.86, 0.98, 0.74, 0.95, 1.0, 0.82, 0.97, 0.78, 0.93, 0.88,
    ],
    [0.92, 1.0, 0.78, 0.9, 1.0, 0.72, 0.96, 0.88, 1.0, 0.8, 0.95],
    [1.0, 0.8, 0.94, 1.0, 0.85, 0.7, 0.97, 0.9, 0.76, 1.0, 0.86],
    [0.84, 1.0, 0.92, 0.75, 1.0, 0.96, 0.8, 1.0, 0.88, 0.72, 0.98],
];

type V = (f32, f32);

/// `a - b` the short way round the wrapped field.
fn wrapped_delta(a: V, b: V) -> V {
    (
        (a.0 - b.0 + W / 2.).rem_euclid(W) - W / 2.,
        (a.1 - b.1 + H / 2.).rem_euclid(H) - H / 2.,
    )
}

fn touching(a: V, b: V, r: f32) -> bool {
    let (dx, dy) = wrapped_delta(a, b);
    dx * dx + dy * dy < r * r
}

fn wrap(p: V) -> V {
    (p.0.rem_euclid(W), p.1.rem_euclid(H))
}

/// The ship's nose direction for a heading (0 up, clockwise).
fn heading(angle: f32) -> V {
    (angle.sin(), -angle.cos())
}

fn rand_range(rng: &mut u64, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rand01(rng)
}

#[derive(Debug, Clone, Copy)]
struct Ship {
    pos: V,
    vel: V,
    /// The heading shown and flown; it eases towards `aim`, which the wheel sets.
    angle: f32,
    aim: f32,
    alive: bool,
    /// Seconds left in hyperspace (gone from the field meanwhile).
    hyper: f32,
}

impl Ship {
    fn new() -> Self {
        Self {
            pos: (W / 2., H / 2.),
            vel: (0., 0.),
            angle: 0.,
            aim: 0.,
            alive: true,
            hyper: 0.,
        }
    }

    fn present(&self) -> bool {
        self.alive && self.hyper <= 0.
    }
}

#[derive(Debug, Clone, Copy)]
struct Rock {
    pos: V,
    vel: V,
    size: u8,
    shape: u8,
    turn: f32,
    spin: f32,
}

#[derive(Debug, Clone, Copy)]
struct Shot {
    pos: V,
    vel: V,
    life: f32,
}

#[derive(Debug, Clone, Copy)]
struct Saucer {
    pos: V,
    vel: V,
    small: bool,
    fire_in: f32,
    veer_in: f32,
}

impl Saucer {
    fn radius(&self) -> f32 {
        if self.small {
            7.
        } else {
            11.
        }
    }
}

/// A fragment flying from something destroyed: a spark (`len` 0) or a line.
#[derive(Debug, Clone, Copy)]
struct Debris {
    pos: V,
    vel: V,
    angle: f32,
    spin: f32,
    len: f32,
    life: f32,
    color: u32,
}

pub struct Rocks {
    pub score: u32,
    pub lives: u8,
    pub wave: u32,
    /// The multiplier on rock points: hits in a row, from 1.
    pub streak: u32,
    next_ship: u32,
    ship: Ship,
    shots: Vec<Shot>,
    rocks: Vec<Rock>,
    saucer: Option<Saucer>,
    saucer_shots: Vec<Shot>,
    debris: Vec<Debris>,
    pub(crate) thrust: bool,
    pub(crate) fire: bool,
    fire_cool: f32,
    respawn: f32,
    /// Counting down to the next wave once the field is clear.
    wave_gap: Option<f32>,
    saucer_in: f32,
    /// Counting down to "game over" once the last ship is gone.
    over_in: Option<f32>,
    over: bool,
    /// Seconds of play (flicker).
    time: f32,
}

impl Rocks {
    pub fn new(rng: &mut u64) -> Self {
        let mut r = Self {
            score: 0,
            lives: 3,
            wave: 1,
            streak: 1,
            next_ship: SHIP_EVERY,
            ship: Ship::new(),
            shots: Vec::new(),
            rocks: Vec::new(),
            saucer: None,
            saucer_shots: Vec::new(),
            debris: Vec::new(),
            thrust: false,
            fire: false,
            fire_cool: 0.,
            respawn: 0.,
            wave_gap: None,
            saucer_in: 0.,
            over_in: None,
            over: false,
            time: 0.,
        };
        r.spawn_wave(rng);
        r
    }

    pub fn over(&self) -> bool {
        self.over
    }

    /// Where things are, to compare runs in tests.
    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> Vec<(f32, f32)> {
        let mut at = vec![self.ship.pos, self.ship.vel];
        at.extend(self.rocks.iter().map(|r| r.pos));
        at.extend(self.shots.iter().map(|s| s.pos));
        at
    }

    /// Wheel detents, clockwise positive. One detent is a fine 6° nudge;
    /// a fast spin turns progressively further.
    pub fn turn(&mut self, detents: i8) {
        let n = usize::from(detents.unsigned_abs());
        if n == 0 {
            return;
        }
        let steps = TURN_ACCEL
            .get(n - 1)
            .copied()
            .unwrap_or(TURN_ACCEL[3] + 2. * n.saturating_sub(4) as f32);
        self.ship.aim += f32::from(detents.signum()) * steps * TURN_STEP;
    }

    pub fn set_thrust(&mut self, on: bool) {
        self.thrust = on;
    }

    /// The fire button: each press shoots at once; held, it repeats.
    pub fn set_fire(&mut self, on: bool) {
        if on && !self.fire {
            self.fire_cool = 0.;
        }
        self.fire = on;
    }

    /// Let go of everything (pausing, leaving).
    pub fn release(&mut self) {
        self.thrust = false;
        self.fire = false;
    }

    pub fn hyperspace(&mut self) {
        if self.ship.present() {
            self.ship.hyper = HYPER_S;
            self.ship.vel = (0., 0.);
        }
    }

    fn add(&mut self, points: u32) {
        self.score += points;
        while self.score >= self.next_ship {
            self.lives = (self.lives + 1).min(MAX_SHIPS);
            self.next_ship += SHIP_EVERY;
        }
    }

    /// A shot of the ship's hit a rock: its points times the streak, which
    /// then grows.
    fn hit_rock(&mut self, i: usize, rng: &mut u64) {
        let points = self.break_rock(i, rng) * self.streak;
        self.add(points);
        self.streak = (self.streak + 1).min(MAX_STREAK);
    }

    fn spawn_wave(&mut self, rng: &mut u64) {
        let count = (2 + 2 * self.wave).min(11);
        let speed = (1. + 0.07 * (self.wave - 1) as f32).min(1.6);
        self.rocks.clear();
        for _ in 0..count {
            // Anywhere well away from the ship.
            let mut pos = (0., 0.);
            for _ in 0..16 {
                pos = (rand_range(rng, 0., W), rand_range(rng, 0., H));
                if !touching(pos, self.ship.pos, 120.) {
                    break;
                }
            }
            let a = rand_range(rng, 0., TAU);
            let v = rand_range(rng, 30., 55.) * speed;
            self.rocks.push(Rock {
                pos,
                vel: (a.cos() * v, a.sin() * v),
                size: 3,
                shape: (next(rng) % SHAPES.len() as u64) as u8,
                turn: rand_range(rng, 0., TAU),
                spin: rand_range(rng, -0.8, 0.8),
            });
        }
        self.saucer_in = (20. - 2. * self.wave as f32).max(8.);
    }

    /// Breaks rock `i`: big and medium rocks split in two smaller, faster
    /// ones. Returns its points.
    fn break_rock(&mut self, i: usize, rng: &mut u64) -> u32 {
        let rock = self.rocks.swap_remove(i);
        self.sparks(rock.pos, 5, ROCK_COLOR, rng);
        if rock.size > 1 {
            let speed = (rock.vel.0.hypot(rock.vel.1) * rand_range(rng, 1.15, 1.45))
                .clamp(40. + 25. * f32::from(3 - rock.size), 150.);
            let base = rock.vel.1.atan2(rock.vel.0);
            for side in [-1., 1.] {
                let a = base + side * rand_range(rng, 0.3, 0.9);
                self.rocks.push(Rock {
                    pos: rock.pos,
                    vel: (a.cos() * speed, a.sin() * speed),
                    size: rock.size - 1,
                    shape: (next(rng) % SHAPES.len() as u64) as u8,
                    turn: rand_range(rng, 0., TAU),
                    spin: rand_range(rng, -1.6, 1.6),
                });
            }
        }
        ROCK_POINTS[usize::from(rock.size)]
    }

    fn sparks(&mut self, pos: V, n: usize, color: u32, rng: &mut u64) {
        for _ in 0..n {
            let a = rand_range(rng, 0., TAU);
            let v = rand_range(rng, 30., 110.);
            self.debris.push(Debris {
                pos,
                vel: (a.cos() * v, a.sin() * v),
                angle: 0.,
                spin: 0.,
                len: 0.,
                life: rand_range(rng, 0.3, 0.7),
                color,
            });
        }
        let extra = self.debris.len().saturating_sub(MAX_DEBRIS);
        self.debris.drain(..extra);
    }

    fn kill_ship(&mut self, rng: &mut u64, color: u32) {
        let ship = self.ship;
        self.ship.alive = false;
        self.ship.hyper = 0.;
        self.respawn = RESPAWN_S;
        self.lives = self.lives.saturating_sub(1);
        self.streak = 1;
        let outline = ship_outline(ship.pos, ship.angle);
        for k in 0..outline.len() {
            let (a, b) = (outline[k], outline[(k + 1) % outline.len()]);
            let mid = ((a.0 + b.0) / 2., (a.1 + b.1) / 2.);
            let out = rand_range(rng, 0., TAU);
            self.debris.push(Debris {
                pos: mid,
                vel: (
                    ship.vel.0 * 0.3 + out.cos() * 40.,
                    ship.vel.1 * 0.3 + out.sin() * 40.,
                ),
                angle: (b.1 - a.1).atan2(b.0 - a.0),
                spin: rand_range(rng, -4., 4.),
                len: (b.0 - a.0).hypot(b.1 - a.1),
                life: 1.4,
                color,
            });
        }
        self.sparks(ship.pos, 8, color, rng);
        if self.lives == 0 {
            self.over_in = Some(OVER_S);
        }
    }

    /// One fixed step of `dt` seconds.
    pub fn step(&mut self, dt: f32, rng: &mut u64) {
        self.time += dt;
        self.step_ship(dt, rng);
        for s in self.shots.iter_mut().chain(self.saucer_shots.iter_mut()) {
            s.pos = wrap((s.pos.0 + s.vel.0 * dt, s.pos.1 + s.vel.1 * dt));
            s.life -= dt;
        }
        // A shot that flies out without hitting anything ends the streak.
        if self.shots.iter().any(|s| s.life <= 0.) {
            self.streak = 1;
        }
        self.shots.retain(|s| s.life > 0.);
        self.saucer_shots.retain(|s| s.life > 0.);
        for r in &mut self.rocks {
            r.pos = wrap((r.pos.0 + r.vel.0 * dt, r.pos.1 + r.vel.1 * dt));
            r.turn += r.spin * dt;
        }
        for d in &mut self.debris {
            d.pos = (d.pos.0 + d.vel.0 * dt, d.pos.1 + d.vel.1 * dt);
            d.angle += d.spin * dt;
            d.life -= dt;
        }
        self.debris.retain(|d| d.life > 0.);
        self.step_saucer(dt, rng);
        self.collide(rng);
        // The next wave once the field is clear.
        if self.rocks.is_empty() && self.saucer.is_none() {
            if self.wave_gap.is_none() {
                self.add(WAVE_BONUS * self.wave);
            }
            let left = self.wave_gap.get_or_insert(WAVE_GAP_S);
            *left -= dt;
            if *left <= 0. {
                self.wave_gap = None;
                self.wave += 1;
                self.spawn_wave(rng);
            }
        }
        if let Some(t) = &mut self.over_in {
            *t -= dt;
            if *t <= 0. {
                self.over = true;
            }
        }
    }

    fn step_ship(&mut self, dt: f32, rng: &mut u64) {
        let s = &mut self.ship;
        s.angle += (s.aim - s.angle) * (1. - (-AIM_RATE * dt).exp());
        if !s.alive {
            self.respawn -= dt;
            let clear = self.rocks.iter().all(|r| {
                !touching(
                    r.pos,
                    (W / 2., H / 2.),
                    SAFE_R + ROCK_R[usize::from(r.size)],
                )
            }) && self.saucer.is_none();
            if self.respawn <= 0. && self.lives > 0 && clear {
                let aim = s.aim;
                *s = Ship::new();
                s.aim = aim;
                s.angle = aim;
            }
            return;
        }
        if s.hyper > 0. {
            s.hyper -= dt;
            if s.hyper <= 0. {
                s.pos = (rand_range(rng, 0., W), rand_range(rng, 0., H));
                if next(rng).is_multiple_of(HYPER_RISK) {
                    self.kill_ship(rng, ROCK_COLOR);
                }
            }
            return;
        }
        if self.thrust {
            let (hx, hy) = heading(s.angle);
            s.vel.0 += hx * THRUST * dt;
            s.vel.1 += hy * THRUST * dt;
        }
        let keep = (-DRAG * dt).exp();
        s.vel = (s.vel.0 * keep, s.vel.1 * keep);
        let speed = s.vel.0.hypot(s.vel.1);
        if speed > MAX_SPEED {
            s.vel = (s.vel.0 * MAX_SPEED / speed, s.vel.1 * MAX_SPEED / speed);
        }
        s.pos = wrap((s.pos.0 + s.vel.0 * dt, s.pos.1 + s.vel.1 * dt));
        self.fire_cool -= dt;
        if self.fire && self.fire_cool <= 0. && self.shots.len() < MAX_SHOTS {
            let (hx, hy) = heading(s.angle);
            self.shots.push(Shot {
                pos: wrap((s.pos.0 + hx * 11., s.pos.1 + hy * 11.)),
                vel: (s.vel.0 + hx * SHOT_SPEED, s.vel.1 + hy * SHOT_SPEED),
                life: SHOT_LIFE,
            });
            self.fire_cool = REFIRE;
        }
    }

    fn step_saucer(&mut self, dt: f32, rng: &mut u64) {
        if self.saucer.is_none() {
            if self.wave >= SAUCER_FROM_WAVE && self.ship.alive && !self.rocks.is_empty() {
                self.saucer_in -= dt;
                if self.saucer_in <= 0. {
                    let small = self.wave >= SMALL_SAUCER_FROM_WAVE;
                    let speed = if small { 95. } else { 70. };
                    let from_left = next(rng) & 1 == 0;
                    self.saucer = Some(Saucer {
                        pos: (
                            if from_left { 0. } else { W },
                            rand_range(rng, 0.15 * H, 0.85 * H),
                        ),
                        vel: (if from_left { speed } else { -speed }, 0.),
                        small,
                        fire_in: 1.,
                        veer_in: 1.3,
                    });
                }
            }
            return;
        }
        let ship = self.ship;
        let accuracy = (0.35 - 0.03 * self.wave as f32).max(0.08);
        let Some(u) = self.saucer.as_mut() else {
            return;
        };
        u.pos.0 += u.vel.0 * dt;
        u.pos.1 = (u.pos.1 + u.vel.1 * dt).rem_euclid(H);
        u.veer_in -= dt;
        if u.veer_in <= 0. {
            u.veer_in = 1.3;
            u.vel.1 = [-50., 0., 50.][(next(rng) % 3) as usize];
        }
        u.fire_in -= dt;
        if u.fire_in <= 0. && self.saucer_shots.len() < 3 {
            // The big saucer fires anywhere; the small one aims.
            let a = if u.small && ship.present() {
                let (dx, dy) = wrapped_delta(ship.pos, u.pos);
                dy.atan2(dx) + rand_range(rng, -accuracy, accuracy)
            } else {
                rand_range(rng, 0., TAU)
            };
            u.fire_in = if u.small { 1.0 } else { 1.2 };
            self.saucer_shots.push(Shot {
                pos: u.pos,
                vel: (a.cos() * SAUCER_SHOT_SPEED, a.sin() * SAUCER_SHOT_SPEED),
                life: SAUCER_SHOT_LIFE,
            });
        }
        let r = u.radius();
        if u.pos.0 < -r || u.pos.0 > W + r {
            // Gone off the far side.
            self.saucer = None;
            self.saucer_in = (20. - 2. * self.wave as f32).max(8.);
        }
    }

    fn collide(&mut self, rng: &mut u64) {
        // The ship's shots.
        let mut i = 0;
        while i < self.shots.len() {
            let p = self.shots[i].pos;
            if let Some(k) = self
                .rocks
                .iter()
                .position(|r| touching(p, r.pos, ROCK_R[usize::from(r.size)]))
            {
                self.shots.swap_remove(i);
                self.hit_rock(k, rng);
                continue;
            }
            if let Some(u) = self.saucer.filter(|u| touching(p, u.pos, u.radius() + 1.)) {
                self.shots.swap_remove(i);
                self.saucer = None;
                self.saucer_in = (20. - 2. * self.wave as f32).max(8.);
                self.sparks(u.pos, 10, SAUCER_COLOR, rng);
                self.add(SAUCER_POINTS[usize::from(u.small)]);
                self.streak = (self.streak + 1).min(MAX_STREAK);
                continue;
            }
            i += 1;
        }
        // The saucer's shots break rocks too (for no points).
        let mut i = 0;
        while i < self.saucer_shots.len() {
            let p = self.saucer_shots[i].pos;
            if let Some(k) = self
                .rocks
                .iter()
                .position(|r| touching(p, r.pos, ROCK_R[usize::from(r.size)]))
            {
                self.saucer_shots.swap_remove(i);
                self.break_rock(k, rng);
                continue;
            }
            i += 1;
        }
        // The saucer flying into a rock.
        if let Some(u) = self.saucer {
            if let Some(k) = self
                .rocks
                .iter()
                .position(|r| touching(u.pos, r.pos, ROCK_R[usize::from(r.size)] + u.radius()))
            {
                self.break_rock(k, rng);
                self.saucer = None;
                self.saucer_in = (20. - 2. * self.wave as f32).max(8.);
                self.sparks(u.pos, 10, SAUCER_COLOR, rng);
            }
        }
        if !self.ship.present() {
            return;
        }
        let at = self.ship.pos;
        if let Some(k) = self
            .rocks
            .iter()
            .position(|r| touching(at, r.pos, ROCK_R[usize::from(r.size)] + SHIP_R * 0.8))
        {
            let points = self.break_rock(k, rng);
            self.add(points);
            self.kill_ship(rng, ROCK_COLOR);
            return;
        }
        if let Some(u) = self
            .saucer
            .filter(|u| touching(at, u.pos, u.radius() + SHIP_R))
        {
            self.saucer = None;
            self.saucer_in = (20. - 2. * self.wave as f32).max(8.);
            self.sparks(u.pos, 10, SAUCER_COLOR, rng);
            self.add(SAUCER_POINTS[usize::from(u.small)]);
            self.kill_ship(rng, ROCK_COLOR);
            return;
        }
        if let Some(i) = self
            .saucer_shots
            .iter()
            .position(|s| touching(at, s.pos, SHIP_R))
        {
            self.saucer_shots.swap_remove(i);
            self.kill_ship(rng, ROCK_COLOR);
        }
    }
}

/// The ship's outline in field pixels: an arrowhead with a notched tail.
fn ship_outline(pos: V, angle: f32) -> [V; 4] {
    let (s, c) = angle.sin_cos();
    [(0., -11.), (7.5, 8.), (0., 4.5), (-7.5, 8.)]
        .map(|(x, y)| (pos.0 + x * c - y * s, pos.1 + x * s + y * c))
}

// ------------------------------------------------------------------ drawing

/// Clips the segment a-b to the field (Liang-Barsky).
fn clip(a: V, b: V) -> Option<(V, V)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0f32, 1f32);
    for (p, q) in [(-dx, a.0), (dx, W - a.0), (-dy, a.1), (dy, H - a.1)] {
        if p == 0. {
            if q < 0. {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0. {
            t0 = t0.max(r);
        } else {
            t1 = t1.min(r);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((
        (a.0 + t0 * dx, a.1 + t0 * dy),
        (a.0 + t1 * dx, a.1 + t1 * dy),
    ))
}

/// A straight line `width` px wide, as one warped quad lying along it.
fn bar(c: &mut Canvas, a: V, b: V, width: f32, color: u32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len < 0.01 {
        return;
    }
    let h = width / 2.;
    let (nx, ny) = (-dy / len * h, dx / len * h);
    // Ends reach a little past the points so outlines join without gaps
    // (but not so far that a clipped line pokes out of the field).
    let reach = h.min(0.8) / len;
    let (ux, uy) = (dx * reach, dy * reach);
    let (ax, ay) = (FIELD_X + a.0 - ux, FIELD_Y + a.1 - uy);
    let (bx, by) = (FIELD_X + b.0 + ux, FIELD_Y + b.1 + uy);
    let corners = [
        [ax - nx, ay - ny],
        [bx - nx, by - ny],
        [bx + nx, by + ny],
        [ax + nx, ay + ny],
    ];
    // The box is the line's bounds; the renderer draws the warped corners.
    let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
    let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let (xs, ys) = (corners.map(|p| p[0]), corners.map(|p| p[1]));
    let (x, y) = (min(xs), min(ys));
    c.draw.push(Quad {
        warp: Some(Warp {
            corners,
            colors: [color; 4],
        }),
        ..Quad::rect(x, y, max(xs) - x, max(ys) - y, color)
    });
}

/// A glowing vector line: a soft wide stroke under a thin bright one.
fn line(c: &mut Canvas, a: V, b: V, color: u32) {
    if let Some((a, b)) = clip(a, b) {
        bar(c, a, b, 6., alpha(color, 0x38));
        bar(c, a, b, 1.8, color);
    }
}

/// Copies of something of radius `r` at `pos`, one per field edge it
/// overlaps, so it shows on both sides while it wraps.
fn copies(pos: V, r: f32) -> impl Iterator<Item = V> {
    let xs = [
        0.,
        if pos.0 < r { W } else { 0. },
        if pos.0 > W - r { -W } else { 0. },
    ];
    let ys = [
        0.,
        if pos.1 < r { H } else { 0. },
        if pos.1 > H - r { -H } else { 0. },
    ];
    let mut out = Vec::with_capacity(4);
    for (i, x) in xs.into_iter().enumerate() {
        for (j, y) in ys.into_iter().enumerate() {
            if (i == 0 || x != 0.) && (j == 0 || y != 0.) {
                out.push((x, y));
            }
        }
    }
    out.into_iter()
}

fn outline(c: &mut Canvas, points: &[V], centre: V, r: f32, color: u32) {
    for (ox, oy) in copies(centre, r) {
        for k in 0..points.len() {
            let (a, b) = (points[k], points[(k + 1) % points.len()]);
            line(c, (a.0 + ox, a.1 + oy), (b.0 + ox, b.1 + oy), color);
        }
    }
}

/// Faint fixed stars behind everything.
fn stars(c: &mut Canvas) {
    let mut seed = 0x5EED_u64;
    for _ in 0..28 {
        let x = rand01(&mut seed) * W;
        let y = rand01(&mut seed) * H;
        let a = 0x30 + (rand01(&mut seed) * 0x40 as f32) as u8;
        c.rect(FIELD_X + x, FIELD_Y + y, 1., 1., alpha(0xC8D4FFFF, a));
    }
}

/// The score strip's middle text.
pub(crate) fn status(r: &Rocks) -> String {
    let ships = if r.lives == 1 { "ship" } else { "ships" };
    let mut s = format!("Wave {} · {} {ships}", r.wave, r.lives);
    if r.streak > 1 {
        s += &format!(" · x{}", r.streak);
    }
    s
}

pub(crate) fn draw(c: &mut Canvas, r: &Rocks, accent: u32) {
    c.rrect(FIELD_X, FIELD_Y, W, H, 10., FIELD_BG);
    c.rrect_outline(FIELD_X, FIELD_Y, W, H, 10., 1., alpha(accent, 0x50));
    stars(c);
    for rock in &r.rocks {
        let radius = ROCK_R[usize::from(rock.size)];
        let shape = &SHAPES[usize::from(rock.shape)];
        let points: Vec<V> = shape
            .iter()
            .enumerate()
            .map(|(k, m)| {
                let a = rock.turn + k as f32 * TAU / shape.len() as f32;
                (
                    rock.pos.0 + a.cos() * radius * m,
                    rock.pos.1 + a.sin() * radius * m,
                )
            })
            .collect();
        outline(c, &points, rock.pos, radius, ROCK_COLOR);
    }
    if let Some(u) = &r.saucer {
        let s = u.radius();
        let (x, y) = u.pos;
        let hull = [
            (x - s, y),
            (x - s * 0.5, y - s * 0.4),
            (x + s * 0.5, y - s * 0.4),
            (x + s, y),
            (x + s * 0.5, y + s * 0.45),
            (x - s * 0.5, y + s * 0.45),
        ];
        outline(c, &hull, u.pos, s, SAUCER_COLOR);
        let dome = [
            (x - s * 0.3, y - s * 0.4),
            (x - s * 0.2, y - s * 0.8),
            (x + s * 0.2, y - s * 0.8),
            (x + s * 0.3, y - s * 0.4),
        ];
        for k in 0..3 {
            line(c, dome[k], dome[k + 1], SAUCER_COLOR);
        }
        line(c, (x - s, y), (x + s, y), SAUCER_COLOR);
    }
    let ship_color = mix(accent, 0xFFFFFFFF, 0.45);
    if r.ship.present() {
        let s = &r.ship;
        outline(c, &ship_outline(s.pos, s.angle), s.pos, 12., ship_color);
        if r.thrust && (r.time * 24.) as i32 % 2 == 0 {
            let (sn, cs) = s.angle.sin_cos();
            let flame: Vec<V> = [(-3.5, 6.5), (0., 13.5), (3.5, 6.5)]
                .iter()
                .map(|(x, y)| (s.pos.0 + x * cs - y * sn, s.pos.1 + x * sn + y * cs))
                .collect();
            for (ox, oy) in copies(s.pos, 16.) {
                for k in 0..2 {
                    let (a, b) = (flame[k], flame[k + 1]);
                    line(c, (a.0 + ox, a.1 + oy), (b.0 + ox, b.1 + oy), 0xFFB45AFF);
                }
            }
        }
    }
    for (shots, color) in [(&r.shots, SHOT_COLOR), (&r.saucer_shots, SAUCER_COLOR)] {
        for s in shots.iter() {
            c.rect(
                FIELD_X + s.pos.0 - 2.5,
                FIELD_Y + s.pos.1 - 2.5,
                5.,
                5.,
                alpha(color, 0x40),
            );
            c.rect(
                FIELD_X + s.pos.0 - 1.,
                FIELD_Y + s.pos.1 - 1.,
                2.,
                2.,
                color,
            );
        }
    }
    for d in &r.debris {
        let fade = alpha(d.color, (255. * (d.life / 0.7).min(1.)) as u8);
        if d.len > 0. {
            let (dx, dy) = (d.angle.cos() * d.len / 2., d.angle.sin() * d.len / 2.);
            line(
                c,
                (d.pos.0 - dx, d.pos.1 - dy),
                (d.pos.0 + dx, d.pos.1 + dy),
                fade,
            );
        } else if (0. ..W).contains(&d.pos.0) && (0. ..H).contains(&d.pos.1) {
            c.rect(FIELD_X + d.pos.0 - 1., FIELD_Y + d.pos.1 - 1., 2., 2., fade);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1. / 60.;

    fn run(r: &mut Rocks, seconds: f32, rng: &mut u64) {
        for _ in 0..(seconds / DT).round() as usize {
            r.step(DT, rng);
        }
    }

    /// A field with nothing in it but the ship.
    fn empty(rng: &mut u64) -> Rocks {
        let mut r = Rocks::new(rng);
        r.rocks.clear();
        r.wave_gap = Some(1000.);
        r
    }

    fn rock(pos: V, vel: V, size: u8) -> Rock {
        Rock {
            pos,
            vel,
            size,
            shape: 0,
            turn: 0.,
            spin: 0.,
        }
    }

    #[test]
    fn everything_wraps_round_the_field() {
        assert_eq!(wrap((-1., H + 2.)), (W - 1., 2.));
        // Distances go the short way across an edge.
        assert_eq!(wrapped_delta((2., 5.), (W - 3., 5.)), (5., 0.));
        assert!(touching((1., 1.), (W - 1., H - 1.), 3.));
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.rocks.push(rock((W - 1., 10.), (60., 0.), 3));
        r.step(0.1, &mut rng);
        assert!((r.rocks[0].pos.0 - 5.).abs() < 0.01, "{:?}", r.rocks[0].pos);
    }

    #[test]
    fn the_wheel_turns_smoothly_and_faster_spins_turn_further() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.turn(1);
        assert!((r.ship.aim - TURN_STEP).abs() < 1e-6);
        assert_eq!(r.ship.angle, 0., "eases, not a jump");
        r.step(DT, &mut rng);
        let first = r.ship.angle;
        assert!(first > 0. && first < TURN_STEP);
        run(&mut r, 0.25, &mut rng);
        assert!((r.ship.angle - TURN_STEP).abs() < 1e-3);
        // Four detents in one report turn seven steps, not four.
        r.turn(-4);
        assert!((r.ship.aim - TURN_STEP * (1. - 7.)).abs() < 1e-5);
    }

    #[test]
    fn thrust_builds_speed_and_drag_lets_it_coast_down() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.set_thrust(true);
        run(&mut r, 0.5, &mut rng);
        let up = -r.ship.vel.1;
        assert!(up > 100., "{up}");
        assert!(r.ship.vel.0.abs() < 1e-3, "straight up");
        run(&mut r, 5., &mut rng);
        assert!(r.ship.vel.0.hypot(r.ship.vel.1) <= MAX_SPEED + 0.01);
        r.set_thrust(false);
        let fast = r.ship.vel.1.abs();
        run(&mut r, 1., &mut rng);
        let slower = r.ship.vel.1.abs();
        assert!(
            slower < fast * 0.6 && slower > fast * 0.4,
            "{fast} {slower}"
        );
    }

    #[test]
    fn fire_shoots_on_press_repeats_when_held_and_caps_at_four() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.set_fire(true);
        r.step(DT, &mut rng);
        assert_eq!(r.shots.len(), 1, "at once");
        run(&mut r, REFIRE - 2. * DT, &mut rng);
        assert_eq!(r.shots.len(), 1, "not faster than the repeat rate");
        run(&mut r, 3. * DT, &mut rng);
        assert_eq!(r.shots.len(), 2);
        run(&mut r, 0.6, &mut rng);
        assert!(r.shots.len() <= MAX_SHOTS);
        // Shots expire.
        r.set_fire(false);
        run(&mut r, SHOT_LIFE + 0.1, &mut rng);
        assert!(r.shots.is_empty());
        // Taps fire as fast as they come (up to the cap).
        for _ in 0..3 {
            r.set_fire(true);
            r.step(DT, &mut rng);
            r.set_fire(false);
            r.step(DT, &mut rng);
        }
        assert_eq!(r.shots.len(), 3);
    }

    #[test]
    fn big_rocks_split_into_medium_then_small_then_dust() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.rocks.push(rock((100., 100.), (30., 0.), 3));
        let hit = |r: &mut Rocks, rng: &mut u64| {
            let target = r.rocks[0].pos;
            r.shots.push(Shot {
                pos: target,
                vel: (0., 0.),
                life: 1.,
            });
            r.collide(rng);
        };
        hit(&mut r, &mut rng);
        assert_eq!(r.score, 10);
        assert_eq!(r.rocks.len(), 2);
        assert!(r.rocks.iter().all(|k| k.size == 2));
        let parent = 30.;
        assert!(r.rocks.iter().all(|k| k.vel.0.hypot(k.vel.1) > parent));
        // Each hit in a row multiplies the next: 20 x2, then 30 x3.
        hit(&mut r, &mut rng);
        assert_eq!(r.score, 10 + 40);
        assert_eq!(r.rocks.iter().filter(|k| k.size == 1).count(), 2);
        // The small one goes for good.
        let i = r.rocks.iter().position(|k| k.size == 1).unwrap();
        r.rocks.swap(0, i);
        hit(&mut r, &mut rng);
        assert_eq!(r.score, 10 + 40 + 90);
        assert_eq!(r.rocks.len(), 2);
        assert!(r.shots.is_empty());
    }

    #[test]
    fn the_streak_tops_out_at_five_and_a_missed_shot_ends_it() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        for _ in 0..7 {
            r.rocks.push(rock((100., 100.), (0., 0.), 1));
            r.shots.push(Shot {
                pos: (100., 100.),
                vel: (0., 0.),
                life: 1.,
            });
            r.collide(&mut rng);
        }
        // 30 x1, x2, x3, x4, then x5 three times.
        assert_eq!(r.score, 30 * (1 + 2 + 3 + 4 + 5 * 3));
        assert_eq!(r.streak, MAX_STREAK);
        assert!(status(&r).ends_with("x5"));
        // A shot into empty space runs out and the count starts again.
        r.set_fire(true);
        r.step(DT, &mut rng);
        r.set_fire(false);
        run(&mut r, SHOT_LIFE + 0.1, &mut rng);
        assert_eq!(r.streak, 1);
        assert!(!status(&r).contains('x'));
    }

    #[test]
    fn hitting_a_rock_costs_a_ship_and_the_next_waits_for_a_clear_centre() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.rocks.push(rock((W / 2. + 20., H / 2.), (0., 0.), 3));
        r.step(DT, &mut rng);
        assert!(!r.ship.alive);
        assert_eq!(r.lives, 2);
        // A medium piece sits on the centre: no new ship until it moves off.
        r.rocks.retain(|k| k.size == 2);
        r.rocks.truncate(1);
        r.rocks[0].pos = (W / 2., H / 2.);
        r.rocks[0].vel = (0., 0.);
        run(&mut r, RESPAWN_S + 1., &mut rng);
        assert!(!r.ship.alive);
        r.rocks[0].pos = (20., 20.);
        r.step(DT, &mut rng);
        assert!(r.ship.alive);
        assert_eq!(r.ship.pos, (W / 2., H / 2.));
    }

    #[test]
    fn the_third_lost_ship_ends_the_game() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        for lives in (0..3).rev() {
            r.rocks.push(rock(r.ship.pos, (0., 0.), 1));
            r.step(DT, &mut rng);
            assert_eq!(r.lives, lives);
            r.rocks.clear();
            run(&mut r, RESPAWN_S + 0.1, &mut rng);
        }
        assert!(!r.ship.alive);
        run(&mut r, OVER_S, &mut rng);
        assert!(r.over());
    }

    #[test]
    fn a_ship_comes_every_five_thousand_keeping_at_most_five() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.add(4_990);
        assert_eq!(r.lives, 3);
        r.add(20);
        assert_eq!(r.lives, 4);
        r.add(5_000);
        assert_eq!(r.lives, 5);
        r.add(5_000);
        assert_eq!(r.lives, 5, "five at most");
    }

    #[test]
    fn hyperspace_jumps_somewhere_else_and_can_go_wrong() {
        let mut outcomes = (0, 0);
        for seed in 1..200u64 {
            let mut rng = seed;
            let mut r = empty(&mut rng);
            r.hyperspace();
            assert!(!r.ship.present());
            run(&mut r, HYPER_S + DT, &mut rng);
            if r.ship.alive {
                outcomes.0 += 1;
                assert!(r.ship.present());
                assert_ne!(r.ship.pos, (W / 2., H / 2.));
            } else {
                outcomes.1 += 1;
            }
        }
        assert!(outcomes.0 > 150 && outcomes.1 > 5, "{outcomes:?}");
    }

    #[test]
    fn clearing_a_wave_brings_a_bigger_one() {
        let mut rng = 3;
        let mut r = Rocks::new(&mut rng);
        assert_eq!(r.rocks.len(), 4);
        assert!(r.rocks.iter().all(|k| !touching(k.pos, r.ship.pos, 100.)));
        r.rocks.clear();
        run(&mut r, WAVE_GAP_S + 0.1, &mut rng);
        assert_eq!(r.score, WAVE_BONUS, "wave 1 cleared");
        assert_eq!(r.wave, 2);
        assert_eq!(r.rocks.len(), 6);
    }

    #[test]
    fn saucers_come_from_wave_two_and_the_small_one_aims() {
        let mut rng = 3;
        let mut r = empty(&mut rng);
        r.rocks.push(rock((30., 30.), (0., 0.), 1));
        run(&mut r, 30., &mut rng);
        assert!(r.saucer.is_none(), "not on wave one");
        r.wave = 4;
        r.saucer_in = 0.;
        r.step(DT, &mut rng);
        let u = r.saucer.expect("a saucer");
        assert!(u.small);
        // Put it level with the ship and let it fire.
        r.saucer.as_mut().unwrap().pos = (W / 2. - 100., H / 2.);
        r.saucer.as_mut().unwrap().vel = (0., 0.);
        r.saucer.as_mut().unwrap().fire_in = 0.;
        r.step(DT, &mut rng);
        let shot = r.saucer_shots[0];
        let angle = shot.vel.1.atan2(shot.vel.0);
        assert!(angle.abs() < 0.3, "aimed at the ship: {angle}");
        // Shooting it scores 750.
        let at = r.saucer.unwrap().pos;
        r.shots.push(Shot {
            pos: at,
            vel: (0., 0.),
            life: 1.,
        });
        r.collide(&mut rng);
        assert!(r.saucer.is_none());
        assert_eq!(r.score, SAUCER_POINTS[1]);
    }

    #[test]
    fn the_same_seed_plays_the_same_game() {
        let play = |seed: u64| {
            let mut rng = seed;
            let mut r = Rocks::new(&mut rng);
            r.set_fire(true);
            for k in 0..600 {
                if k % 40 == 0 {
                    r.turn(3);
                }
                r.set_thrust(k % 90 < 30);
                r.step(DT, &mut rng);
            }
            (r.score, r.rocks.len(), r.ship.pos, r.lives)
        };
        assert_eq!(play(11), play(11));
        assert_ne!(play(11), play(12));
    }

    #[test]
    fn lines_are_clipped_to_the_field() {
        assert_eq!(clip((-10., 10.), (10., 10.)), Some(((0., 10.), (10., 10.))));
        assert_eq!(clip((-10., -10.), (-5., -1.)), None);
        let mut c = Canvas::new();
        line(&mut c, (W - 5., 20.), (W + 30., 20.), ROCK_COLOR);
        for q in c.finish() {
            assert!(q.x + q.w <= FIELD_X + W + 2., "{q:?}");
        }
    }
}

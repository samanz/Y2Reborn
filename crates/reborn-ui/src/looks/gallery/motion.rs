//! Gallery's motion: values that glide to their targets along one
//! emphasised ease, and the clock that keeps frames coming at the panel's
//! rate while anything still moves.
//!
//! Everything here runs on the time `super::now` gives: `None` in previews
//! and tests (`Ui::motion_fixed`), where every value is drawn at rest.
use std::cell::Cell;
use std::time::{Duration, Instant};

/// The emphasised ease, cubic-bezier(0.2, 0, 0, 1), at `t` (0..1).
pub(crate) fn ease(t: f32) -> f32 {
    if t <= 0. {
        return 0.;
    }
    if t >= 1. {
        return 1.;
    }
    let curve = |u: f32, a: f32, b: f32| {
        3. * (1. - u) * (1. - u) * u * a + 3. * (1. - u) * u * u * b + u * u * u
    };
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if curve(mid, 0.2, 0.) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    curve((lo + hi) / 2., 0., 1.)
}

fn elapsed(since: Instant, now: Instant, ms: u32) -> f32 {
    now.saturating_duration_since(since).as_secs_f32() * 1000. / ms.max(1) as f32
}

/// When the last running transition ends.
#[derive(Default)]
pub(crate) struct Clock(Cell<Option<Instant>>);

impl Clock {
    /// Something moves until `since + ms` (and one frame more, to draw it
    /// at rest).
    pub(crate) fn hold(&self, since: Instant, ms: u32) {
        let end = since + Duration::from_millis(u64::from(ms) + 40);
        if self.0.get().is_none_or(|u| u < end) {
            self.0.set(Some(end));
        }
    }

    pub(crate) fn moving(&self) -> bool {
        self.0.get().is_some_and(|u| Instant::now() < u)
    }
}

#[derive(Clone, Copy)]
struct Leg {
    from: f32,
    to: f32,
    since: Option<Instant>,
}

impl Leg {
    fn at(self, now: Instant, ms: u32) -> f32 {
        match self.since {
            Some(since) => self.from + (self.to - self.from) * ease(elapsed(since, now, ms)),
            None => self.to,
        }
    }
}

/// A value gliding toward its target.
#[derive(Default)]
pub(crate) struct Glide(Cell<Option<Leg>>);

impl Glide {
    /// Aim at `to` and return the value at `now`. The first value, and every
    /// value without a clock, is at rest.
    pub(crate) fn toward(&self, to: f32, now: Option<Instant>, ms: u32, clock: &Clock) -> f32 {
        let Some(now) = now else {
            self.0.set(Some(Leg {
                from: to,
                to,
                since: None,
            }));
            return to;
        };
        let leg = match self.0.get() {
            Some(leg) if (leg.to - to).abs() > 0.01 => Leg {
                from: leg.at(now, ms),
                to,
                since: Some(now),
            },
            Some(leg) => leg,
            None => Leg {
                from: to,
                to,
                since: None,
            },
        };
        self.0.set(Some(leg));
        if let Some(since) = leg.since {
            clock.hold(since, ms);
        }
        leg.at(now, ms)
    }

    /// Where the value is heading (nothing before its first frame).
    pub(crate) fn target(&self) -> Option<f32> {
        self.0.get().map(|l| l.to)
    }
}

/// A 0..1 entrance that restarts whenever `key` changes (a crossfade
/// between tabs, the focus glow arriving on a new card).
#[derive(Default)]
pub(crate) struct Fade {
    key: Cell<Option<usize>>,
    since: Cell<Option<Instant>>,
}

impl Fade {
    pub(crate) fn progress(&self, key: usize, now: Option<Instant>, ms: u32, clock: &Clock) -> f32 {
        if self.key.get() != Some(key) {
            let before = self.key.get().is_some();
            self.key.set(Some(key));
            self.since.set(now.filter(|_| before));
        }
        match (self.since.get(), now) {
            (Some(since), Some(now)) => {
                clock.hold(since, ms);
                ease(elapsed(since, now, ms))
            }
            _ => 1.,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ease_starts_quick_and_settles_softly() {
        assert_eq!(ease(0.), 0.);
        assert!((ease(1.) - 1.).abs() < 1e-4);
        assert!(ease(0.25) > 0.5, "emphasised: most of the way early");
        assert!(ease(0.5) < ease(0.6));
    }

    #[test]
    fn a_glide_rests_without_a_clock_and_moves_with_one() {
        let clock = Clock::default();
        let g = Glide::default();
        assert_eq!(g.toward(100., None, 300, &clock), 100.);
        assert!(!clock.moving());
        let now = Instant::now();
        assert_eq!(g.toward(100., Some(now), 300, &clock), 100.);
        let mid = g.toward(200., Some(now + Duration::from_millis(1)), 300, &clock);
        assert!((100. ..120.).contains(&mid), "{mid}");
        assert!(clock.moving());
        let end = g.toward(200., Some(now + Duration::from_millis(400)), 300, &clock);
        assert_eq!(end, 200.);
    }
}

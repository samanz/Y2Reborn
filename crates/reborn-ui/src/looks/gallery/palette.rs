//! The whole look's colour, taken from the album playing: a background
//! darkened from the cover, a deeper one it fades to, a raised surface, two
//! soft glows and a vivid accent. Every colour eases to the next album's
//! over 900 ms.
use super::motion::{ease, Clock};
use crate::theme::mix;
use std::cell::Cell;
use std::time::Instant;

/// Text, always: #F2F3F7.
pub(crate) const FG: u32 = 0xF2F3F7FF;
/// Secondary text: #A7ACBC.
pub(crate) const MUTED: u32 = 0xA7ACBCFF;
/// Disabled text and quiet marks.
pub(crate) const DIM: u32 = 0x7E8396FF;
/// Problems.
pub(crate) const DANGER: u32 = 0xE3605AFF;

/// How long the colours take to follow a new album.
pub(crate) const CHANGE_MS: u32 = 900;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Colors {
    pub bg: u32,
    pub bg_deep: u32,
    pub surface: u32,
    pub accent: u32,
    pub glow_a: u32,
    pub glow_b: u32,
}

/// Ink and periwinkle, before any cover is known.
pub(crate) const INK: Colors = Colors {
    bg: 0x0D0E13FF,
    bg_deep: 0x07070AFF,
    surface: 0x1A1B23FF,
    accent: 0x8D98FFFF,
    glow_a: 0x2A2E5CFF,
    glow_b: 0x1A1C3AFF,
};

fn rgb(c: u32) -> (f32, f32, f32) {
    let ch = |s: u32| ((c >> s) & 0xFF) as f32 / 255.;
    (ch(24), ch(16), ch(8))
}

fn pack(r: f32, g: f32, b: f32) -> u32 {
    let byte = |x: f32| (x * 255.).round().clamp(0., 255.) as u32;
    byte(r) << 24 | byte(g) << 16 | byte(b) << 8 | 0xFF
}

/// Relative luminance (WCAG).
pub(crate) fn luminance(c: u32) -> f32 {
    let lin = |x: f32| {
        if x <= 0.039_28 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = rgb(c);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast between two colours.
#[cfg(test)]
pub(crate) fn contrast(a: u32, b: u32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn hsv(c: u32) -> (f32, f32, f32) {
    let (r, g, b) = rgb(c);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d <= f32::EPSILON {
        0.
    } else if max == r {
        60. * ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    (h, if max <= 0. { 0. } else { d / max }, max)
}

fn from_hsv(h: f32, s: f32, v: f32) -> u32 {
    let k = |n: f32| {
        let k = (n + h.rem_euclid(360.) / 60.).rem_euclid(6.);
        v - v * s * k.min(4. - k).clamp(0., 1.)
    };
    pack(k(5.), k(3.), k(1.))
}

/// A cover colour raised to something that glows on a dark background,
/// turned by `degrees`; saturation is held below neon.
pub(crate) fn vivid(c: u32, degrees: f32) -> u32 {
    let (h, s, v) = hsv(c);
    from_hsv(
        h + degrees,
        (s * 1.15).clamp(0.35, 0.78),
        v.clamp(0.62, 0.92),
    )
}

/// `c` toward black by `f`.
pub(crate) fn darken(c: u32, f: f32) -> u32 {
    mix(c, 0x000000FF, f)
}

/// `c` darkened until text and muted text read on it (luminance at most
/// `max`, so #F2F3F7 stays above 7:1 and muted above 4.5:1).
fn deep_enough(c: u32, max: f32) -> u32 {
    let mut c = c;
    for _ in 0..20 {
        if luminance(c) <= max {
            break;
        }
        c = darken(c, 0.12);
    }
    c
}

/// The colours for the album playing, from its cover's accent (0 before one
/// is known).
pub(crate) fn of_cover(accent: u32) -> Colors {
    if accent == 0 {
        return INK;
    }
    let accent = vivid(accent, 0.);
    // The room is ink tinted by the cover, so the glows carry the colour and
    // the dark stays deep.
    let (h, s, v) = hsv(tamed(accent));
    let base = from_hsv(h, s.clamp(0.3, 0.75), v.max(0.5));
    Colors {
        bg: deep_enough(mix(INK.bg, base, 0.16), 0.03),
        bg_deep: deep_enough(mix(INK.bg_deep, base, 0.07), 0.01),
        surface: deep_enough(mix(INK.surface, base, 0.2), 0.05),
        accent,
        glow_a: darken(vivid(base, 0.), 0.32),
        glow_b: darken(vivid(tamed(accent), -36.), 0.48),
    }
}

/// `c` with a yellow hue turned toward amber: yellows darken to olive, which
/// reads as mud on a dark room; amber stays warm.
pub(crate) fn tamed(c: u32) -> u32 {
    let (h, s, v) = hsv(c);
    if (40. ..76.).contains(&h) {
        from_hsv(30. + (h - 40.) * 0.15, s, v)
    } else {
        c
    }
}

impl Colors {
    fn toward(self, to: Colors, t: f32) -> Colors {
        Colors {
            bg: mix(self.bg, to.bg, t),
            bg_deep: mix(self.bg_deep, to.bg_deep, t),
            surface: mix(self.surface, to.surface, t),
            accent: mix(self.accent, to.accent, t),
            glow_a: mix(self.glow_a, to.glow_a, t),
            glow_b: mix(self.glow_b, to.glow_b, t),
        }
    }

    /// The backdrop's colour at height `y` (its gradient alone).
    pub(crate) fn bg_at(&self, y: f32) -> u32 {
        mix(self.bg, self.bg_deep, (y / 360.).clamp(0., 1.))
    }

    /// The backdrop's colour at (x, y): the gradient and both glows, as the
    /// glow mask falls off (a smoothstep from the centre), for hiding a
    /// cover's corners exactly where they sit.
    pub(crate) fn bg_xy(&self, x: f32, y: f32) -> u32 {
        let glow = |cx: f32, cy: f32, rx: f32, ry: f32, a: f32| {
            let r = (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt();
            let t = (1. - r).max(0.);
            a * t * t * (3. - 2. * t)
        };
        let base = self.bg_at(y);
        let with_a = mix(base, self.glow_a, glow(64., 24., 360., 260., 166. / 255.));
        mix(
            with_a,
            self.glow_b,
            glow(448., 320., 320., 240., 140. / 255.),
        )
    }
}

/// The colours on screen, easing toward each new album's.
#[derive(Default)]
pub(crate) struct Anim {
    from: Cell<Option<Colors>>,
    to: Cell<Option<Colors>>,
    since: Cell<Option<Instant>>,
}

impl Anim {
    pub(crate) fn at(&self, target: Colors, now: Option<Instant>, clock: &Clock) -> Colors {
        let shown = |now: Instant| match (self.from.get(), self.to.get(), self.since.get()) {
            (Some(from), Some(to), Some(since)) => {
                let t =
                    now.saturating_duration_since(since).as_secs_f32() * 1000. / CHANGE_MS as f32;
                from.toward(to, ease(t))
            }
            (_, Some(to), _) => to,
            _ => target,
        };
        match now {
            None => {
                self.from.set(None);
                self.to.set(Some(target));
                self.since.set(None);
                target
            }
            Some(now) => {
                if self.to.get() != Some(target) {
                    let start = self.to.get().map(|_| shown(now));
                    self.from.set(start);
                    self.to.set(Some(target));
                    self.since.set(start.map(|_| now));
                }
                if let Some(since) = self.since.get() {
                    clock.hold(since, CHANGE_MS);
                }
                shown(now)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn album_colours_stay_legible() {
        for accent in [0xFF965AFF, 0x5AE6CDFF, 0xF2E14CFF, 0xBECDE1FF, 0] {
            let c = of_cover(accent);
            assert!(contrast(FG, c.bg) >= 7., "{c:08X?}");
            assert!(contrast(MUTED, c.bg) >= 4.5, "{c:08X?}");
            assert!(contrast(FG, c.surface) >= 7., "{c:08X?}");
        }
        let (r, _, b) = rgb(of_cover(0xFF965AFF).bg);
        assert!(r > b, "a warm cover gives a warm room");
        let (r, _, b) = rgb(of_cover(0x5AE6CDFF).bg);
        assert!(b > r, "a cool cover a cool one");
    }

    #[test]
    fn vivid_colours_glow_but_keep_their_hue() {
        // A muddy brown becomes a brighter, more saturated orange.
        let c = vivid(0x5A4A3AFF, 0.);
        let (r, g, b) = rgb(c);
        assert!(r > g && g > b && r >= 0.6, "{c:08X}");
        // A third of a turn takes red to green.
        let (r, g, _) = rgb(vivid(0xE03030FF, 120.));
        assert!(g > r);
    }

    #[test]
    fn colours_ease_to_the_next_album() {
        let clock = Clock::default();
        let anim = Anim::default();
        let warm = of_cover(0xFF965AFF);
        let cool = of_cover(0x5AE6CDFF);
        let now = Instant::now();
        assert_eq!(
            anim.at(warm, Some(now), &clock),
            warm,
            "the first is at rest"
        );
        let mid = anim.at(cool, Some(now + Duration::from_millis(5)), &clock);
        assert_ne!(mid, cool);
        assert!(clock.moving());
        let later = now + Duration::from_millis(5 + u64::from(CHANGE_MS) + 10);
        assert_eq!(anim.at(cool, Some(later), &clock), cool);
        assert_eq!(anim.at(warm, None, &clock), warm, "previews snap");
    }
}

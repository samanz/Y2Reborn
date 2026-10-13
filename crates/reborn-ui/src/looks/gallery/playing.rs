//! Now Playing: the cover on the left over an accent glow, the song on the
//! right in the title face, then the seek bar, the transport and the volume
//! in a glass pill, all over the blurred cover.
use super::chrome;
use super::kit::{self, display, glass, kicker, on_accent, top_right, top_text, SOFT, TIGHT};
use super::palette::{Colors, DANGER, DIM, FG, MUTED};
use crate::{
    canvas::{Canvas, Icon},
    components,
    look::Frame,
    text::{self, Face},
    theme::{alpha, HEIGHT, WIDTH},
};
use reborn_core::{AudioOutput, PlaybackState, RepeatMode};
use reborn_graphics::{Quad, Tex};

/// The seek and volume rails: text at 18 %.
const RAIL: u32 = 0xF2F3F72E;

/// The cover playing at (x, y), or a quiet tile.
fn art(c: &mut Canvas, f: &Frame, x: f32, y: f32, size: f32) {
    if f.has_art {
        c.draw.push(Quad {
            tex: Tex::Art,
            ..Quad::rect(x, y, size, size, 0xFFFFFFFF)
        });
    } else {
        c.rrect(x, y, size, size, 16., 0x2A2D3AFF);
        c.icon(
            Icon::Vinyl,
            x + size / 2.,
            y + size / 2.,
            (size * 0.4).round(),
            DIM,
        );
    }
}

pub(crate) fn draw(c: &mut Canvas, f: &Frame, colors: &Colors) {
    let Some(t) = f.m.current() else {
        return;
    };
    let m = f.m;
    let accent = colors.accent;
    if f.has_art {
        c.draw.push(Quad {
            tex: Tex::Backdrop,
            ..Quad::rect(0., 0., WIDTH, HEIGHT, 0xFFFFFFFF)
        });
        c.draw.push(
            Quad::rect(0., 0., WIDTH, HEIGHT, alpha(colors.bg_deep, 0x80))
                .horizontal(alpha(colors.bg_deep, 0xEE)),
        );
    }
    chrome::bar(c, f.ui, m, colors);
    // The cover with its glow and shadow.
    let (ax, ay, a) = (20., 40., 172.);
    c.glow(
        ax + a / 2.,
        ay + a / 2. + 6.,
        a * 0.82,
        a * 0.78,
        alpha(accent, 0x66),
    );
    components::cover_shadow(c, ax, ay, a);
    art(c, f, ax, ay, a);
    // The right column.
    let (x, w) = (214., 246.);
    kicker(c, x, 42., w, "Playing from", MUTED);
    top_text(
        c,
        x,
        56.,
        w,
        crate::display_or_unknown(&t.album),
        Face::DM13,
        FG,
    );
    let title = crate::track_title(t);
    // The largest size that sets every word on a line, in two lines; a long
    // title takes three lines of the smallest.
    let fits = |face: Face, lines: usize| {
        title
            .split_whitespace()
            .all(|word| text::width_tracked(face, word, TIGHT) <= w)
            && text::wrap(face, title, w, lines + 1).len() <= lines
    };
    let (face, lines) = [Face::SY34, Face::SY26, Face::SY23, Face::SY20]
        .into_iter()
        .filter(|face| *face != Face::SY34 || title.is_ascii())
        .find(|face| fits(*face, 2))
        .map_or((Face::SY20, 3), |face| (face, 2));
    let lead = (face.px() * 1.12).round();
    let mut y = 80.;
    for line in text::wrap(face, title, w, lines) {
        display(c, x, y, w, &line, face, FG);
        y += lead;
    }
    y += 6.;
    top_text(
        c,
        x,
        y,
        w,
        crate::display_or_unknown(&t.artist),
        Face::DM16,
        SOFT,
    );
    let offline =
        !m.sources.is_empty() && !m.sources.iter().any(|s| s.id == t.source_id && s.online);
    let note = if offline {
        Some(("SD card removed", "Reinsert the card to keep listening"))
    } else {
        match m.playback {
            PlaybackState::Error => Some(("No audio output", "Check headphones, then press Play")),
            PlaybackState::Buffering => Some(("Starting…", "")),
            PlaybackState::Paused => Some(("Paused", "")),
            PlaybackState::Stopped => Some(("Stopped", "")),
            PlaybackState::Playing => None,
        }
    };
    if let Some((state, help)) = note {
        let tint = if offline || m.playback == PlaybackState::Error {
            DANGER
        } else {
            MUTED
        };
        let top = (y + 26.).min(196.);
        kicker(c, x, top, w, state, tint);
        if !help.is_empty() {
            top_text(c, x, top + 14., w, help, Face::DM12, MUTED);
        }
    }
    // The seek bar across the width, with mono times.
    let value = components::progress(m.position_ms, t.duration_ms);
    let (sx, sy, sw) = (24., 232., 432.);
    c.rrect(sx, sy - 1.5, sw, 3., 1.5, RAIL);
    let at_x = (sx + sw * value).round();
    kit::rrect_horizontal(
        c,
        sx,
        sy - 1.5,
        (sw * value).round().max(3.),
        3.,
        1.5,
        accent,
        FG,
    );
    c.glow(at_x, sy, 14., 12., alpha(accent, 0x90));
    c.disc(at_x, sy, 5., FG);
    top_text(
        c,
        sx,
        sy + 10.,
        120.,
        &components::time(m.position_ms),
        Face::PM11,
        MUTED,
    );
    let remaining = format!(
        "-{}",
        components::time(t.duration_ms.saturating_sub(m.position_ms))
    );
    top_right(c, sx + sw, sy + 10., &remaining, Face::PM11, MUTED);
    // The transport: the accent play button with its glow, previous and next.
    let (cx, cy) = (240., 278.);
    c.glow(cx, cy + 4., 56., 44., alpha(accent, 0x78));
    c.disc(cx, cy, 25., accent);
    let playing = m.playback == PlaybackState::Playing;
    c.icon(
        if playing { Icon::Pause } else { Icon::Play },
        cx + if playing { 0. } else { 1. },
        cy,
        22.,
        on_accent(accent),
    );
    c.icon(Icon::Prev, cx - 66., cy, 24., FG);
    c.icon(Icon::Next, cx + 66., cy, 24., FG);
    c.icon(
        Icon::Shuffle,
        40.,
        cy,
        20.,
        if m.settings.shuffle { accent } else { DIM },
    );
    c.icon(
        if m.settings.repeat == RepeatMode::Track {
            Icon::RepeatOne
        } else {
            Icon::Repeat
        },
        440.,
        cy,
        20.,
        if m.settings.repeat == RepeatMode::Off {
            DIM
        } else {
            accent
        },
    );
    // The volume, the wheel's target here, in a glass pill.
    glass(c, 14., 316., 452., 36., 18.);
    let output = if m.output == AudioOutput::Wired {
        Icon::Headphones
    } else {
        Icon::Bluetooth
    };
    c.icon(output, 37., 334., 18., MUTED);
    let (vx, vy, vw) = (58., 334., 352.);
    c.rrect(vx, vy - 1.5, vw, 3., 1.5, RAIL);
    let level = f32::from(m.settings.volume.min(100)) / 100.;
    let at = c.draw.len();
    c.rrect(vx, vy - 1.5, (vw * level).round().max(3.), 3., 1.5, accent);
    if f.interactive {
        c.mark_focus(at);
    }
    top_right(
        c,
        452.,
        327.,
        &m.settings.volume.to_string(),
        Face::PM11,
        FG,
    );
}

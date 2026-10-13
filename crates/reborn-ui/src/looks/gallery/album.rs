//! An album's page: the room washed in the album's colour, the cover large
//! with its shadow, the name in the title face over the artist, mono facts
//! in chips, a filled Play pill and an outlined Shuffle pill, then the
//! songs as rows (mono number, title, mono time; the one playing in the
//! accent with level bars).
//!
//! Play and Shuffle are the album's first two rows, so the standard focus
//! walks them.
use super::kit::{self, display, display_face, kicker, top_right, top_text, PillStyle, CLEAR};
use super::palette::{self, Colors, FG, MUTED};
use super::{now, GUTTER};
use crate::{
    canvas::{Canvas, Icon},
    components,
    look::Frame,
    screens::{is_current, row_track},
    text::{self, Face},
    theme::{alpha, WIDTH},
};
use reborn_core::Track;

const COVER: f32 = 120.;
const TOP: f32 = 40.;
/// Where the pills and the songs start on the page.
const PILLS_Y: f32 = TOP + COVER + 16.;
const SONGS_Y: f32 = PILLS_Y + kit::PILL_H + 16.;
const SONG_H: f32 = 44.;

/// "34 min" or "1 h 12 min".
fn length(ms: u64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    if minutes >= 60 {
        format!("{} h {} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

/// The colour the room is washed in: the accent while the album plays,
/// else the calm tint its cover stands in with, made vivid.
fn wash_color(f: &Frame, colors: &Colors, t: &Track) -> u32 {
    let playing =
        f.m.current()
            .is_some_and(|c| c.album == t.album && c.album_artist == t.album_artist);
    if playing {
        colors.accent
    } else {
        palette::vivid(components::placeholder_color(components::art_key(t)), 0.)
    }
}

pub(crate) fn draw(c: &mut Canvas, f: &Frame, colors: &Colors) {
    let (ui, m, tracks) = (f.ui, f.m, f.tracks);
    let st = &ui.gallery;
    let rows = ui.rows(m, tracks);
    let songs: Vec<(usize, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(n, r)| row_track(r, tracks).map(|(i, _)| (n, i)))
        .collect();
    // Focus: a pill (0 Play, 1 Shuffle) or a song (its place in `songs`).
    let focus = m.navigation.focus.min(rows.len().saturating_sub(1));
    let (pill_focus, song_focus) = match rows.get(focus).map(|r| r.key.as_str()) {
        Some("play:collection") => (Some(0), None),
        Some("shuffle:collection") => (Some(1), None),
        _ => (None, songs.iter().position(|(n, _)| *n == focus)),
    };
    let fresh = st.fresh_page(m);
    let arrive = st.focus_in.progress(
        pill_focus.unwrap_or(100 + song_focus.unwrap_or(0)),
        now(ui),
        200,
        &st.clock,
    );
    let bottom = kit::content_bottom(f);
    let (top, bottom_at) = match song_focus {
        None => (0., PILLS_Y + kit::PILL_H),
        Some(s) => {
            let y = SONGS_Y + s as f32 * SONG_H;
            (y, y + SONG_H)
        }
    };
    let from = if fresh {
        0.
    } else {
        st.sheet.target().unwrap_or(0.)
    };
    let to = kit::aim(from, top, bottom_at, 44., bottom);
    let mut starts = vec![0., PILLS_Y - 44.];
    starts.extend((0..songs.len()).map(|n| SONGS_Y + n as f32 * SONG_H - 44.));
    let to = kit::snap(to, &starts, bottom_at, bottom);
    let scroll = kit::glide(&st.sheet, f, to);

    // What the page is about: the album's first song with artwork.
    let head = ui
        .collection_track(m, tracks)
        .and_then(|t| tracks.iter().position(|x| std::ptr::eq(x, t)));
    let art = songs
        .iter()
        .map(|(_, i)| *i)
        .find(|&i| tracks[i].artwork)
        .or(head);
    let t = head.and_then(|i| tracks.get(i));
    let title = t.map_or("Album", |t| crate::display_or_unknown(&t.album));
    let artist = t.map_or("", |t| {
        crate::display_or_unknown(if t.album_artist.is_empty() {
            &t.artist
        } else {
            &t.album_artist
        })
    });

    // The room washed in the album's colour.
    let wash: Option<kit::Wash> = t.map(|t| {
        (
            110.,
            90. - scroll * 0.5,
            300.,
            230.,
            alpha(palette::tamed(wash_color(f, colors, t)), 0x58),
        )
    });
    if let Some((x, y, rx, ry, color)) = wash {
        c.glow(x, y, rx, ry, color);
    }

    // The cover and what the page is about.
    let y = TOP - scroll;
    if y + COVER > 48. {
        components::cover_shadow(c, GUTTER, y, COVER);
        let key = art.and_then(|i| tracks.get(i).map(|t| (i, t)));
        components::cover_rounded(c, ui, key, GUTTER, y, COVER, kit::COVER_R);
        kit::round_corners(c, colors, (GUTTER, y, COVER), kit::COVER_R, None, wash);
    }
    let x = GUTTER + COVER + 20.;
    let w = WIDTH - GUTTER - x;
    if y + 4. >= CLEAR {
        kicker(c, x, y + 4., w, "Album", colors.accent);
    }
    let face = display_face(title, w, &[Face::SY26, Face::SY23]);
    let lines = text::wrap(face, title, w, 2);
    let lead = face.px() + 4.;
    let mut ty = y + 20.;
    for line in &lines {
        if ty >= CLEAR {
            display(c, x, ty, w, line, face, FG);
        }
        ty += lead;
    }
    if !artist.is_empty() && ty + 2. >= CLEAR {
        top_text(c, x, ty + 2., w, artist, Face::DM15, MUTED);
    }
    // Facts in mono chips along the cover's foot.
    let count = songs.len();
    let total: u64 = songs.iter().map(|(_, i)| tracks[*i].duration_ms).sum();
    let mut facts = vec![kit::plural(count, "song", "songs"), length(total)];
    if let Some(t) = art.and_then(|i| tracks.get(i)) {
        if !t.codec.is_empty() {
            let khz = t.sample_rate / 1000;
            facts.push(if khz > 0 {
                format!("{} {khz}k", t.codec)
            } else {
                t.codec.clone()
            });
        }
    }
    let cy = y + COVER - 20.;
    if cy >= CLEAR {
        let mut cx = x;
        for fact in &facts {
            if cx + kit::chip_width(fact) > WIDTH - GUTTER {
                break;
            }
            cx += kit::chip(c, cx, cy, fact) + 6.;
        }
    }

    // Play and Shuffle.
    let py = PILLS_Y - scroll;
    if (py + kit::PILL_H > 0. || pill_focus.is_some()) && !songs.is_empty() {
        let lit = f.interactive;
        let w = kit::pill(
            c,
            colors,
            GUTTER,
            py,
            "Play",
            Some(Icon::Play),
            PillStyle::Filled,
            lit && pill_focus == Some(0),
            arrive,
        );
        kit::pill(
            c,
            colors,
            GUTTER + w + 12.,
            py,
            "Shuffle",
            Some(Icon::Shuffle),
            PillStyle::Outlined,
            lit && pill_focus == Some(1),
            arrive,
        );
    }

    // The songs.
    for (n, (_, i)) in songs.iter().enumerate() {
        let ry = SONGS_Y + n as f32 * SONG_H - scroll;
        if (ry + SONG_H < CLEAR || ry > bottom) && song_focus != Some(n) {
            continue;
        }
        let t = &tracks[*i];
        let focused = f.interactive && song_focus == Some(n);
        let number = if t.track > 0 { t.track as usize } else { n + 1 };
        song_row(c, f, colors, t, number, ry, focused, arrive);
    }
    if songs.is_empty() {
        top_text(
            c,
            GUTTER,
            PILLS_Y - scroll,
            WIDTH - 2. * GUTTER,
            "No songs here on the player",
            Face::DM14,
            MUTED,
        );
    }
    if m.current().is_some() {
        kit::bottom_veil(c, colors, bottom + 8.);
        kit::mini_player(c, f, colors, kit::MINI_LOW);
    }
    kit::status(c, f, colors);
}

/// One song: its number (or level bars while it plays), the title (and an
/// artist that differs from the album's) and its length.
#[allow(clippy::too_many_arguments)]
fn song_row(
    c: &mut Canvas,
    f: &Frame,
    colors: &Colors,
    t: &Track,
    number: usize,
    y: f32,
    focused: bool,
    arrive: f32,
) {
    let (x, w) = (GUTTER - 4., WIDTH - 2. * GUTTER + 8.);
    if focused {
        kit::row_focus(c, colors, x, y + 2., w, SONG_H - 4., arrive);
    }
    let playing = is_current(f.m, t);
    let mid = y + SONG_H / 2.;
    if playing {
        kit::eq_bars(c, GUTTER + 6., mid + 6., colors.accent);
    } else {
        top_right(
            c,
            GUTTER + 22.,
            mid - 7.,
            &number.to_string(),
            Face::PM12,
            MUTED,
        );
    }
    let tx = GUTTER + 40.;
    let time = components::time(t.duration_ms);
    let tw = WIDTH - GUTTER - 12. - text::width(Face::PM11, &time) - 16. - tx;
    let ink = if playing { colors.accent } else { FG };
    if !t.album_artist.is_empty() && t.artist != t.album_artist {
        top_text(c, tx, mid - 17., tw, crate::track_title(t), Face::DM15, ink);
        top_text(
            c,
            tx,
            mid + 2.,
            tw,
            crate::display_or_unknown(&t.artist),
            Face::DM12,
            MUTED,
        );
    } else {
        top_text(c, tx, mid - 9., tw, crate::track_title(t), Face::DM15, ink);
    }
    top_right(c, WIDTH - GUTTER - 8., mid - 7., &time, Face::PM11, MUTED);
}

//! Every list screen Gallery does not lay out on its own (artists, songs,
//! folders, the queue, Settings and its pages): the screen's name in the
//! title face with its position in a glass pill, an optional message and
//! facts, rows with covers, mono times and pill switches under the glowing
//! focus, a thin scroll pill and the mini player at the foot.
use super::chrome::{self, panel_focus};
use super::kit::{
    self, display, display_face, eq_bars, glass, kicker, top_right, top_text, MINI_LOW, SOFT,
};
use super::palette::{Colors, DIM, FG, MUTED};
use crate::{
    canvas::{Canvas, Icon},
    components,
    look::Frame,
    pages::{self, Page},
    screens::{is_current, row_track, screen_title},
    text::{self, Face},
    theme::{alpha, mix},
    views_library::empty_copy,
    Item, RowKind,
};
use reborn_core::{AppModel, MediaSource, Screen};

/// Where the list starts, its pitch and a row's height.
const TOP: f32 = 76.;
const PITCH: f32 = 48.;
const ROW_H: f32 = 46.;
/// Where the rows start under an album or artist header.
const COLLECTION_TOP: f32 = 170.;
/// The list ends above the mini player.
const BOTTOM: f32 = MINI_LOW - 8.;

fn catalog_like(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Albums
            | Screen::Artists
            | Screen::Genres
            | Screen::Tracks
            | Screen::Folders
            | Screen::Album
            | Screen::Artist
            | Screen::Queue
            | Screen::Diagnostics
            | Screen::DiagnosticSection
    )
}

fn shows_counter(screen: Screen) -> bool {
    catalog_like(screen) && screen != Screen::Diagnostics
}

/// One row as the list draws it.
struct RowDraw<'a> {
    item: &'a Item,
    y: f32,
    focused: bool,
    /// Right-hand text: a duration.
    trailing: String,
    playing: bool,
    /// The cover standing for the row, else an icon in its place.
    art: Option<usize>,
    icon: Option<Icon>,
}

/// What a Folders row is: the SD card, a folder or a song.
fn folder_icon(m: &AppModel, item: &Item) -> Option<Icon> {
    if m.screen != Screen::Folders {
        return None;
    }
    if item.key.starts_with("track:") {
        return Some(Icon::Note);
    }
    let card = m.sources.iter().any(|s| {
        matches!(s.kind, MediaSource::SdCard(_)) && item.key.ends_with(&*s.root.to_string_lossy())
    });
    Some(if card { Icon::Sd } else { Icon::Folder })
}

/// The screen's name and a position such as "3 / 24" in a glass pill.
fn title(c: &mut Canvas, name: &str, detail: &str) {
    let detail = text::fit(Face::PM11, detail, 130.);
    let pill = if detail.is_empty() {
        0.
    } else {
        text::width(Face::PM11, &detail) + 20.
    };
    let face = display_face(name, 440. - pill, &[Face::SY26, Face::SY23, Face::SY20]);
    display(
        c,
        18.,
        34. + (26. - face.px()) / 2.,
        440. - pill,
        name,
        face,
        FG,
    );
    if pill > 0. {
        glass(c, 462. - pill, 40., pill, 22., 11.);
        top_right(c, 452., 44., &detail, Face::PM11, MUTED);
    }
}

/// A row: the cover, two lines, mono times, pill switches.
fn row(c: &mut Canvas, f: &Frame, colors: &Colors, r: &RowDraw) {
    let accent = colors.accent;
    let y = r.y;
    if r.focused && r.item.enabled {
        panel_focus(c, 16., y, 448., ROW_H, 14., accent);
    }
    let mut x = 30.;
    let action = match r.item.key.as_str() {
        "play:collection" => Some(Icon::Play),
        "shuffle:collection" => Some(Icon::Shuffle),
        _ => None,
    };
    if let Some(icon) = action {
        // Play and Shuffle as round accent buttons in the cover column.
        c.disc(42., y + ROW_H / 2., 18., alpha(accent, 0x30));
        c.icon(icon, 42., y + ROW_H / 2., 16., mix(accent, 0xFFFFFFFF, 0.2));
        x = 72.;
    } else if let Some(i) = r.art {
        let behind = if r.focused && r.item.enabled {
            Some(mix(0x1E1F24FF, accent, 0.16))
        } else {
            None
        };
        kit::cover(c, f, colors, Some(i), 24., y + 5., 36., 10., behind);
        x = 72.;
    } else if let Some(icon) = r.icon {
        c.rrect(
            24.,
            y + 5.,
            36.,
            36.,
            10.,
            mix(colors.surface, accent, 0.18),
        );
        c.icon(icon, 42., y + ROW_H / 2., 18., mix(accent, FG, 0.55));
        x = 72.;
    }
    let check = matches!(r.item.kind, RowKind::Check(true));
    let toggle = match r.item.kind {
        RowKind::Toggle(on) => Some(on),
        _ => None,
    };
    let chevron = r.item.kind == RowKind::Chevron;
    let mut tail_w = if toggle.is_some() {
        46.
    } else if !r.trailing.is_empty() {
        text::width(Face::PM11, &r.trailing) + 20.
    } else if check || chevron {
        28.
    } else {
        0.
    };
    if r.playing {
        tail_w += 18.;
    }
    let two = !r.item.secondary.is_empty();
    let ink = if r.item.enabled { FG } else { DIM };
    top_text(
        c,
        x,
        y + if two { 6. } else { 14. },
        438. - x - tail_w,
        &r.item.label,
        Face::DM15,
        if r.playing { accent } else { ink },
    );
    if two {
        top_text(
            c,
            x,
            y + 27.,
            438. - x - tail_w,
            &r.item.secondary,
            Face::DM12,
            MUTED,
        );
    }
    let mid = y + ROW_H / 2.;
    let mut right = 450.;
    if let Some(on) = toggle {
        kit::switch(c, right, mid, on, accent);
        right -= 46.;
    } else if !r.trailing.is_empty() {
        top_right(c, right, mid - 7., &r.trailing, Face::PM11, MUTED);
        right -= text::width(Face::PM11, &r.trailing) + 10.;
    } else if check {
        c.icon(Icon::Check, 440., mid, 18., accent);
        right -= 28.;
    } else if chevron {
        c.icon(Icon::Chevron, 442., mid, 14., DIM);
        right -= 28.;
    }
    if r.playing {
        eq_bars(c, right - 10., mid + 6., accent);
    }
}

/// A thin pill for the scroll position, no track.
fn scroll_pill(c: &mut Canvas, top: f32, height: f32, scroll: usize, visible: usize, count: usize) {
    if count <= visible {
        return;
    }
    let thumb = (height * visible as f32 / count as f32).max(18.);
    let y = top + (height - thumb) * scroll as f32 / (count - visible).max(1) as f32;
    c.rrect(469., y.round(), 3., thumb.round(), 1.5, 0xF2F3F755);
}

/// The album or artist above its rows: the cover over its glow, a mono
/// label, the name and the artist.
fn collection_header(c: &mut Canvas, f: &Frame, colors: &Colors, count: usize) {
    // An artist's rows are albums, not songs: take the first one's cover.
    let rows = f.ui.rows(f.m, f.tracks);
    let head =
        f.ui.collection_track(f.m, f.tracks)
            .and_then(|t| f.tracks.iter().position(|x| std::ptr::eq(x, t)))
            .or_else(|| rows.iter().find_map(|r| r.art));
    let (ax, ay, a) = (18., 76., 86.);
    c.glow(
        ax + a / 2.,
        ay + a / 2.,
        a * 0.85,
        a * 0.7,
        alpha(colors.accent, 0x55),
    );
    components::cover_shadow(c, ax, ay, a);
    kit::cover(c, f, colors, head, ax, ay, a, kit::COVER_R, None);
    let Some(t) = head.and_then(|i| f.tracks.get(i)) else {
        return;
    };
    let items = count.saturating_sub(1);
    let (kind, name, sub) = if f.m.screen == Screen::Artist {
        (
            format!("Artist · {}", kit::plural(items, "album", "albums")),
            crate::display_or_unknown(&t.artist).to_owned(),
            String::new(),
        )
    } else {
        (
            format!("Album · {}", kit::plural(items, "song", "songs")),
            crate::display_or_unknown(&t.album).to_owned(),
            crate::display_or_unknown(if t.album_artist.is_empty() {
                &t.artist
            } else {
                &t.album_artist
            })
            .to_owned(),
        )
    };
    let (x, w) = (122., 340.);
    kicker(c, x, 80., w, &kind, MUTED);
    let face = if text::wrap(Face::SY23, &name, w, 3).len() <= 2 {
        Face::SY23
    } else {
        Face::SY20
    };
    let mut y = 96.;
    for line in text::wrap(face, &name, w, 2) {
        display(c, x, y, w, &line, face, FG);
        y += 27.;
    }
    if !sub.is_empty() {
        top_text(c, x, y + 3., w, &sub, Face::DM14, SOFT);
    }
}

/// A message: the title over the body; returns the y below it.
fn hero(c: &mut Canvas, title: &str, body: &str, y: f32) -> f32 {
    let mut y = y;
    for line in text::wrap(Face::SY23, title, 432., 2) {
        display(c, 24., y, 432., &line, Face::SY23, FG);
        y += 29.;
    }
    if !body.is_empty() {
        y += 6.;
        for line in text::wrap(Face::DM14, body, 432., 3) {
            top_text(c, 24., y, 432., &line, Face::DM14, MUTED);
            y += 21.;
        }
    }
    y
}

/// Two-column facts; returns the y below them.
fn facts(c: &mut Canvas, facts: &[(String, String)], top: f32) -> f32 {
    let mut y = top;
    for (label, value) in facts {
        top_text(c, 28., y + 7., 150., label, Face::DM14, MUTED);
        let value = text::fit(Face::DM14, value, 270.);
        top_right(c, 452., y + 7., &value, Face::DM14, FG);
        y += 34.;
        if y > BOTTOM - 34. {
            break;
        }
    }
    y
}

/// The list screens; `footer` draws the mini player at the foot (a tab's
/// root has it in the tab pill's shell instead).
pub(crate) fn draw(c: &mut Canvas, f: &Frame, colors: &Colors, footer: bool) {
    let (ui, m, tracks) = (f.ui, f.m, f.tracks);
    let page = if catalog_like(m.screen) {
        Page::default()
    } else if m.screen == Screen::ValueDetail {
        let (title, value) = m
            .navigation
            .filter
            .split_once('\u{1f}')
            .unwrap_or(("Detail", &m.navigation.filter));
        Page {
            hero: Some(pages::Hero {
                title: title.into(),
                body: value.into(),
            }),
            rows: ui.rows(m, tracks),
            ..Default::default()
        }
    } else {
        pages::page(ui, m, tracks)
    };
    let count = ui.row_count(m, tracks);
    let focus = m.navigation.focus.min(count.saturating_sub(1));
    let detail = if count > 0 && shows_counter(m.screen) {
        format!("{} / {}", focus + 1, count)
    } else {
        String::new()
    };
    chrome::bar(c, ui, m, colors);
    title(c, &screen_title(m), &detail);
    let mut top = TOP;
    let mut visible = ((BOTTOM - top) / PITCH).floor().max(1.) as usize;
    if matches!(m.screen, Screen::Album | Screen::Artist) && count > 0 {
        collection_header(c, f, colors, count);
        top = COLLECTION_TOP;
        visible = ((BOTTOM - top) / PITCH).floor().max(1.) as usize;
    }
    if count == 0 && catalog_like(m.screen) {
        let (title, body, action) = empty_copy(m);
        let y = hero(c, title, body, top + 36.);
        if let Some(action) = action {
            let item = Item::new(action, "empty");
            row(
                c,
                f,
                colors,
                &RowDraw {
                    item: &item,
                    y: y + 14.,
                    focused: f.interactive,
                    trailing: String::new(),
                    playing: false,
                    art: None,
                    icon: None,
                },
            );
        }
    } else {
        if let Some(h) = &page.hero {
            let y = if page.rows.is_empty() && page.facts.is_empty() {
                top + 52.
            } else {
                top + 20.
            };
            top = hero(c, &h.title, &h.body, y) + 14.;
        }
        if !page.facts.is_empty() {
            top = facts(c, &page.facts, top + 4.) + 10.;
        }
        if page.hero.is_some() || !page.facts.is_empty() {
            visible = (((BOTTOM - top) / PITCH).floor() as usize).max(1);
        }
        let scroll = m
            .navigation
            .scroll
            .min(focus)
            .max(focus.saturating_add(1).saturating_sub(visible));
        for (i, item) in ui
            .visible_rows(m, tracks, scroll, visible)
            .iter()
            .enumerate()
        {
            let index = scroll + i;
            let song = row_track(item, tracks).map(|(_, t)| t);
            let queued = if m.screen == Screen::Queue {
                m.queue.get(index)
            } else {
                None
            };
            let playing = if m.screen == Screen::Queue {
                index == m.queue_position
            } else {
                song.is_some_and(|t| is_current(m, t))
            };
            let trailing = song
                .or(queued)
                .map(|t| components::time(t.duration_ms))
                .unwrap_or_default();
            row(
                c,
                f,
                colors,
                &RowDraw {
                    item,
                    y: top + i as f32 * PITCH,
                    focused: f.interactive && index == focus,
                    trailing,
                    playing,
                    // A queued song's cover is the library's copy of it.
                    art: item.art.or_else(|| {
                        queued.and_then(|q| {
                            tracks
                                .iter()
                                .position(|t| t.id == q.id && t.source_id == q.source_id)
                        })
                    }),
                    icon: folder_icon(m, item),
                },
            );
        }
        scroll_pill(c, top, visible as f32 * PITCH, scroll, visible, count);
    }
    match &page.status {
        Some(status) => chrome::note(c, MINI_LOW, status),
        None if footer && m.current().is_some() => {
            kit::bottom_veil(c, colors, MINI_LOW);
            kit::mini_player(c, f, colors, MINI_LOW);
        }
        None => {}
    }
}

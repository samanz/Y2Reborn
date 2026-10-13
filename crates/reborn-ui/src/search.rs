//! Search: the typed query and a character ring above live results.
//!
//! While typing, the wheel turns the ring under the field (letters, digits,
//! Space, Delete, Done), Select adds the focused stop and Back deletes; the
//! results below follow every character. Done, or holding Select, moves the
//! focus down into the results; Back, or turning back past the first result,
//! returns to typing. Results come grouped (artists, albums, songs), each cut
//! to a few rows with "Show all" for the rest.
//!
//! Select on a song plays its album from that song (a result list is a mixed
//! bag; the album is the listening the song belongs to). Artists and albums
//! open their pages. The index is built once per library, when Search is
//! used, never on the way through other screens.
use crate::{
    canvas::{Canvas, Icon},
    components,
    text::{self, Face},
    theme::{color, layout, mix, HEIGHT, WIDTH},
    Item, Ui,
};
use reborn_core::{
    search::{Index, Kind, Searcher},
    Action, AppModel, Effect, Screen, Track,
};
use std::cell::RefCell;

/// The ring's characters; Space, Delete and Done follow them.
const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
/// Longest query.
const QUERY_MAX: usize = 40;
/// Rows an opened ("Show all") group shows at most.
const SHOWN_ALL: usize = 200;

/// Rows each group shows before "Show all".
fn shown(kind: Kind) -> usize {
    match kind {
        Kind::Songs => 5,
        _ => 3,
    }
}

/// One stop on the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Char(u8),
    Space,
    Delete,
    Done,
}

impl Key {
    const COUNT: usize = CHARS.len() + 3;

    fn at(stop: usize) -> Self {
        match stop.checked_sub(CHARS.len()) {
            None => Key::Char(CHARS[stop]),
            Some(0) => Key::Space,
            Some(1) => Key::Delete,
            Some(_) => Key::Done,
        }
    }

    /// What the stop says; the focused one has room for a whole word.
    fn label(self, focused: bool) -> String {
        match (self, focused) {
            (Key::Char(c), _) => (c as char).to_string(),
            (Key::Space, _) => "Space".into(),
            (Key::Delete, true) => "Delete".into(),
            (Key::Delete, false) => "Del".into(),
            (Key::Done, true) => "Results".into(),
            (Key::Done, false) => "Done".into(),
        }
    }
}

/// The search screen's state. The results are cached until the query or the
/// library changes.
#[derive(Default)]
pub struct SearchView {
    pub query: String,
    /// The focused ring stop.
    stop: usize,
    /// The wheel moves through the results instead of typing.
    pub(crate) results: bool,
    /// The group opened with "Show all".
    expanded: Option<Kind>,
    cache: RefCell<Cache>,
}

#[derive(Default)]
struct Cache {
    /// The library the index was built from: its address and length.
    library: Option<(usize, usize)>,
    index: Index,
    searcher: Searcher,
    /// Bumped whenever the index is rebuilt.
    generation: u64,
    /// What `rows` were made for: query, opened group, index generation.
    rows_for: Option<(String, Option<Kind>, u64)>,
    rows: Vec<Item>,
}

impl Cache {
    fn ensure(&mut self, tracks: &[Track]) {
        let library = (tracks.as_ptr() as usize, tracks.len());
        if self.library != Some(library) {
            self.index = Index::build(tracks);
            self.searcher.reset();
            self.library = Some(library);
            self.generation += 1;
        }
    }
}

impl SearchView {
    /// The focused ring stop.
    pub(crate) fn key(&self) -> Key {
        Key::at(self.stop.min(Key::COUNT - 1))
    }

    fn turn(&mut self, delta: i32) {
        self.stop = (self.stop as i32 + delta).rem_euclid(Key::COUNT as i32) as usize;
    }

    fn type_char(&mut self, c: char) {
        if self.query.len() >= QUERY_MAX
            || (c == ' ' && (self.query.is_empty() || self.query.ends_with(' ')))
        {
            return;
        }
        self.query.push(c);
        self.expanded = None;
    }

    fn delete(&mut self) {
        self.query.pop();
        self.expanded = None;
    }

    /// Rebuild the index the next time it is needed (the library changed in
    /// place).
    pub(crate) fn forget_index(&self) {
        self.cache.borrow_mut().library = None;
    }
}

/// The rows of the results, group headings included as rows that cannot be
/// focused.
pub(crate) fn rows(ui: &Ui, tracks: &[Track]) -> Vec<Item> {
    let s = &ui.search;
    let query = s.query.trim();
    if query.is_empty() {
        return vec![];
    }
    let mut cache = s.cache.borrow_mut();
    cache.ensure(tracks);
    let wanted = (query.to_owned(), s.expanded, cache.generation);
    if cache.rows_for.as_ref() != Some(&wanted) {
        let rows = result_rows(&mut cache, tracks, query, s.expanded);
        cache.rows = rows;
        cache.rows_for = Some(wanted);
    }
    cache.rows.clone()
}

fn result_rows(
    cache: &mut Cache,
    tracks: &[Track],
    query: &str,
    expanded: Option<Kind>,
) -> Vec<Item> {
    let Cache {
        index, searcher, ..
    } = cache;
    let groups = searcher.run(index, query, |k| {
        if expanded == Some(k) {
            SHOWN_ALL
        } else {
            shown(k)
        }
    });
    let mut rows = vec![];
    for g in groups {
        let mut heading = Item::new(g.kind.title(), format!("head:{}", g.kind.key()))
            .with_secondary(g.total.to_string());
        heading.enabled = false;
        rows.push(heading);
        for e in g.hits.iter().filter_map(|&at| index.entry(g.kind, at)) {
            let mut item = match g.kind {
                Kind::Songs => {
                    let t = &tracks[e.track];
                    let detail = match (t.artist.is_empty(), t.album.is_empty()) {
                        (false, false) => format!("{} · {}", t.artist, t.album),
                        (false, true) => t.artist.clone(),
                        (true, _) => crate::display_or_unknown(&t.album).to_owned(),
                    };
                    Item::new(crate::track_title(t), format!("track:{}", e.track))
                        .with_secondary(detail)
                }
                Kind::Artists => {
                    Item::new(&e.name, &e.key).with_secondary(Kind::Songs.count(e.count))
                }
                Kind::Albums => {
                    Item::new(&e.name, &e.key).with_secondary(crate::display_or_unknown(&e.detail))
                }
            };
            item.art = Some(e.track);
            rows.push(item);
        }
        if g.hits.len() < g.total && expanded != Some(g.kind) {
            rows.push(Item::new(
                format!("Show all {}", g.kind.count(g.total)),
                format!("more:{}", g.kind.key()),
            ));
        }
    }
    rows
}

impl Ui {
    /// Open Search with an empty query, typing.
    pub(crate) fn open_search(&mut self, m: &mut AppModel) {
        let s = &mut self.search;
        s.query.clear();
        s.stop = 0;
        s.results = false;
        s.expanded = None;
        Self::go(m, Screen::Search, "");
    }

    /// Input on the Search screen before the usual handling; `None` lets it
    /// through (Home, transport, Back with nothing typed, the focus moving
    /// in the results).
    pub(crate) fn search_action(
        &mut self,
        m: &mut AppModel,
        tracks: &[Track],
        action: Action,
    ) -> Option<Effect> {
        if m.screen != Screen::Search || m.navigation.modal.is_some() {
            return None;
        }
        if !self.search.results {
            return self.search_typing(m, tracks, action);
        }
        let rows = self.rows(m, tracks);
        let first = rows.iter().position(|r| r.enabled);
        match action {
            Action::Back => {
                if self.search.expanded.take().is_none() {
                    self.search.results = false;
                }
                Some(Effect::None)
            }
            Action::WheelCounterClockwise(_) | Action::NavigateUp
                if first.is_none_or(|f| m.navigation.focus <= f) =>
            {
                self.search.results = false;
                Some(Effect::None)
            }
            Action::Select => {
                let row = rows.get(m.navigation.focus).filter(|r| r.enabled)?;
                Some(self.search_select(m, tracks, &row.key))
            }
            _ => None,
        }
    }

    fn search_typing(
        &mut self,
        m: &mut AppModel,
        tracks: &[Track],
        action: Action,
    ) -> Option<Effect> {
        let s = &mut self.search;
        // A slow turn is one stop per detent; a fast spin skips ahead.
        let spin = |n: u8| i32::from(n.clamp(1, 4));
        match action {
            Action::WheelClockwise(n) => s.turn(spin(n)),
            Action::WheelCounterClockwise(n) => s.turn(-spin(n)),
            Action::NavigateDown => s.turn(1),
            Action::NavigateUp => s.turn(-1),
            Action::Select => match s.key() {
                Key::Char(c) => s.type_char(c as char),
                Key::Space => s.type_char(' '),
                Key::Delete => s.delete(),
                Key::Done => {
                    self.focus_results(m, tracks);
                    return Some(Effect::None);
                }
            },
            Action::ContextMenu => {
                self.focus_results(m, tracks);
                return Some(Effect::None);
            }
            Action::Back if !s.query.is_empty() => s.delete(),
            _ => return None,
        }
        // The results start again from the top as the query changes.
        m.navigation.focus = 0;
        m.navigation.scroll = 0;
        Some(Effect::None)
    }

    /// Move the focus into the results, on the first that can be chosen.
    fn focus_results(&mut self, m: &mut AppModel, tracks: &[Track]) {
        match self.rows(m, tracks).iter().position(|r| r.enabled) {
            Some(first) => {
                self.search.results = true;
                m.navigation.focus = first;
                m.navigation.scroll = 0;
            }
            None if self.search.query.trim().is_empty() => self.flash("Type a name first"),
            None => self.flash("Nothing found"),
        }
    }

    fn search_select(&mut self, m: &mut AppModel, tracks: &[Track], key: &str) -> Effect {
        if let Some(kind) = key.strip_prefix("more:").and_then(Kind::from_key) {
            self.search.expanded = Some(kind);
            return Effect::None;
        }
        if let Some(i) = key
            .strip_prefix("track:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            let Some(t) = tracks.get(i) else {
                return Effect::None;
            };
            // The song's album in album order, as its page lists it.
            let mut members = vec![i];
            if !t.album.is_empty() {
                let album = crate::album_filter(t);
                members = (0..tracks.len())
                    .filter(|&j| crate::track_matches(&tracks[j], &album))
                    .collect();
                members.sort_by_key(|&j| {
                    let t = &tracks[j];
                    (t.disc, t.track, t.filename.as_str())
                });
            }
            Self::go(m, Screen::NowPlaying, "");
            return Effect::PlayCollection {
                selected: i,
                members,
                shuffle: false,
            };
        }
        let screen = if key.starts_with("artist:") {
            Screen::Artist
        } else {
            Screen::Album
        };
        Self::go(m, screen, key);
        Effect::None
    }
}

// ---------------------------------------------------------------- drawing

/// Row heights: headings (the rows that cannot be chosen) are short.
fn row_h(item: &Item) -> f32 {
    if item.enabled {
        46.
    } else {
        24.
    }
}

/// The end of `s` that fits in `max`, led by an ellipsis when cut.
fn tail(face: Face, s: &str, max: f32) -> String {
    let chars: Vec<char> = s.chars().collect();
    (0..chars.len())
        .map(|start| {
            let rest: String = chars[start..].iter().collect();
            if start == 0 {
                rest
            } else {
                format!("…{rest}")
            }
        })
        .find(|shown| text::width(face, shown) <= max)
        .unwrap_or_default()
}

pub(crate) fn draw(c: &mut Canvas, ui: &Ui, m: &AppModel, tracks: &[Track], interactive: bool) {
    let s = &ui.search;
    let typing = !s.results;
    let accent = ui.accent();
    components::status_bar(c, ui, m, "SEARCH", true);

    // The field: what is typed (its end kept in view), a cursor while typing.
    let (fx, fy, fw, fh) = (16., layout::LIST_TOP + 4., WIDTH - 32., 40.);
    let field = if typing {
        color::GLASS_STRONG
    } else {
        color::GLASS
    };
    c.rrect(fx, fy, fw, fh, fh / 2., field);
    let mid = fy + fh / 2.;
    c.icon(Icon::Search, fx + 22., mid, 18., color::MUTED);
    let mut room = fw - 58.;
    if !typing {
        let hint = "Back: edit";
        c.text_mid_right(fx + fw - 18., mid, hint, Face::R11, color::DIM);
        room -= text::width(Face::R11, hint) + 10.;
    }
    if s.query.is_empty() {
        let hint = "Artists, albums, songs";
        c.text_mid(fx + 46., mid, room, hint, Face::R15, color::DIM);
        if typing {
            c.rect(fx + 40., mid - 12., 2., 24., accent);
        }
    } else {
        let shown = tail(Face::S20, &s.query, room);
        let w = c.text(
            fx + 40.,
            mid + Face::S20.mid_offset(),
            &shown,
            Face::S20,
            color::TEXT,
        );
        if typing {
            c.rect((fx + 42. + w).round(), mid - 12., 2., 24., accent);
        }
    }

    let (mut top, mut bottom) = (fy + fh + 8., HEIGHT - 6.);
    if typing {
        top = ring(c, s, fy + fh + 26., interactive) + 12.;
        bottom = HEIGHT - 22.;
        c.text_center(
            240.,
            HEIGHT - 10. + Face::R11.mid_offset(),
            "Select: add  ·  Back: delete  ·  Hold Select: results",
            Face::R11,
            color::DIM,
        );
    }

    let rows = ui.rows(m, tracks);
    if rows.is_empty() {
        let (title, body) = if s.query.trim().is_empty() {
            (
                "Find music".to_owned(),
                "Turn the wheel to pick letters. Results appear as you type.",
            )
        } else {
            (
                format!("No matches for \u{201c}{}\u{201d}", s.query.trim()),
                "Try fewer letters, or the start of a word.",
            )
        };
        let cy = (top + bottom) / 2. - 14.;
        c.text_center(
            240.,
            cy + Face::S17.mid_offset(),
            &text::fit(Face::S17, &title, 420.),
            Face::S17,
            color::TEXT,
        );
        for (n, line) in text::wrap(Face::R13, body, 400., 2).iter().enumerate() {
            c.text_center(
                240.,
                cy + 26. + n as f32 * 18. + Face::R13.mid_offset(),
                line,
                Face::R13,
                color::MUTED,
            );
        }
        return;
    }
    let focus = m.navigation.focus.min(rows.len() - 1);
    let area = bottom - top;
    // The first row drawn: the list's scroll, moved on until the focus fits.
    let fits = |from: usize| rows[from..=focus].iter().map(row_h).sum::<f32>() <= area;
    let mut start = if typing {
        0
    } else {
        m.navigation.scroll.min(focus)
    };
    while !typing && start < focus && !fits(start) {
        start += 1;
    }
    // Keep a group's heading above its first row when there is room.
    if !typing && start > 0 && !rows[start - 1].enabled && fits(start - 1) {
        start -= 1;
    }
    let mut y = top;
    let mut drawn = 0;
    for (n, row) in rows.iter().enumerate().skip(start) {
        let h = row_h(row);
        // A heading needs room for a row under it too.
        let needs = match rows.get(n + 1) {
            Some(next) if !row.enabled => h + row_h(next),
            _ => h,
        };
        if y + needs > bottom + 1. {
            break;
        }
        let selected = interactive && !typing && n == focus;
        draw_row(c, ui, tracks, row, y, selected, accent);
        y += h;
        drawn += 1;
    }
    components::scroll_rail(c, top, area, start, rows.len(), drawn, accent);
}

/// The character ring centred at `cy`, styled as the password wheel; returns
/// its bottom edge.
fn ring(c: &mut Canvas, s: &SearchView, cy: f32, interactive: bool) -> f32 {
    let focused = s.key();
    for offset in -5i32..=5 {
        let key = Key::at((s.stop as i32 + offset).rem_euclid(Key::COUNT as i32) as usize);
        let cx = 240. + offset as f32 * 42.;
        if offset == 0 {
            let label = key.label(true);
            let face = if matches!(key, Key::Char(_)) {
                Face::S25
            } else {
                Face::S15
            };
            let w = (text::width(face, &label) + 26.).max(44.);
            let at = c.draw.len();
            c.rrect(cx - w / 2., cy - 19., w, 38., 14., 0xF2F3F5FF);
            if interactive {
                c.mark_focus(at);
            }
            c.text_mid_center(cx, cy, &label, face, color::INK);
        } else {
            let face = match key {
                Key::Char(_) if offset.abs() == 1 => Face::R17,
                Key::Char(_) => Face::R15,
                _ => Face::R11,
            };
            // Room for a wide focused word: its neighbours step aside.
            let shift = if matches!(focused, Key::Char(_)) {
                0.
            } else {
                offset.signum() as f32 * 18.
            };
            let tint = if offset.abs() <= 2 {
                color::MUTED
            } else {
                color::DIM
            };
            c.text_mid_center(cx + shift, cy, &key.label(false), face, tint);
        }
    }
    cy + 19.
}

fn draw_row(
    c: &mut Canvas,
    ui: &Ui,
    tracks: &[Track],
    row: &Item,
    y: f32,
    selected: bool,
    accent: u32,
) {
    if !row.enabled {
        components::section_label(c, 22., y + 14., &row.label);
        c.text_mid_right(WIDTH - 28., y + 14., &row.secondary, Face::R11, color::DIM);
        return;
    }
    if selected {
        components::accent_row(c, 10., y + 2., WIDTH - 24., 42., accent);
    }
    let (ax, ay, size) = (20., y + 6., 34.);
    let (x, w) = (66., WIDTH - 100.);
    let face = if selected { Face::S15 } else { Face::R15 };
    if row.key.starts_with("more:") {
        c.icon(Icon::Chevron, ax + size / 2., ay + size / 2., 16., accent);
        c.text_mid(x, y + 23., w, &row.label, face, accent);
        return;
    }
    let art = row.art.and_then(|i| tracks.get(i).map(|t| (i, t)));
    components::cover(c, ui, art, ax, ay, size);
    if row.key.starts_with("artist:") {
        // Artists are round, as on the Artists list.
        let bg = if selected {
            mix(color::BG, accent, 0.18)
        } else {
            color::BG
        };
        c.round_off(ax, ay, size, size / 2., bg);
    }
    c.text_mid(x, y + 15., w, &row.label, face, color::TEXT);
    let detail = if selected { accent } else { color::MUTED };
    c.text_mid(x, y + 33., w, &row.secondary, Face::R11, detail);
}

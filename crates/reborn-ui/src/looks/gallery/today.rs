//! Today, Gallery's home: a greeting for the local time and the date, the
//! "for right now" card that shuffles everything, then shelves of cards
//! built from the library on the player: albums new on it, a handful
//! picked for the day, the artists with the most albums and the library's
//! other ways in.
//!
//! The wheel walks every card in reading order: the hero, then across a
//! shelf and on to the next, which slides up into view.
use super::kit::{self, display, kicker, plural, top_text, CLEAR};
use super::palette::{Colors, FG, MUTED};
use super::{now, State, GUTTER};
use crate::{
    canvas::{Canvas, Icon},
    components,
    look::Frame,
    text::Face,
    theme::{alpha, fade, mix, HEIGHT, WIDTH},
    Ui,
};
use reborn_core::{clock::Local, AppModel, Effect, Screen, Track};
use reborn_graphics::Quad;
use std::collections::HashMap;

/// What a card shows as its picture.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Art {
    /// A library song's cover.
    Track(usize),
    /// An artist: their first album's cover, round.
    Portrait(usize),
    /// A library destination: an icon on a quiet tile.
    Tile(Icon),
}

/// What Select on a card does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Go {
    Album(String),
    Artist(String),
    /// A Home row key (`genres`, `settings`), handled as the standard rows are.
    Key(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Card {
    pub title: String,
    pub sub: String,
    pub art: Art,
    pub go: Go,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shelf {
    pub title: &'static str,
    pub sub: &'static str,
    pub cards: Vec<Card>,
    /// A short strip of small cards under the hero (Today with nothing
    /// playing has the room).
    pub compact: bool,
}

/// Cards per shelf, at most.
const SHELF_MAX: usize = 10;
/// The compact strip under the hero: cards, its top and height.
const LEAD: usize = 3;
const LEAD_Y: f32 = 228.;
const LEAD_H: f32 = 64.;

// ------------------------------------------------------------------ layout
// One 4 px scale; a 16 px gutter; cards 16, covers 12, pills round.

/// The greeting's top, the date's, the hero's.
const GREETING_TOP: f32 = 40.;
const DATE_TOP: f32 = 80.;
const HERO_TOP: f32 = 104.;
const HERO_H: f32 = 112.;
/// The first shelf's top, and the distance from one shelf to the next.
const SHELF_TOP: f32 = 272.;
const SHELF_PITCH: f32 = 212.;
/// A shelf's title sits this far below the status bar once it is in view.
const SHELF_VIEW: f32 = 44.;
/// Covers, and the step from one card to the next.
const COVER: f32 = 96.;
const STEP: f32 = 112.;
/// The page and a shelf glide this long.
const SCROLL_MS: u32 = 360;
const FOCUS_MS: u32 = 220;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The greeting for the hour.
pub(crate) fn greeting(hour: u8) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=20 => "Good evening",
        _ => "Late night",
    }
}

/// Days since 1970-01-01 of a calendar date.
fn days(t: &Local) -> i64 {
    let (y, m) = if t.month <= 2 {
        (i64::from(t.year) - 1, i64::from(t.month) + 9)
    } else {
        (i64::from(t.year), i64::from(t.month) - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + i64::from(t.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "Monday, October 12".
pub(crate) fn date_line(t: &Local) -> String {
    // 1970-01-01 was a Thursday.
    let weekday = (days(t) + 4).rem_euclid(7) as usize;
    let month = MONTHS
        .get(usize::from(t.month.saturating_sub(1)))
        .copied()
        .unwrap_or("");
    format!("{}, {month} {}", WEEKDAYS[weekday], t.day)
}

/// The card for the album `tracks[i]` stands for.
fn album_card(tracks: &[Track], i: usize) -> Card {
    let t = &tracks[i];
    let artist = if t.album_artist.is_empty() {
        &t.artist
    } else {
        &t.album_artist
    };
    Card {
        title: crate::display_or_unknown(&t.album).to_owned(),
        sub: crate::display_or_unknown(artist).to_owned(),
        art: Art::Track(i),
        go: Go::Album(crate::album_filter(t)),
    }
}

/// The track standing for each album: the first with artwork.
fn album_heads(tracks: &[Track]) -> HashMap<(&str, &str), usize> {
    let mut heads: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        if !t.online || t.album.trim().is_empty() {
            continue;
        }
        let slot = heads
            .entry((t.album_artist.as_str(), t.album.as_str()))
            .or_insert(i);
        if !tracks[*slot].artwork && t.artwork {
            *slot = i;
        }
    }
    heads
}

/// A number that orders `s` differently every day (FNV-1a over the text and
/// the day).
fn daily(s: &str, day: i64) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in s.bytes().chain(day.to_le_bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    h
}

/// Today's shelves, from the library alone. Empty shelves are left out; the
/// library shelf is always there (it is the way to everything else).
pub(crate) fn build(ui: &Ui, m: &AppModel, tracks: &[Track], day: i64) -> Vec<Shelf> {
    let heads = album_heads(tracks);
    let mut shelves = vec![];

    // Albums newest on the player.
    let mut newest: HashMap<(&str, &str), i64> = HashMap::new();
    for t in tracks.iter().filter(|t| t.online) {
        let at = newest
            .entry((t.album_artist.as_str(), t.album.as_str()))
            .or_insert(t.mtime);
        *at = (*at).max(t.mtime);
    }
    let mut fresh: Vec<(i64, usize)> = heads
        .iter()
        .map(|(k, &h)| (newest.get(k).copied().unwrap_or(0), h))
        .collect();
    fresh.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    // With nothing playing, the three newest sit under the hero.
    let idle = m.current().is_none() && fresh.len() > LEAD;
    let lead: Vec<Card> = if idle {
        fresh
            .drain(..LEAD)
            .map(|(_, i)| album_card(tracks, i))
            .collect()
    } else {
        vec![]
    };
    if !fresh.is_empty() {
        shelves.push(Shelf {
            title: "New in your library",
            sub: if idle {
                "More added to the player lately"
            } else {
                "Freshly added to the player"
            },
            cards: fresh
                .iter()
                .take(SHELF_MAX)
                .map(|(_, i)| album_card(tracks, *i))
                .collect(),
            compact: false,
        });
    }

    // A handful of albums for the day, a different handful tomorrow.
    let mut picks: Vec<(u64, usize)> = heads
        .iter()
        .map(|((artist, album), &h)| (daily(&format!("{artist}\u{1f}{album}"), day), h))
        .collect();
    picks.sort_unstable();
    if picks.len() > LEAD {
        shelves.push(Shelf {
            title: "Picked for today",
            sub: "A different handful every day",
            cards: picks
                .into_iter()
                .take(6)
                .map(|(_, i)| album_card(tracks, i))
                .collect(),
            compact: false,
        });
    }

    // The artists with the most albums, each by their first album's cover.
    let mut artists: HashMap<&str, (usize, usize)> = HashMap::new();
    for &h in heads.values() {
        let name = tracks[h].artist.as_str();
        if name.trim().is_empty() {
            continue;
        }
        let slot = artists.entry(name).or_insert((0, h));
        slot.0 += 1;
        slot.1 = slot.1.min(h);
    }
    let mut artists: Vec<(&str, usize, usize)> =
        artists.into_iter().map(|(n, (a, h))| (n, a, h)).collect();
    artists.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    if artists.len() > 1 {
        shelves.push(Shelf {
            title: "Your artists",
            sub: "The ones with the most on the player",
            cards: artists
                .into_iter()
                .take(SHELF_MAX)
                .map(|(name, albums, h)| Card {
                    title: name.to_owned(),
                    sub: plural(albums, "album", "albums"),
                    art: Art::Portrait(h),
                    go: Go::Artist(format!("artist:{name}")),
                })
                .collect(),
            compact: false,
        });
    }

    // The library: every other way in.
    let [artists, albums, songs, genres] = ui.library_counts(tracks);
    let tile = |title: &str, sub: String, icon: Icon, key: &'static str| Card {
        title: title.into(),
        sub,
        art: Art::Tile(icon),
        go: Go::Key(key),
    };
    shelves.push(Shelf {
        title: "Your library",
        sub: "Everything on the player",
        cards: vec![
            tile(
                "Albums",
                plural(albums, "album", "albums"),
                Icon::Vinyl,
                "albums",
            ),
            tile(
                "Artists",
                plural(artists, "artist", "artists"),
                Icon::Person,
                "artists",
            ),
            tile(
                "Genres",
                plural(genres, "genre", "genres"),
                Icon::Eq,
                "genres",
            ),
            tile(
                "Songs",
                plural(songs, "song", "songs"),
                Icon::Note,
                "tracks",
            ),
            tile("Folders", "On the player".into(), Icon::Folder, "folders"),
            tile("Settings", "Make it yours".into(), Icon::Sun, "settings"),
        ],
        compact: false,
    });
    if !lead.is_empty() {
        shelves.insert(
            0,
            Shelf {
                title: "Recently added",
                sub: "",
                cards: lead,
                compact: true,
            },
        );
    }
    shelves
}

/// A stamp of everything the shelves are built from.
pub(crate) type Stamp = (usize, usize, bool, i64);

/// The day Today is built for: the local date, or day 0 before the clock
/// is set.
fn today(ui: &Ui, m: &AppModel) -> i64 {
    crate::time_zone::local(ui, m).map_or(0, |t| days(&t))
}

/// Run `f` with Today's shelves, rebuilding them when what they show changed.
pub(crate) fn with_shelves<R>(
    ui: &Ui,
    m: &AppModel,
    tracks: &[Track],
    f: impl FnOnce(&[Shelf]) -> R,
) -> R {
    let st = &ui.gallery;
    let day = today(ui, m);
    let stamp = (
        tracks.as_ptr() as usize,
        tracks.len(),
        m.current().is_none(),
        day,
    );
    if st.shelves.borrow().0 != Some(stamp) {
        let built = build(ui, m, tracks, day);
        *st.shelves.borrow_mut() = (Some(stamp), built);
    }
    f(&st.shelves.borrow().1)
}

/// Where focus index `focus` is: `None` for the hero, else (shelf, card).
pub(crate) fn locate(shelves: &[Shelf], focus: usize) -> Option<(usize, usize)> {
    let mut n = focus.checked_sub(1)?;
    for (s, shelf) in shelves.iter().enumerate() {
        if n < shelf.cards.len() {
            return Some((s, n));
        }
        n -= shelf.cards.len();
    }
    None
}

/// The focus index of a shelf's first card.
fn first_of(shelves: &[Shelf], shelf: usize) -> usize {
    1 + shelves[..shelf]
        .iter()
        .map(|s| s.cards.len())
        .sum::<usize>()
}

/// Everything the wheel can reach: the hero and every card.
pub(crate) fn count(shelves: &[Shelf]) -> usize {
    1 + shelves.iter().map(|s| s.cards.len()).sum::<usize>()
}

// ------------------------------------------------------------------- input

/// Everything on the player, shuffled.
fn shuffle_all(m: &mut AppModel, tracks: &[Track]) -> Effect {
    let members: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].online).collect();
    let Some(&selected) = members.first() else {
        return Effect::None;
    };
    Ui::go(m, Screen::NowPlaying, "");
    Effect::PlayCollection {
        selected,
        members,
        shuffle: true,
    }
}

/// The wheel and Select on Today. `None` leaves the action to the player.
pub(crate) fn input(
    ui: &mut Ui,
    m: &mut AppModel,
    tracks: &[Track],
    action: reborn_core::Action,
) -> Option<Effect> {
    use reborn_core::Action as A;
    let (count, focus, place, firsts, go) = with_shelves(ui, m, tracks, |shelves| {
        let count = count(shelves);
        let focus = ui.gallery.focus.get().min(count - 1);
        let place = locate(shelves, focus);
        let go = place.map(|(s, c)| shelves[s].cards[c].go.clone());
        let firsts: Vec<usize> = (0..shelves.len()).map(|s| first_of(shelves, s)).collect();
        (count, focus, place, firsts, go)
    });
    let st = &ui.gallery;
    let set = |to: usize| st.focus.set(to.min(count - 1));
    match action {
        A::WheelClockwise(_) | A::NavigateRight => set(focus + 1),
        A::WheelCounterClockwise(_) | A::NavigateLeft => set(focus.saturating_sub(1)),
        A::NavigateDown => set(match place {
            None => firsts.first().copied().unwrap_or(0),
            Some((s, _)) => firsts.get(s + 1).copied().unwrap_or(focus),
        }),
        A::NavigateUp => set(match place {
            None | Some((0, _)) => 0,
            Some((s, _)) => firsts[s - 1],
        }),
        A::Select => {
            return Some(match go {
                None => shuffle_all(m, tracks),
                Some(Go::Album(key)) => {
                    Ui::go(m, Screen::Album, key);
                    Effect::None
                }
                Some(Go::Artist(key)) => {
                    Ui::go(m, Screen::Artist, key);
                    Effect::None
                }
                Some(Go::Key(key)) => ui.select_row(m, tracks, key),
            })
        }
        _ => return None,
    }
    Some(Effect::None)
}

// ----------------------------------------------------------------- drawing

/// The hero's gradient for a moment of the day, deepened a little so white
/// type reads on every stop.
fn mood(name: &str) -> (u32, u32) {
    let (from, to) = match name {
        "Slow Morning" => (0xD9774AFF, 0x8E3F6EFF),
        "Afternoon Lift" => (0x2E8C8CFF, 0x3B4FA8FF),
        "Golden Hour" => (0xE0952EFF, 0xB0305AFF),
        _ => (0x3B3F8FFF, 0x2E6F8CFF),
    };
    (mix(from, 0x000000FF, 0.14), mix(to, 0x000000FF, 0.3))
}

/// The moment of the day the hero is named after.
fn moment(hour: u8) -> &'static str {
    match hour {
        5..=11 => "Slow Morning",
        12..=16 => "Afternoon Lift",
        17..=20 => "Golden Hour",
        _ => "Night Swim",
    }
}

/// The page's scroll for a focus: the hero shows the greeting; a shelf
/// sits just under the status bar.
fn page_target(shelves: &[Shelf], place: Option<(usize, usize)>) -> f32 {
    match place {
        None => 0.,
        Some((s, _)) if shelves[s].compact => 0.,
        Some((s, _)) => shelf_top(shelves, s) - SHELF_VIEW,
    }
}

/// Where shelf `s` starts on the page (a compact lead strip takes no
/// place in the run of shelves).
fn shelf_top(shelves: &[Shelf], s: usize) -> f32 {
    let lead = usize::from(shelves.first().is_some_and(|x| x.compact));
    SHELF_TOP + lead as f32 * 48. + s.saturating_sub(lead) as f32 * SHELF_PITCH
}

/// A shelf's sideways scroll that keeps card `card` in view, third from
/// the left once the row has moved.
pub(crate) fn row_target(cards: usize, card: usize) -> f32 {
    let content = cards as f32 * STEP - (STEP - COVER);
    let max = (content - (WIDTH - 2. * GUTTER)).max(0.);
    ((card as f32 - 2.) * STEP).clamp(0., max)
}

pub(crate) fn draw(c: &mut Canvas, f: &Frame, colors: &Colors) {
    let (ui, m) = (f.ui, f.m);
    let st: &State = &ui.gallery;
    let now = now(ui);
    with_shelves(ui, m, f.tracks, |shelves| {
        let total = count(shelves);
        let focus = st.focus.get().min(total - 1);
        st.focus.set(focus);
        let place = locate(shelves, focus);
        let scroll = st
            .page
            .toward(page_target(shelves, place), now, SCROLL_MS, &st.clock);
        let shown = |y: f32, h: f32| y - scroll + h > 0. && y - scroll < HEIGHT;
        let lit = f.interactive && !st.pill.get();
        let arrive = st.focus_in.progress(focus, now, FOCUS_MS, &st.clock);

        // The greeting and the date.
        let local = crate::time_zone::local(ui, m);
        if shown(GREETING_TOP, 80.) {
            let hello = local.as_ref().map_or("Hello", |t| greeting(t.hour));
            display(
                c,
                GUTTER,
                GREETING_TOP - scroll,
                WIDTH - 2. * GUTTER,
                hello,
                Face::SY34,
                FG,
            );
            let [_, albums, _, _] = ui.library_counts(f.tracks);
            let mut line = local.as_ref().map(date_line).unwrap_or_default();
            if albums > 0 {
                if !line.is_empty() {
                    line.push_str("  ·  ");
                }
                line.push_str(&plural(albums, "album", "albums"));
                line.push_str(" on the player");
            }
            top_text(
                c,
                GUTTER,
                DATE_TOP - scroll,
                WIDTH - 2. * GUTTER,
                &line,
                Face::PM11,
                MUTED,
            );
        }

        // The hero.
        if shown(HERO_TOP, HERO_H) || (lit && place.is_none()) {
            let hour = local.as_ref().map_or(12, |t| t.hour);
            hero(
                c,
                colors,
                HERO_TOP - scroll,
                moment(hour),
                lit && place.is_none(),
                arrive,
            );
        }

        // The shelves.
        let mut rows = st.rows.borrow_mut();
        rows.resize_with(shelves.len(), Default::default);
        for (s, shelf) in shelves.iter().enumerate() {
            let here = place.filter(|p| p.0 == s).map(|p| p.1);
            if shelf.compact {
                if shown(LEAD_Y, LEAD_H) || here.is_some() {
                    let focus = here.filter(|_| lit);
                    lead_strip(c, f, colors, shelf, LEAD_Y - scroll, focus, arrive);
                }
                continue;
            }
            let top = shelf_top(shelves, s);
            let aim = here.map_or_else(
                || rows[s].target().unwrap_or(0.),
                |card| row_target(shelf.cards.len(), card),
            );
            let off = rows[s].toward(aim, now, SCROLL_MS, &st.clock);
            if !shown(top, SHELF_PITCH) && here.is_none() {
                continue;
            }
            let y = top - scroll;
            if y >= CLEAR {
                display(
                    c,
                    GUTTER,
                    y,
                    WIDTH - 2. * GUTTER,
                    shelf.title,
                    Face::SY20,
                    FG,
                );
            }
            if y + 26. >= CLEAR {
                top_text(
                    c,
                    GUTTER,
                    y + 26.,
                    WIDTH - 2. * GUTTER,
                    shelf.sub,
                    Face::DM13,
                    MUTED,
                );
            }
            for (n, card) in shelf.cards.iter().enumerate() {
                let x = GUTTER + n as f32 * STEP - off;
                if x > WIDTH || x + COVER < 0. {
                    continue;
                }
                let focused = lit && here == Some(n);
                draw_card(c, f, colors, card, x, y + 48., focused, arrive);
            }
        }
    });
}

/// The compact strip under the hero: a mono kicker and three small cards,
/// each a cover, a title and a muted line.
fn lead_strip(
    c: &mut Canvas,
    f: &Frame,
    colors: &Colors,
    shelf: &Shelf,
    y: f32,
    focus: Option<usize>,
    arrive: f32,
) {
    kicker(
        c,
        GUTTER,
        y,
        WIDTH - 2. * GUTTER,
        shelf.title,
        colors.accent,
    );
    let (w, h, gap) = (144., 48., 8.);
    let r = kit::THUMB_R + 4.;
    for (n, card) in shelf.cards.iter().enumerate() {
        let x = GUTTER + n as f32 * (w + gap);
        let cy = y + 16.;
        let focused = focus == Some(n);
        if focused {
            kit::focus_glow(c, colors, x, cy, w, h, arrive);
        }
        let fill = mix(colors.surface, FG, if focused { 0.09 } else { 0.05 });
        c.rrect(x, cy, w, h, r, fill);
        c.rrect_outline(x, cy, w, h, r, 1., kit::HAIRLINE);
        let a = 32.;
        let art = match card.art {
            Art::Track(i) | Art::Portrait(i) => Some(i),
            Art::Tile(_) => None,
        };
        kit::cover(
            c,
            f,
            colors,
            art,
            x + 8.,
            cy + 8.,
            a,
            kit::THUMB_R,
            Some(fill),
        );
        let tx = x + 8. + a + 10.;
        let tw = w - (tx - x) - 8.;
        top_text(c, tx, cy + 8., tw, &card.title, Face::DMB14, FG);
        top_text(c, tx, cy + 26., tw, &card.sub, Face::DM12, MUTED);
        if focused {
            kit::focus_ring(c, colors, x, cy, w, h, r, arrive);
        }
    }
}

/// The "for right now" card: a gradient for the moment of the day, a mono
/// kicker, the moment's name and a white play disc.
fn hero(c: &mut Canvas, colors: &Colors, y: f32, name: &str, focused: bool, arrive: f32) {
    let (x, w, h) = (GUTTER, WIDTH - 2. * GUTTER, HERO_H);
    let lift = if focused { -2. * arrive } else { 0. };
    let y = y + lift;
    let (from, to) = mood(name);
    if focused {
        c.glow(
            x + w / 2.,
            y + h / 2. + 10.,
            w * 0.52,
            h * 0.85,
            fade(alpha(colors.accent, 0x50), arrive),
        );
    } else {
        c.glow(x + w / 2., y + h / 2. + 12., w * 0.5, h * 0.7, 0x00000060);
    }
    kit::rrect_horizontal(c, x, y, w, h, 16., from, to);
    // Light falling from the top left.
    c.glow(x + 96., y + 8., 200., 72., 0xFFFFFF1C);
    if focused {
        let at = c.draw.len();
        c.rrect_outline(x, y, w, h, 16., 2., fade(0xFFFFFFA8, 0.4 + 0.6 * arrive));
        c.mark_focus(at);
    } else {
        c.rrect_outline(x, y, w, h, 16., 1., 0xFFFFFF1F);
    }
    let tx = x + 20.;
    let tw = w - 20. - 76.;
    kicker(c, tx, y + 20., tw, "Shuffle · for right now", 0xFFFFFFC8);
    display(c, tx, y + 36., tw, name, Face::SY26, 0xFFFFFFFF);
    top_text(
        c,
        tx,
        y + 76.,
        tw,
        "Everything on the player, in a new order.",
        Face::DM13,
        0xFFFFFFD0,
    );
    let (cx, cy) = (x + w - 20. - 24., y + 20. + 24.);
    c.glow(cx, cy + 4., 36., 32., 0x00000050);
    c.disc(cx, cy, 24., 0xFFFFFFFF);
    // The triangle's weight sits left of its box: nudge it right.
    c.icon(Icon::Play, cx + 2., cy, 20., 0x111217FF);
}

/// One card: the picture (cover, portrait or a library tile), its title and
/// a muted line. The focused card lifts on a soft glow with a ring.
#[allow(clippy::too_many_arguments)]
fn draw_card(
    c: &mut Canvas,
    f: &Frame,
    colors: &Colors,
    card: &Card,
    x: f32,
    y: f32,
    focused: bool,
    arrive: f32,
) {
    // The artwork lifts; the text stays on the row's baseline.
    let base = y;
    let lift = if focused { (-4. * arrive).round() } else { 0. };
    let y = y + lift;
    let round = matches!(card.art, Art::Portrait(_));
    let r = if round { COVER / 2. } else { kit::COVER_R };
    let glow: kit::Wash = (
        x + COVER / 2.,
        y + COVER / 2. + 8.,
        COVER * 0.75,
        COVER * 0.7,
        fade(alpha(colors.accent, 0x66), arrive),
    );
    if focused {
        c.glow(glow.0, glow.1, glow.2, glow.3, glow.4);
    } else if !round {
        // A portrait's large corners take the room's colour, so it casts no
        // shadow for them to cover.
        components::cover_shadow(c, x, y, COVER);
    }
    match &card.art {
        Art::Track(i) => kit::cover(c, f, colors, Some(*i), x, y, COVER, r, None),
        Art::Portrait(i) => {
            let key = f.tracks.get(*i).map(|t| (*i, t));
            components::cover_rounded(c, f.ui, key, x, y, COVER, r);
            let wash = focused.then_some(glow);
            kit::round_corners(c, colors, (x, y, COVER), r, None, wash);
        }
        Art::Tile(icon) => {
            // A quiet tile lit from the top left, the icon at its centre.
            let tint = mix(colors.surface, colors.accent, 0.16);
            c.draw
                .push(Quad::rect(x, y, COVER, COVER, mix(tint, FG, 0.06)).vertical(colors.surface));
            kit::round_corners(c, colors, (x, y, COVER), r, None, None);
            c.glow(x + 30., y + 24., 56., 44., alpha(colors.accent, 0x26));
            c.rrect_outline(x, y, COVER, COVER, r, 1., 0xF2F3F71A);
            c.icon(
                *icon,
                x + COVER / 2.,
                y + COVER / 2.,
                32.,
                mix(colors.accent, FG, 0.55),
            );
        }
    }
    if focused {
        let tint = fade(kit::ring_tint(colors), 0.35 + 0.65 * arrive);
        let at = c.draw.len();
        if round {
            let (cx, cy) = (x + COVER / 2., y + COVER / 2.);
            super::icons::circle(c, cx, cy, COVER / 2. + 4., 2., tint);
        } else {
            c.rrect_outline(x - 3., y - 3., COVER + 6., COVER + 6., r + 3., 2., tint);
        }
        c.mark_focus(at);
    }
    let tw = COVER + 4.;
    let y = base;
    if y + COVER + 10. >= CLEAR {
        top_text(c, x, y + COVER + 10., tw, &card.title, Face::DMB14, FG);
    }
    if y + COVER + 28. >= CLEAR {
        top_text(c, x, y + COVER + 28., tw, &card.sub, Face::DM12, MUTED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_the_focus_third_from_the_left() {
        assert_eq!(row_target(10, 0), 0.);
        assert_eq!(row_target(10, 2), 0.);
        assert_eq!(row_target(10, 3), STEP);
        // The last card ends at the right gutter, not beyond.
        assert_eq!(row_target(10, 9), 10. * STEP - 16. - 448.);
        assert_eq!(row_target(3, 2), 0., "a short shelf never moves");
    }

    #[test]
    fn greetings_follow_the_hour() {
        assert_eq!(greeting(9), "Good morning");
        assert_eq!(greeting(12), "Good afternoon");
        assert_eq!(greeting(19), "Good evening");
        assert_eq!(greeting(23), "Late night");
        assert_eq!(greeting(3), "Late night");
    }

    #[test]
    fn the_date_line_names_the_weekday() {
        let at = |year, month, day| Local {
            year,
            month,
            day,
            hour: 9,
            minute: 41,
        };
        assert_eq!(date_line(&at(2026, 10, 12)), "Monday, October 12");
        assert_eq!(date_line(&at(2024, 2, 29)), "Thursday, February 29");
        assert_eq!(date_line(&at(1970, 1, 1)), "Thursday, January 1");
        assert_eq!(days(&at(1970, 1, 2)), 1);
    }

    #[test]
    fn the_daily_picks_change_with_the_day_only() {
        assert_eq!(daily("a", 7), daily("a", 7));
        assert_ne!(daily("a", 7), daily("a", 8));
        assert_ne!(daily("a", 7), daily("b", 7));
    }

    #[test]
    fn the_hero_names_fit_beside_the_play_disc() {
        for name in [
            "Slow Morning",
            "Afternoon Lift",
            "Golden Hour",
            "Night Swim",
        ] {
            assert!(crate::text::width_tracked(Face::SY26, name, kit::TIGHT) < 448. - 96.);
        }
    }
}

//! Gallery: the player as a gallery of its covers.
//!
//! Four tabs in a floating pill: Today (`Screen::Home`), Albums, Artists
//! and Songs. The selected tab is a solid capsule that slides between them,
//! with the mini player stacked above. The whole look takes its colour from
//! the album playing and eases to the next album's. Today is the home: a
//! greeting, the "for right now" card and shelves of cards the wheel walks
//! in reading order. Albums is a wall of covers; albums, Now Playing and
//! every list have Gallery's own layouts, set in DM Sans, Syne and IBM Plex
//! Mono; pages slide in from the right and back.
//!
//! The wheel model: Back at a tab's root moves the focus to the pill, the
//! wheel slides between tabs and Select enters one (it becomes the root).
//! Everything else keeps its usual meaning.
mod album;
mod chrome;
mod icons;
mod kit;
mod list;
mod motion;
mod palette;
mod playing;
mod today;
mod wall;

#[cfg(test)]
mod tests;

use self::icons::Glyph;
use self::motion::{ease, Clock, Fade, Glide};
use self::palette::{Colors, FG, INK};
use crate::{
    canvas::Canvas,
    components::{SheetRow, Tone},
    look::{Frame, Look},
    text::{self, Face},
    theme::{alpha, fade, mix, HEIGHT, WIDTH},
    Ui,
};
use reborn_core::{Action, AppModel, Effect, Screen, Track};
use reborn_graphics::Quad;
use std::cell::{Cell, RefCell};
use std::time::Instant;

pub(crate) struct Gallery;

/// The outer margin everything lines up on.
pub(crate) const GUTTER: f32 = 16.;

/// The tabs: name, glyph and the Home row that opens each.
const TABS: [(&str, Glyph, &str); 4] = [
    ("Today", Glyph::Compass, ""),
    ("Albums", Glyph::Grid, "albums"),
    ("Artists", Glyph::Person, "artists"),
    ("Songs", Glyph::Note, "tracks"),
];

/// The pill and the mini player above it.
const PILL_H: f32 = 40.;
const PILL_Y: f32 = HEIGHT - 12. - PILL_H;
const MINI_Y: f32 = PILL_Y - 8. - kit::MINI_H;
const PILL_MS: u32 = 320;
const TAB_FADE_MS: u32 = 200;
/// A page slides in (or back out) this long.
const PUSH_MS: u32 = 360;

/// A page's identity, for telling a push from a pop.
type PageKey = (usize, Screen, String);

/// A page slide under way: +1 a push, -1 a pop, with the page it leaves.
struct Slide {
    dir: i8,
    since: Instant,
    old: Vec<Quad>,
}

#[derive(Default)]
struct Pages {
    /// The page drawn last, and its quads (for the next slide).
    key: Option<PageKey>,
    last: Vec<Quad>,
    slide: Option<Slide>,
}

/// What Gallery keeps between frames.
#[derive(Default)]
pub(crate) struct State {
    /// The focus is on the tab pill.
    pill: Cell<bool>,
    /// The tab the pill's capsule is on while it has the focus.
    pill_tab: Cell<usize>,
    /// Today's focus: 0 the hero, then every card in reading order.
    focus: Cell<usize>,
    /// Today's shelves and what they were built from.
    shelves: RefCell<(Option<today::Stamp>, Vec<today::Shelf>)>,
    /// Today's scroll, and each shelf's sideways scroll.
    page: Glide,
    rows: RefCell<Vec<Glide>>,
    /// The scroll of the page on screen (an album, the wall), and which
    /// page that is.
    sheet: Glide,
    sheet_for: RefCell<Option<(Screen, String)>>,
    /// The capsule's position, and the pill rising on tabs other than
    /// Today (0 hidden .. 1 up).
    slide: Glide,
    rise: Glide,
    /// The focus glow arriving on a card; the crossfade between tabs.
    focus_in: Fade,
    tab_in: Fade,
    /// Pages sliding in and out.
    pages: RefCell<Pages>,
    /// The colours on screen.
    colors: palette::Anim,
    clock: Clock,
}

impl State {
    /// Whether this frame's page is new since the last frame drawn (its
    /// scroll then starts from the top).
    fn fresh_page(&self, m: &AppModel) -> bool {
        let key = (m.screen, m.navigation.filter.clone());
        let mut seen = self.sheet_for.borrow_mut();
        if seen.as_ref() == Some(&key) {
            return false;
        }
        *seen = Some(key);
        true
    }
}

thread_local! {
    /// The colours of the frame being drawn, for what is drawn without the
    /// player's state (dialogs, sheets, toasts).
    static SHOWN: Cell<Colors> = const { Cell::new(INK) };
}

/// The clock Gallery's motion runs on: `None` in previews and tests, where
/// everything is drawn at rest.
fn now(ui: &Ui) -> Option<Instant> {
    (!ui.motion_fixed).then(Instant::now)
}

/// The tab whose root is on screen: the screen with nothing under it.
fn root_tab(m: &AppModel) -> Option<usize> {
    if !m.navigation.history.is_empty() || !m.navigation.stack.is_empty() {
        return None;
    }
    match m.screen {
        Screen::Home => Some(0),
        Screen::Albums if m.navigation.filter.is_empty() => Some(1),
        Screen::Artists => Some(2),
        Screen::Tracks if m.navigation.filter.is_empty() => Some(3),
        _ => None,
    }
}

/// Open `tab` as the root: what its Home row opens, with nothing under it.
fn open_tab(ui: &mut Ui, m: &mut AppModel, tracks: &[Track], tab: usize) -> Effect {
    Ui::home(m);
    let effect = match TABS[tab].2 {
        "" => Effect::None,
        key => ui.select_row(m, tracks, key),
    };
    m.navigation.stack.clear();
    m.navigation.history.clear();
    effect
}

/// The room: the background fading to its deeper tone, a glow from the top
/// left and a deeper one low on the right.
fn room(c: &mut Canvas, colors: &Colors) {
    c.draw
        .push(Quad::rect(0., 0., WIDTH, HEIGHT, colors.bg).vertical(colors.bg_deep));
    c.glow(64., 24., 360., 260., alpha(colors.glow_a, 0xA6));
    c.glow(448., 320., 320., 240., alpha(colors.glow_b, 0x8C));
}

/// Quads moved sideways onto the canvas; only a page arriving keeps its
/// focus.
fn place(c: &mut Canvas, quads: &[Quad], dx: f32, keep: bool) {
    c.draw.extend(quads.iter().map(|q| {
        let mut q = q.clone();
        q.x += dx;
        if let Some(w) = &mut q.warp {
            for corner in &mut w.corners {
                corner[0] += dx;
            }
        }
        q.focus_target &= keep;
        q
    }));
}

impl Gallery {
    /// Input on a tab's root and on the pill.
    fn tabs_input(
        ui: &mut Ui,
        m: &mut AppModel,
        tracks: &[Track],
        tab: usize,
        action: Action,
    ) -> Option<Effect> {
        let st = &ui.gallery;
        if st.pill.get() {
            let at = st.pill_tab.get();
            match action {
                Action::WheelClockwise(_) | Action::NavigateRight | Action::NavigateDown => {
                    st.pill_tab.set((at + 1).min(TABS.len() - 1));
                }
                Action::WheelCounterClockwise(_) | Action::NavigateLeft | Action::NavigateUp => {
                    st.pill_tab.set(at.saturating_sub(1));
                }
                Action::Back => st.pill.set(false),
                Action::Select => {
                    st.pill.set(false);
                    if at != tab {
                        return Some(open_tab(ui, m, tracks, at));
                    }
                }
                Action::ContextMenu => {}
                _ => return None,
            }
            return Some(Effect::None);
        }
        if action == Action::Back {
            st.pill.set(true);
            st.pill_tab.set(tab);
            return Some(Effect::None);
        }
        if tab == 0 {
            return today::input(ui, m, tracks, action);
        }
        None
    }

    /// The pill (and the mini player above it), raised by `rise` (0..1). On
    /// Artists and Songs the mini player rests at the foot without it.
    fn shell(c: &mut Canvas, f: &Frame, colors: &Colors, tab: usize, rise: f32) {
        let st = &f.ui.gallery;
        let dy = ((1. - rise) * (HEIGHT - MINI_Y + 16.)).round();
        let focused = st.pill.get() && f.interactive;
        if focused {
            // One focal point: the screen steps back behind the pill.
            c.rect(
                0.,
                0.,
                WIDTH,
                HEIGHT,
                fade(alpha(colors.bg_deep, 0x66), rise),
            );
        }
        let rests = matches!(tab, 2 | 3) && f.m.current().is_some();
        let mini_y = if rests {
            (kit::MINI_LOW + (MINI_Y - kit::MINI_LOW) * rise).round()
        } else {
            MINI_Y + dy
        };
        let veil = if rests {
            colors.bg_deep
        } else {
            fade(colors.bg_deep, rise)
        };
        // With nothing playing, the room shows down to the pill.
        let (anchor, soft) = if f.m.current().is_none() {
            (PILL_Y + dy - 4., 12.)
        } else {
            (mini_y, 48.)
        };
        c.draw.push(
            Quad::rect(0., anchor - soft, WIDTH, soft, alpha(colors.bg_deep, 0)).vertical(veil),
        );
        c.rect(0., anchor, WIDTH, HEIGHT - anchor, veil);
        kit::mini_player(c, f, colors, mini_y);
        if rise < 0.01 {
            return;
        }
        let on = if focused { st.pill_tab.get() } else { tab };
        let inner = (WIDTH - 2. * GUTTER - 8.) / TABS.len() as f32;
        let slide = st
            .slide
            .toward(on as f32 * inner, now(f.ui), PILL_MS, &st.clock);
        let y = PILL_Y + dy;
        let (x, w) = (GUTTER, WIDTH - 2. * GUTTER);
        c.glow(
            WIDTH / 2.,
            y + PILL_H / 2. + 8.,
            w / 2. + 8.,
            34.,
            0x00000070,
        );
        if focused {
            c.glow(
                x + 4. + slide + inner / 2.,
                y + PILL_H / 2.,
                inner * 0.9,
                40.,
                alpha(colors.accent, 0x70),
            );
            c.rrect_outline(
                x - 3.,
                y - 3.,
                w + 6.,
                PILL_H + 6.,
                (PILL_H + 6.) / 2.,
                2.,
                kit::ring_tint(colors),
            );
        }
        let fill = mix(colors.surface, FG, 0.04);
        c.rrect(x, y, w, PILL_H, PILL_H / 2., fill);
        c.rrect_outline(
            x,
            y,
            w,
            PILL_H,
            PILL_H / 2.,
            1.,
            if focused {
                alpha(kit::ring_tint(colors), 0x80)
            } else {
                kit::HAIRLINE
            },
        );
        let at = c.draw.len();
        c.rrect(
            x + 4. + slide,
            y + 4.,
            inner,
            PILL_H - 8.,
            (PILL_H - 8.) / 2.,
            FG,
        );
        if focused {
            c.mark_focus(at);
        }
        for (i, (label, glyph, _)) in TABS.iter().enumerate() {
            let cx = x + 4. + inner * (i as f32 + 0.5);
            // Ink flips as the capsule passes under a label.
            let under = 1. - ((x + 4. + slide + inner / 2. - cx).abs() / inner).clamp(0., 1.);
            let ink = mix(mix(FG, fill, 0.12), colors.bg, under);
            let lw = text::width(Face::DMB14, label);
            let gx = (cx - (16. + 6. + lw) / 2.).round();
            icons::draw(c, *glyph, gx + 8., y + PILL_H / 2., 17., ink);
            c.text(
                gx + 22.,
                y + PILL_H / 2. + Face::DMB14.mid_offset(),
                label,
                Face::DMB14,
                ink,
            );
        }
    }

    /// The page itself, without the slide.
    fn page(c: &mut Canvas, f: &Frame, colors: &Colors) {
        match f.m.screen {
            Screen::Home => today::draw(c, f, colors),
            Screen::Albums if f.ui.row_count(f.m, f.tracks) > 0 => wall::draw(c, f, colors),
            Screen::Album => album::draw(c, f, colors),
            Screen::NowPlaying if f.m.current().is_some() => playing::draw(c, f, colors),
            _ => list::draw(c, f, colors, root_tab(f.m).is_none()),
        }
    }

    fn draw_screen(c: &mut Canvas, f: &Frame, colors: &Colors) {
        let st = &f.ui.gallery;
        let now = now(f.ui);
        let tab = root_tab(f.m);
        if tab.is_none() {
            st.pill.set(false);
        }
        let start = c.draw.len();
        // With the pill focused the screen under it is drawn at rest.
        let pill = tab.is_some() && st.pill.get() && f.interactive;
        let under = Frame {
            ui: f.ui,
            m: f.m,
            tracks: f.tracks,
            has_art: f.has_art,
            interactive: f.interactive && !pill,
        };
        Self::page(c, &under, colors);
        if let Some(tab) = tab {
            // A new tab fades in from the room.
            let shown = st.tab_in.progress(tab, now, TAB_FADE_MS, &st.clock);
            if shown < 1. {
                c.rect(0., 0., WIDTH, HEIGHT, fade(colors.bg, 1. - shown));
            }
            if tab == 0 {
                st.rise.toward(1., None, PILL_MS, &st.clock);
                Self::shell(c, f, colors, tab, 1.);
                kit::status(c, f, colors);
            } else {
                let rise = st
                    .rise
                    .toward(if pill { 1. } else { 0. }, now, PILL_MS, &st.clock);
                if rise > 0.01 || matches!(tab, 2 | 3) {
                    Self::shell(c, f, colors, tab, rise);
                }
            }
        } else {
            st.rise.toward(0., None, PILL_MS, &st.clock);
        }
        Self::slide(c, f, colors, start, now);
    }

    /// Slide a pushed page in from the right over the one it leaves (which
    /// shifts a quarter left and dims), and the reverse on a pop.
    fn slide(c: &mut Canvas, f: &Frame, colors: &Colors, start: usize, now: Option<Instant>) {
        let st = &f.ui.gallery;
        let mut pages = st.pages.borrow_mut();
        let key: PageKey = (
            f.m.navigation.history.len(),
            f.m.screen,
            f.m.navigation.filter.clone(),
        );
        if let (Some(before), Some(now)) = (pages.key.clone(), now) {
            if before.0 != key.0 {
                let dir = if key.0 > before.0 { 1 } else { -1 };
                let old = std::mem::take(&mut pages.last);
                pages.slide = Some(Slide {
                    dir,
                    since: now,
                    old,
                });
            }
        }
        pages.key = Some(key);
        pages.last = c.draw[start..].to_vec();
        let Some(now) = now else {
            pages.slide = None;
            return;
        };
        let Some(slide) = &pages.slide else {
            return;
        };
        let t = now.saturating_duration_since(slide.since).as_secs_f32() * 1000. / PUSH_MS as f32;
        if t >= 1. {
            pages.slide = None;
            return;
        }
        st.clock.hold(slide.since, PUSH_MS);
        let p = ease(t);
        let new: Vec<Quad> = c.draw.split_off(start);
        // The page underneath, dimmed, then the one on top with its room.
        let (under, under_dx, top, top_dx, dim) = if slide.dir > 0 {
            (
                &slide.old,
                -0.25 * WIDTH * p,
                &new,
                (1. - p) * WIDTH,
                0.45 * p,
            )
        } else {
            (
                &new,
                -0.25 * WIDTH * (1. - p),
                &slide.old,
                p * WIDTH,
                0.45 * (1. - p),
            )
        };
        place(c, under, under_dx.round(), slide.dir < 0);
        c.rect(0., 0., WIDTH, HEIGHT, fade(colors.bg_deep, dim));
        let dx = top_dx.round();
        // A soft shadow along the top page's edge.
        c.glow(dx, HEIGHT / 2., 28., HEIGHT * 0.7, 0x00000070);
        let mut sheet = Canvas::new();
        room(&mut sheet, colors);
        place(c, &sheet.draw, dx, false);
        place(c, top, dx, slide.dir > 0);
    }
}

impl Look for Gallery {
    fn id(&self) -> &'static str {
        "gallery"
    }
    fn name(&self) -> &'static str {
        "Gallery"
    }
    fn about(&self) -> &'static str {
        "Covers first, in the album's colours"
    }
    fn accent(&self, cover: u32) -> u32 {
        if cover == 0 {
            INK.accent
        } else {
            palette::vivid(cover, 0.)
        }
    }
    /// Standard drawings still on screen (the volume line, the computer
    /// connection, text entry) read in Gallery's type.
    fn face(&self, face: Face) -> Face {
        match face {
            Face::R11 | Face::S10 => Face::DM12,
            Face::R13 | Face::S13 => Face::DM14,
            Face::R15 | Face::S15 => Face::DM15,
            Face::R17 | Face::S17 => Face::DM16,
            Face::S20 => Face::SY20,
            Face::L23 | Face::S25 => Face::SY26,
            Face::L44 | Face::S28 | Face::S36 | Face::Gallery(_) => face,
        }
    }

    /// The album's room, easing to the next album's colours.
    fn backdrop(&self, c: &mut Canvas, f: &Frame) {
        let st = &f.ui.gallery;
        let colors = st
            .colors
            .at(palette::of_cover(f.ui.accent), now(f.ui), &st.clock);
        SHOWN.with(|s| s.set(colors));
        room(c, &colors);
    }

    fn screen(&self, c: &mut Canvas, f: &Frame) -> bool {
        let colors = SHOWN.with(Cell::get);
        Self::draw_screen(c, f, &colors);
        if f.ui.gallery.clock.moving() {
            f.ui.look_moving.set(true);
        }
        true
    }
    fn status_bar(&self, c: &mut Canvas, ui: &Ui, m: &AppModel, title: &str, back: bool) -> bool {
        let _ = back;
        chrome::bar(c, ui, m, &SHOWN.with(Cell::get));
        chrome::status_title(c, title);
        true
    }
    fn dialog(
        &self,
        c: &mut Canvas,
        title: &str,
        body: &str,
        rows: &[SheetRow],
        focus: usize,
        danger: bool,
    ) -> bool {
        let _ = danger;
        chrome::dialog(c, &SHOWN.with(Cell::get), title, body, rows, focus);
        true
    }
    fn action_sheet(
        &self,
        c: &mut Canvas,
        title: &str,
        subtitle: &str,
        rows: &[SheetRow],
        focus: usize,
    ) -> bool {
        chrome::dialog(c, &SHOWN.with(Cell::get), title, subtitle, rows, focus);
        true
    }
    fn toast(&self, c: &mut Canvas, message: &str, tone: Tone) -> bool {
        chrome::toast(c, &SHOWN.with(Cell::get), message, tone);
        true
    }

    fn input(
        &self,
        ui: &mut Ui,
        m: &mut AppModel,
        tracks: &[Track],
        action: Action,
    ) -> Option<Effect> {
        if m.screen_off || m.navigation.modal.is_some() || ui.usb_overlay(m) {
            return None;
        }
        let st = &ui.gallery;
        if action == Action::Home {
            // Home is Today, from the top.
            st.pill.set(false);
            st.focus.set(0);
            return None;
        }
        let Some(tab) = root_tab(m) else {
            st.pill.set(false);
            return None;
        };
        Self::tabs_input(ui, m, tracks, tab, action)
    }
}

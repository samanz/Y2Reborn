//! Looks: other presentations of the same player.
//!
//! The listener's look is `Settings::look`, an id; empty, or an id that is
//! not installed, means the standard look. Every frame is drawn with one look
//! current (`with`), and a look may take over any of these, each on its own:
//!
//! - the frame's backdrop, and any screen (`screen` returns `false` to keep
//!   the standard one);
//! - the status bar, dialogs, action sheets and toasts drawn above screens;
//! - the faces text is set in (metrics follow, so layouts measure what is
//!   drawn) and the accent;
//! - input on its own screens, before the standard handling (`input`).
//!
//! A look that moves by itself (a slide, a crossfade) sets `Ui::look_moving`
//! while it draws, so the caller keeps asking for frames.
//!
//! The standard look takes over nothing: with it every frame is the UI v3
//! frame.
use crate::{
    canvas::Canvas,
    components::{SheetRow, Tone},
    text::Face,
    theme::{color, HEIGHT, WIDTH},
    Ui,
};
use reborn_core::{Action, AppModel, Effect, Track};
use std::cell::Cell;

/// Everything a look needs to draw one frame.
pub(crate) struct Frame<'a> {
    pub ui: &'a Ui,
    pub m: &'a AppModel,
    pub tracks: &'a [Track],
    /// The current track's artwork is resident (`Tex::Art`).
    pub has_art: bool,
    /// No sheet, dialog or entry surface is above the screen.
    pub interactive: bool,
}

pub(crate) trait Look: Sync {
    /// Stable identifier stored in the settings.
    fn id(&self) -> &'static str;
    /// The accent for this frame, given the one taken from the cover (0 when
    /// no cover has been seen).
    fn accent(&self, cover: u32) -> u32 {
        if cover == 0 {
            color::ACCENT
        } else {
            cover
        }
    }
    /// The face to set text in where a screen asks for `face`.
    fn face(&self, face: Face) -> Face {
        face
    }
    /// Paint the whole frame's background.
    fn backdrop(&self, c: &mut Canvas, f: &Frame) {
        let _ = f;
        c.rect(0., 0., WIDTH, HEIGHT, color::BG);
    }
    /// Draw the current screen in this look's own layout. `false` keeps the
    /// standard screen.
    fn screen(&self, c: &mut Canvas, f: &Frame) -> bool {
        let _ = (c, f);
        false
    }
    /// The top strip of standard screens. `false` keeps the standard one.
    fn status_bar(&self, c: &mut Canvas, ui: &Ui, m: &AppModel, title: &str, back: bool) -> bool {
        let _ = (c, ui, m, title, back);
        false
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
        let _ = (c, title, body, rows, focus, danger);
        false
    }
    fn action_sheet(
        &self,
        c: &mut Canvas,
        title: &str,
        subtitle: &str,
        rows: &[SheetRow],
        focus: usize,
    ) -> bool {
        let _ = (c, title, subtitle, rows, focus);
        false
    }
    fn toast(&self, c: &mut Canvas, message: &str, tone: Tone) -> bool {
        let _ = (c, message, tone);
        false
    }
    /// Input the look takes before the standard handling (after transport
    /// keys, alerts, pairing and text entry). `None` keeps the standard
    /// handling.
    fn input(
        &self,
        ui: &mut Ui,
        m: &mut AppModel,
        tracks: &[Track],
        action: Action,
    ) -> Option<Effect> {
        let _ = (ui, m, tracks, action);
        None
    }
}

/// The standard UI v3 look.
pub(crate) struct Standard;

impl Look for Standard {
    fn id(&self) -> &'static str {
        ""
    }
}

/// Every installed look, the standard one first.
pub(crate) static LOOKS: &[&dyn Look] = &[&Standard];

/// The look stored as `id`; the standard look for an empty or unknown id.
pub(crate) fn by_id(id: &str) -> &'static dyn Look {
    LOOKS
        .iter()
        .copied()
        .find(|l| l.id() == id)
        .unwrap_or(&Standard)
}

/// The look a model's settings select.
pub(crate) fn of(m: &AppModel) -> &'static dyn Look {
    by_id(&m.settings.look)
}

thread_local! {
    /// The look of the frame being drawn (see `with`).
    static CURRENT: Cell<&'static dyn Look> = const { Cell::new(&Standard) };
}

/// The look of the frame being drawn; the standard look outside a frame.
pub(crate) fn current() -> &'static dyn Look {
    CURRENT.with(Cell::get)
}

/// Run `f` with `look` current, restoring the previous one.
pub(crate) fn with<T>(look: &'static dyn Look, f: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|l| l.replace(look));
    let out = f();
    CURRENT.with(|l| l.set(previous));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_looks_fall_back_to_the_standard_one() {
        assert_eq!(by_id("").id(), "");
        assert_eq!(by_id("no-such-look").id(), "");
        let ids: Vec<_> = LOOKS.iter().map(|l| l.id()).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "ids are distinct");
        assert_eq!(LOOKS[0].id(), "", "the standard look comes first");
    }

    /// A look that sets all text large and shows no toasts.
    struct Large;
    impl Look for Large {
        fn id(&self) -> &'static str {
            "large"
        }
        fn face(&self, _: Face) -> Face {
            Face::S36
        }
        fn toast(&self, _: &mut Canvas, _: &str, _: Tone) -> bool {
            true
        }
    }
    static LARGE: Large = Large;

    #[test]
    fn a_look_replaces_a_component_only_inside_its_frame() {
        let toast = || {
            let mut c = Canvas::new();
            crate::components::toast(&mut c, "Saved", Tone::Info);
            c.finish().len()
        };
        assert!(toast() > 0);
        assert_eq!(with(&LARGE, toast), 0);
        assert!(toast() > 0);
    }

    #[test]
    fn a_frame_look_is_scoped_and_its_faces_measure_what_is_drawn() {
        assert_eq!(current().id(), "");
        let (inner, px) = with(&LARGE, || (current().id(), Face::R11.px()));
        assert_eq!(inner, "large");
        assert_eq!(px, Face::S36.px());
        assert_eq!(current().id(), "");
        assert!(Face::R11.px() < Face::S36.px());
    }

    #[test]
    fn the_standard_look_keeps_the_cover_accent_and_faces() {
        assert_eq!(Standard.accent(0), color::ACCENT);
        assert_eq!(Standard.accent(0x336699FF), 0x336699FF);
        assert_eq!(Standard.face(Face::S15), Face::S15);
    }
}

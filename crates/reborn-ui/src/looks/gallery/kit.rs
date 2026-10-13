//! Gallery's components, shared by every screen it lays out: text set by
//! its top edge, one focus language (a soft accent glow under a lifted
//! element, a 2 px light ring 3 px outside it), pills, mono chips, artwork
//! with one radius family (cards 16, covers 12, thumbs 8, pills round),
//! rows, the mini player and the fades content scrolls under.
use super::motion::Glide;
use super::palette::{Colors, FG, MUTED};
use super::{chrome, now, GUTTER};
use crate::{
    canvas::{Canvas, Icon},
    components,
    look::Frame,
    maskdata::{CORNERS, CORNER_RADII, INVERSE_CORNERS},
    text::{self, Face},
    theme::{alpha, fade, mix, HEIGHT, WIDTH},
};
use reborn_core::PlaybackState;
use reborn_graphics::{Quad, Tex};

/// Covers, thumbnails and cards: one radius family.
pub(crate) const COVER_R: f32 = 12.;
pub(crate) const THUMB_R: f32 = 8.;
pub(crate) const CARD_R: f32 = 16.;
/// Text that scrolls above this line (under the status bar) is not drawn.
pub(crate) const CLEAR: f32 = 30.;
/// The mini player resting at the foot, and its height.
pub(crate) const MINI_H: f32 = 44.;
pub(crate) const MINI_LOW: f32 = HEIGHT - 12. - MINI_H;
/// Hairlines: text at 10 %.
pub(crate) const HAIRLINE: u32 = 0xF2F3F71A;
/// Glass: the raised surface at 80 %, over the room.
const GLASS: u32 = 0x1E1F24CC;
/// Titles are set tight.
pub(crate) const TIGHT: f32 = -0.6;
/// Secondary lines on artwork-heavy screens: text at 75 %.
pub(crate) const SOFT: u32 = 0xF2F3F7BF;

/// Where a page's content ends: above the mini player when one shows.
pub(crate) fn content_bottom(f: &Frame) -> f32 {
    if f.m.current().is_some() {
        MINI_LOW - 8.
    } else {
        HEIGHT - 12.
    }
}

// -------------------------------------------------------------------- text

/// Distance from a line's top to its baseline (19/24 of the size).
pub(crate) fn baseline(face: Face) -> f32 {
    (face.px() * 19. / 24.).round()
}

/// Text with its top at `top`, cut to `width`.
pub(crate) fn top_text(
    c: &mut Canvas,
    x: f32,
    top: f32,
    width: f32,
    s: &str,
    face: Face,
    tint: u32,
) {
    let shown = text::fit(face, s, width.max(0.));
    c.text(x, top + baseline(face), &shown, face, tint);
}

/// Right-aligned text with its top at `top`.
pub(crate) fn top_right(c: &mut Canvas, right: f32, top: f32, s: &str, face: Face, tint: u32) {
    c.text_right(right, top + baseline(face), s, face, tint);
}

/// Small spaced capitals in the mono face ("PLAYING FROM").
pub(crate) fn kicker(c: &mut Canvas, x: f32, top: f32, width: f32, s: &str, tint: u32) {
    let s = text::fit(Face::PM10, &s.to_uppercase(), width);
    c.text_tracked(x, top + baseline(Face::PM10), &s, Face::PM10, tint, 1.8);
}

/// A title set tight, with its top at `top`.
pub(crate) fn display(
    c: &mut Canvas,
    x: f32,
    top: f32,
    width: f32,
    s: &str,
    face: Face,
    tint: u32,
) {
    // `fit` measures untracked; give back what the tight tracking saves.
    let saved = -TIGHT * s.chars().count().saturating_sub(1) as f32;
    let shown = text::fit(face, s, width + saved);
    c.text_tracked(x, top + baseline(face), &shown, face, tint, TIGHT);
}

/// The largest title face that sets `s` on one line within `width`; the
/// 34 px face only has basic Latin.
pub(crate) fn display_face(s: &str, width: f32, faces: &[Face]) -> Face {
    faces
        .iter()
        .copied()
        .filter(|f| *f != Face::SY34 || s.is_ascii())
        .find(|f| text::width_tracked(*f, s, TIGHT) <= width)
        .unwrap_or(faces[faces.len() - 1])
}

/// "12 albums", with thin-space thousands.
pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", crate::screens::count_label(n))
    }
}

// ----------------------------------------------------------------- colour

/// Readable ink on the accent: dark on light accents, white on dark ones.
pub(crate) fn on_accent(accent: u32) -> u32 {
    let ch = |s: u32| ((accent >> s) & 0xFF) as f32 / 255.;
    let luma = 0.2126 * ch(24) + 0.7152 * ch(16) + 0.0722 * ch(8);
    if luma > 0.45 {
        0x111217FF
    } else {
        0xFFFFFFFF
    }
}

/// The focus ring's colour: light, warmed by the accent.
pub(crate) fn ring_tint(colors: &Colors) -> u32 {
    mix(colors.accent, FG, 0.55)
}

// ----------------------------------------------------------------- shapes

/// The corner sprites' slot whose radius is nearest `r`.
fn corner_slot(r: f32) -> usize {
    CORNER_RADII
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (f32::from(*a.1) - r)
                .abs()
                .total_cmp(&(f32::from(*b.1) - r).abs())
        })
        .map_or(0, |(i, _)| i)
}

/// A rounded rectangle fading from `from` at the left edge to `to` at the
/// right (alpha included), from the same non-overlapping pieces as
/// `Canvas::rrect`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rrect_horizontal(
    c: &mut Canvas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    from: u32,
    to: u32,
) {
    let (x, y, w, h) = (x.round(), y.round(), w.round(), h.round());
    if w <= 0. || h <= 0. {
        return;
    }
    let at = |px: f32| mix(from, to, (px - x) / w);
    let mut piece = |q: Quad, x0: f32, x1: f32| {
        if q.w > 0. && q.h > 0. {
            c.draw.push(Quad { color: at(x0), ..q }.horizontal(at(x1)));
        }
    };
    let r = r.min(w / 2.).min(h / 2.).floor();
    if r < 2. {
        return piece(Quad::rect(x, y, w, h, 0), x, x + w);
    }
    let slot = corner_slot(r);
    let r = f32::from(CORNER_RADII[slot]);
    let corner =
        |i: usize, cx: f32, cy: f32| Quad::masked(Tex::Mask, CORNERS[slot][i], cx, cy, r, r, 0);
    let (mid, right) = (x + r, x + w - r);
    piece(Quad::rect(mid, y, w - 2. * r, r, 0), mid, right);
    piece(Quad::rect(x, y + r, w, h - 2. * r, 0), x, x + w);
    piece(Quad::rect(mid, y + h - r, w - 2. * r, r, 0), mid, right);
    piece(corner(0, x, y), x, mid);
    piece(corner(1, right, y), right, x + w);
    piece(corner(2, x, y + h - r), x, mid);
    piece(corner(3, right, y + h - r), right, x + w);
}

/// A glassy raised surface: translucent fill with a hairline border.
pub(crate) fn glass(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32) {
    c.rrect(x, y, w, h, r, GLASS);
    c.rrect_outline(x, y, w, h, r, 1., HAIRLINE);
}

/// Three little level bars: this one is playing.
pub(crate) fn eq_bars(c: &mut Canvas, x: f32, bottom: f32, tint: u32) {
    for (i, h) in [8., 12., 6.].into_iter().enumerate() {
        c.rrect(x + i as f32 * 4., bottom - h, 2., h, 1., tint);
    }
}

/// A pill switch for on/off rows, its right edge at `right`.
pub(crate) fn switch(c: &mut Canvas, right: f32, cy: f32, on: bool, accent: u32) {
    let (w, h) = (34., 20.);
    let x = right - w;
    if on {
        c.rrect(x, cy - h / 2., w, h, h / 2., accent);
        c.disc(x + w - h / 2., cy, 7., on_accent(accent));
    } else {
        c.rrect(x, cy - h / 2., w, h, h / 2., 0x30323CFF);
        c.disc(x + h / 2., cy, 7., MUTED);
    }
}

// ------------------------------------------------------------------ focus

/// The soft accent light under a focused element (drawn before it).
pub(crate) fn focus_glow(
    c: &mut Canvas,
    colors: &Colors,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    arrive: f32,
) {
    c.glow(
        x + w / 2.,
        y + h / 2. + 6.,
        w / 2. + 18.,
        h / 2. + 20.,
        fade(alpha(colors.accent, 0x5C), arrive),
    );
}

/// The focus ring 3 px outside an element (drawn after it); marks the focus.
#[allow(clippy::too_many_arguments)]
pub(crate) fn focus_ring(
    c: &mut Canvas,
    colors: &Colors,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    arrive: f32,
) {
    let at = c.draw.len();
    c.rrect_outline(
        x - 3.,
        y - 3.,
        w + 6.,
        h + 6.,
        r + 3.,
        2.,
        fade(ring_tint(colors), 0.35 + 0.65 * arrive),
    );
    c.mark_focus(at);
}

/// The fill of a focused row's band.
pub(crate) fn band(colors: &Colors) -> u32 {
    alpha(mix(colors.surface, FG, 0.08), 0xF0)
}

/// A focused row: a raised band with the ring at its edge; marks the focus.
#[allow(clippy::too_many_arguments)]
pub(crate) fn row_focus(
    c: &mut Canvas,
    colors: &Colors,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    arrive: f32,
) {
    c.glow(
        x + w / 2.,
        y + h / 2. + 4.,
        w / 2. + 10.,
        h / 2. + 14.,
        fade(alpha(colors.accent, 0x3A), arrive),
    );
    c.rrect(x, y, w, h, COVER_R, band(colors));
    let at = c.draw.len();
    c.rrect_outline(
        x,
        y,
        w,
        h,
        COVER_R,
        2.,
        fade(ring_tint(colors), 0.3 + 0.55 * arrive),
    );
    c.mark_focus(at);
}

// ------------------------------------------------------------------ pills

/// How a pill is filled.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PillStyle {
    /// The main action: the accent, with ink that reads on it.
    Filled,
    /// Secondary actions: a hairline capsule.
    Outlined,
}

pub(crate) const PILL_H: f32 = 32.;

/// The width a pill takes.
pub(crate) fn pill_width(label: &str, icon: Option<Icon>) -> f32 {
    text::width(Face::DMB14, label) + 32. + if icon.is_some() { 20. } else { 0. }
}

/// A pill button at (x, y); returns its width.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pill(
    c: &mut Canvas,
    colors: &Colors,
    x: f32,
    y: f32,
    label: &str,
    icon: Option<Icon>,
    style: PillStyle,
    focused: bool,
    arrive: f32,
) -> f32 {
    let w = pill_width(label, icon);
    let h = PILL_H;
    if focused {
        focus_glow(c, colors, x, y, w, h, arrive);
    }
    let ink = match style {
        PillStyle::Filled => {
            c.rrect(x, y, w, h, h / 2., colors.accent);
            on_accent(colors.accent)
        }
        PillStyle::Outlined => {
            c.rrect(
                x,
                y,
                w,
                h,
                h / 2.,
                alpha(mix(colors.surface, FG, 0.05), 0xC0),
            );
            c.rrect_outline(x, y, w, h, h / 2., 1., 0xF2F3F740);
            FG
        }
    };
    let mut tx = x + 16.;
    if let Some(icon) = icon {
        // Play's triangle sits a pixel right of its box's centre.
        let nudge = if icon == Icon::Play { 1. } else { 0. };
        c.icon(icon, tx + 7. + nudge, y + h / 2., 14., ink);
        tx += 20.;
    }
    c.text(
        tx,
        y + h / 2. + Face::DMB14.mid_offset(),
        label,
        Face::DMB14,
        ink,
    );
    if focused {
        focus_ring(c, colors, x, y, w, h, h / 2., arrive);
    }
    w
}

/// A mono fact chip ("8 SONGS"); returns its width.
pub(crate) fn chip(c: &mut Canvas, x: f32, y: f32, label: &str) -> f32 {
    let label = label.to_uppercase();
    let w = text::width_tracked(Face::PM10, &label, 1.2) + 18.;
    c.rrect(x, y, w, 20., 10., 0xF2F3F70A);
    c.rrect_outline(x, y, w, 20., 10., 1., 0xF2F3F726);
    c.text_tracked(
        x + 9.,
        y + 10. + Face::PM10.mid_offset(),
        &label,
        Face::PM10,
        MUTED,
        1.2,
    );
    w
}

/// The width a chip takes.
pub(crate) fn chip_width(label: &str) -> f32 {
    text::width_tracked(Face::PM10, &label.to_uppercase(), 1.2) + 18.
}

// --------------------------------------------------------------- artwork

/// A soft glow laid over the room: centre, radii and colour.
pub(crate) type Wash = (f32, f32, f32, f32, u32);

/// Round a picture's corners off with what lies behind each corner: the
/// room where it sits (with `wash` over it), or `behind` when the picture
/// is on a surface.
pub(crate) fn round_corners(
    c: &mut Canvas,
    colors: &Colors,
    (x, y, size): (f32, f32, f32),
    r: f32,
    behind: Option<u32>,
    wash: Option<Wash>,
) {
    let want = r.min(size / 2.).round().max(1.);
    let slot = corner_slot(want);
    // Beyond the largest sprite (a round portrait) that one is drawn larger.
    let largest = f32::from(CORNER_RADII[CORNER_RADII.len() - 1]);
    let r = if want > largest {
        want
    } else {
        f32::from(CORNER_RADII[slot])
    };
    let (x, y, size) = (x.round(), y.round(), size.round());
    // What lies behind the picture at (px, py).
    let tint = |px: f32, py: f32| {
        let mut tint = behind.unwrap_or_else(|| colors.bg_xy(px, py));
        if let (None, Some((gx, gy, rx, ry, color))) = (behind, wash) {
            let d = (((px - gx) / rx).powi(2) + ((py - gy) / ry).powi(2)).sqrt();
            let t = (1f32 - d).max(0.);
            let a = (color & 0xFF) as f32 / 255. * t * t * (3. - 2. * t);
            tint = mix(tint, color | 0xFF, a);
        }
        tint
    };
    let far = x + size - r;
    let low = y + size - r;
    // Each corner's square, and the centre of the circle it rounds to.
    let corners = [
        ((x, y), (x + r, y + r)),
        ((far, y), (far, y + r)),
        ((x, low), (x + r, low)),
        ((far, low), (far, low)),
    ];
    for (i, ((cx, cy), centre)) in corners.into_iter().enumerate() {
        let src = INVERSE_CORNERS[slot][i];
        if r <= largest {
            // Where the corner shows: just inside the picture's corner.
            let tip = (
                if i % 2 == 0 { cx } else { cx + r },
                if i < 2 { cy } else { cy + r },
            );
            let at = (
                tip.0 + (centre.0 - tip.0) * 0.2,
                tip.1 + (centre.1 - tip.1) * 0.2,
            );
            c.draw
                .push(Quad::masked(Tex::Mask, src, cx, cy, r, r, tint(at.0, at.1)));
            continue;
        }
        // A large corner in tiles, each in the colour behind it, so the room's
        // fall-off shows no seams; tiles inside the circle are left out.
        const TILES: u16 = 4;
        let (step, part) = (r / f32::from(TILES), src[2] / TILES);
        for ty in 0..TILES {
            for tx in 0..TILES {
                let (qx, qy) = (cx + f32::from(tx) * step, cy + f32::from(ty) * step);
                let inside = [(0., 0.), (step, 0.), (0., step), (step, step)]
                    .iter()
                    .all(|(dx, dy)| (qx + dx - centre.0).hypot(qy + dy - centre.1) <= r - 0.5);
                if inside {
                    continue;
                }
                c.draw.push(Quad::masked(
                    Tex::Mask,
                    [src[0] + tx * part, src[1] + ty * part, part, part],
                    qx,
                    qy,
                    step,
                    step,
                    tint(qx + step / 2., qy + step / 2.),
                ));
            }
        }
    }
}

/// A library song's cover with rounded corners, over the room (or over the
/// surface `behind`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn cover(
    c: &mut Canvas,
    f: &Frame,
    colors: &Colors,
    track: Option<usize>,
    x: f32,
    y: f32,
    size: f32,
    r: f32,
    behind: Option<u32>,
) {
    let key = track.and_then(|i| f.tracks.get(i).map(|t| (i, t)));
    components::cover_rounded(c, f.ui, key, x, y, size, r);
    round_corners(c, colors, (x, y, size), r, behind, None);
}

// ---------------------------------------------------------------- scroll

/// The scroll that keeps [top, bottom) (page coordinates) inside the view
/// [view_top, view_bottom), moving as little as it can from `from`.
pub(crate) fn aim(from: f32, top: f32, bottom: f32, view_top: f32, view_bottom: f32) -> f32 {
    let mut aim = from;
    if bottom - aim > view_bottom {
        aim = bottom - view_bottom;
    }
    if top - aim < view_top {
        aim = top - view_top;
    }
    aim.max(0.).round()
}

/// A scroll that rests on the start of a part of the page (`starts`, page
/// coordinates of where each part begins, less the view's top), so none is
/// cut by the status bar: the start at or above `to` when the focus still
/// fits, else the next one.
pub(crate) fn snap(to: f32, starts: &[f32], focus_bottom: f32, view_bottom: f32) -> f32 {
    let below = starts
        .iter()
        .copied()
        .filter(|s| *s <= to)
        .fold(0f32, f32::max);
    if focus_bottom - below <= view_bottom {
        below
    } else {
        starts.iter().copied().find(|s| *s >= to).unwrap_or(to)
    }
}

/// A page's scroll gliding to `to`.
pub(crate) fn glide(glide: &Glide, f: &Frame, to: f32) -> f32 {
    let st = &f.ui.gallery;
    glide.toward(to, now(f.ui), 360, &st.clock)
}

// ------------------------------------------------------------ the frame

/// The status bar over a soft fade, so pages scroll under it.
pub(crate) fn status(c: &mut Canvas, f: &Frame, colors: &Colors) {
    c.draw.push(
        Quad::rect(0., 0., WIDTH, CLEAR, alpha(colors.bg, 0xF4)).vertical(alpha(colors.bg, 0xC0)),
    );
    c.draw.push(
        Quad::rect(0., CLEAR, WIDTH, 12., alpha(colors.bg, 0xC0)).vertical(alpha(colors.bg, 0)),
    );
    chrome::bar(c, f.ui, f.m, colors);
}

/// The fade content scrolls into above the mini player (or the bottom).
pub(crate) fn bottom_veil(c: &mut Canvas, colors: &Colors, at: f32) {
    c.draw.push(
        Quad::rect(0., at - 40., WIDTH, 40., alpha(colors.bg_deep, 0)).vertical(colors.bg_deep),
    );
    c.rect(0., at, WIDTH, HEIGHT - at, colors.bg_deep);
}

/// The mini player: the cover, the song in bold over the artist, a round
/// accent play button and a thin progress line along the bottom.
pub(crate) fn mini_player(c: &mut Canvas, f: &Frame, colors: &Colors, y: f32) {
    let Some(t) = f.m.current() else {
        return;
    };
    let (x, w, h) = (GUTTER, WIDTH - 2. * GUTTER, MINI_H);
    let fill = mix(colors.surface, FG, 0.06);
    c.glow(x + w / 2., y + h / 2. + 6., w / 2. + 8., 30., 0x00000060);
    c.rrect(x, y, w, h, CARD_R, fill);
    c.rrect_outline(x, y, w, h, CARD_R, 1., HAIRLINE);
    let a = 32.;
    let (ax, ay) = (x + 6., y + 6.);
    // The thumbnail when it is resident, else the playing artwork.
    if f.has_art && !f.ui.thumb_resident(components::art_key(t)) {
        c.draw.push(Quad {
            tex: Tex::Art,
            ..Quad::rect(ax, ay, a, a, 0xFFFFFFFF)
        });
    } else {
        let key = f
            .tracks
            .iter()
            .position(|x| x.id == t.id && x.source_id == t.source_id)
            .map(|i| (i, &f.tracks[i]));
        components::cover_rounded(c, f.ui, key, ax, ay, a, THUMB_R);
    }
    c.round_off(ax, ay, a, THUMB_R, fill);
    let tx = ax + a + 12.;
    let tw = x + w - 56. - tx;
    top_text(c, tx, y + 7., tw, crate::track_title(t), Face::DMB14, FG);
    top_text(
        c,
        tx,
        y + 25.,
        tw,
        crate::display_or_unknown(&t.artist),
        Face::DM12,
        MUTED,
    );
    let (cx, cy) = (x + w - 6. - 16., y + h / 2.);
    c.disc(cx, cy, 16., colors.accent);
    let playing = f.m.playback == PlaybackState::Playing;
    c.icon(
        if playing { Icon::Pause } else { Icon::Play },
        cx + if playing { 0. } else { 1. },
        cy,
        14.,
        on_accent(colors.accent),
    );
    let value = components::progress(f.m.position_ms, t.duration_ms);
    let (px, pw) = (x + 16., w - 32.);
    c.rect(px, y + h - 3., pw, 2., 0xF2F3F714);
    c.rect(px, y + h - 3., (pw * value).round(), 2., colors.accent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_keeps_the_focus_in_view_and_rests_on_a_part() {
        assert_eq!(aim(0., 100., 140., 44., 300.), 0.);
        assert_eq!(aim(0., 300., 340., 44., 300.), 40.);
        assert_eq!(aim(200., 100., 140., 44., 300.), 56.);
        // A part starting at 80 fits with the focus: rest there.
        assert_eq!(snap(90., &[0., 80., 160.], 300., 300.), 80.);
        // It would push the focus out: the next part.
        assert_eq!(snap(90., &[0., 80., 160.], 400., 300.), 160.);
    }

    #[test]
    fn titles_take_the_largest_face_that_fits() {
        let faces = [Face::SY34, Face::SY26, Face::SY20];
        assert_eq!(display_face("Low Tide", 400., &faces), Face::SY34);
        assert_eq!(display_face("Lumière", 400., &faces), Face::SY26);
        let long = "A Landscape in Sound and Much More Besides";
        assert_eq!(display_face(long, 200., &faces), Face::SY20);
    }
}

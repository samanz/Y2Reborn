//! Albums in Gallery: a wall of covers five across, the focused one lifted
//! on its glow and named in the header above, scrolling under the header
//! after the focus. The wheel walks the albums in reading order and Select
//! opens one, as in the standard grid.
use super::kit::{self, display, display_face, kicker, top_text, SOFT};
use super::palette::{Colors, FG};
use super::GUTTER;
use crate::{
    canvas::Canvas,
    components,
    look::Frame,
    screens::screen_title,
    text::Face,
    theme::{alpha, fade, HEIGHT, WIDTH},
};
use reborn_graphics::Quad;

const COLUMNS: usize = 5;
const CELL: f32 = 80.;
const GAP: f32 = 10.;
const PITCH: f32 = CELL + GAP;
/// Where the wall starts under the header.
const TOP: f32 = 112.;
/// The focused cover grows by this much as it lifts.
const LIFT: f32 = 12.;

/// The wall's scroll for the focused `row` of `rows`: that row about a
/// third of the way down the area under the header, never past either end.
fn scroll_for(row: usize, rows: usize) -> f32 {
    let area = HEIGHT - TOP;
    let content = rows as f32 * PITCH - GAP + 12.;
    let max = (content - area).max(0.);
    (row as f32 * PITCH + CELL / 2. - area * 0.36).clamp(0., max)
}

pub(crate) fn draw(c: &mut Canvas, f: &Frame, colors: &Colors) {
    let (ui, m, tracks) = (f.ui, f.m, f.tracks);
    let count = ui.row_count(m, tracks);
    let focus = m.navigation.focus.min(count.saturating_sub(1));
    let rows_total = count.div_ceil(COLUMNS);
    let st = &ui.gallery;
    if st.fresh_page(m) {
        st.sheet
            .toward(scroll_for(focus / COLUMNS, rows_total), None, 0, &st.clock);
    }
    let scroll = kit::glide(&st.sheet, f, scroll_for(focus / COLUMNS, rows_total)).round();
    let x0 = ((WIDTH - (COLUMNS as f32 * PITCH - GAP)) / 2.).floor();
    let place = |i: usize| {
        let (col, line) = ((i % COLUMNS) as f32, (i / COLUMNS) as f32);
        (x0 + col * PITCH, TOP + line * PITCH - scroll)
    };
    // Rows on screen, the one passing under the header included.
    let first_row = ((scroll - (TOP - 40.)) / PITCH).floor().max(0.) as usize;
    let last_row = (((HEIGHT - TOP + scroll) / PITCH).ceil().max(0.) as usize).min(rows_total);
    let first = first_row * COLUMNS;
    let shown = (last_row.saturating_sub(first_row) * COLUMNS).min(count.saturating_sub(first));
    let rows = ui.visible_rows(m, tracks, first, shown);
    let lit = f.interactive && !st.pill.get();
    for (n, row) in rows.iter().enumerate() {
        let i = first + n;
        if lit && i == focus {
            continue;
        }
        let (x, y) = place(i);
        kit::cover(c, f, colors, row.art, x, y, CELL, 10., None);
        if lit {
            // The rest of the wall steps back behind the focused cover.
            c.rrect(x, y, CELL, CELL, 10., 0x00000040);
        }
    }
    // The header reads over the covers passing under it.
    c.rect(0., 0., WIDTH, TOP - 16., alpha(colors.bg, 0xF2));
    c.draw.push(
        Quad::rect(0., TOP - 16., WIDTH, 16., alpha(colors.bg, 0xF2)).vertical(alpha(colors.bg, 0)),
    );
    // The focused cover last, lifted over its neighbours.
    let focused = ui.row_at(m, tracks, focus);
    if let Some(row) = focused.as_ref().filter(|_| lit) {
        let (x, y) = place(focus);
        let size = CELL + LIFT;
        let x = (x - LIFT / 2.).clamp(4., WIDTH - size - 4.);
        let y = (y - LIFT / 2.).max(TOP - 8.);
        kit::focus_glow(c, colors, x, y, size, size, 1.);
        components::cover_shadow(c, x, y, size);
        kit::cover(c, f, colors, row.art, x, y, size, 12., None);
        kit::focus_ring(c, colors, x, y, size, size, 12., 1.);
    }
    // The header: the place among the albums, then the album and its artist.
    let w = WIDTH - 2. * GUTTER;
    let place_line = if count > 0 {
        format!("{}  ·  {} of {}", screen_title(m), focus + 1, count)
    } else {
        screen_title(m)
    };
    kicker(c, GUTTER, 40., w, &place_line, colors.accent);
    if let Some(row) = &focused {
        let face = display_face(&row.label, w, &[Face::SY26, Face::SY23, Face::SY20]);
        display(
            c,
            GUTTER,
            54. + (26. - face.px()) / 2.,
            w,
            &row.label,
            face,
            FG,
        );
        top_text(c, GUTTER, 86., w, &row.secondary, Face::DM13, SOFT);
    }
    // A thin pill for the place in the wall.
    let area = HEIGHT - TOP;
    let content = rows_total as f32 * PITCH - GAP + 12.;
    if content > area {
        let height = area - 16.;
        let thumb = (height * area / content).clamp(18., height);
        let y = TOP + 8. + (height - thumb) * scroll / (content - area);
        c.rrect(
            WIDTH - 7.,
            y.round(),
            3.,
            thumb.round(),
            1.5,
            fade(FG, 0.33),
        );
    }
    kit::status(c, f, colors);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wall_keeps_the_focused_row_in_view() {
        assert_eq!(scroll_for(0, 20), 0.);
        assert_eq!(scroll_for(1, 2), 0., "a short library does not scroll");
        for row in [3, 8, 19] {
            let y = TOP + row as f32 * PITCH - scroll_for(row, 20);
            assert!(y >= TOP - 1. && y + CELL <= HEIGHT, "{row}: {y}");
        }
        // Five covers and their gaps span the width within the gutters.
        let span = COLUMNS as f32 * PITCH - GAP;
        assert!(span <= WIDTH - 2. * GUTTER, "{span}");
    }
}

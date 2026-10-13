//! The tab glyphs, drawn in code on a 24-unit grid with 2-unit strokes and
//! round caps. Strokes are chains of soft discs (the disc mask, scaled), so
//! they come out smooth at any angle with only the canvas's quads.
use crate::{canvas::Canvas, maskdata};
use reborn_graphics::{Quad, Tex};

/// The tab glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Compass,
    Grid,
    Person,
    Note,
}

/// A soft round dot of diameter `d` centred on (x, y).
fn dot(c: &mut Canvas, x: f32, y: f32, d: f32, color: u32) {
    c.draw.push(Quad::masked(
        Tex::Mask,
        maskdata::DISC,
        x - d / 2.,
        y - d / 2.,
        d,
        d,
        color,
    ));
}

/// A straight stroke of width `w` with round caps.
fn line(c: &mut Canvas, a: (f32, f32), b: (f32, f32), w: f32, color: u32) {
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let n = (len / (w * 0.3)).ceil().max(1.) as usize;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        dot(c, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, w, color);
    }
}

/// An elliptical arc stroke from `from` to `to` degrees (0 is +x, angles
/// turn clockwise on screen).
#[allow(clippy::too_many_arguments)]
fn arc(c: &mut Canvas, cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, w: f32, color: u32) {
    let span = (to - from).abs().to_radians() * rx.max(ry);
    let n = (span / (w * 0.3)).ceil().max(2.) as usize;
    for i in 0..=n {
        let a = (from + (to - from) * i as f32 / n as f32).to_radians();
        dot(c, cx + rx * a.cos(), cy + ry * a.sin(), w, color);
    }
}

/// A circle of radius `r` stroked `w` wide (a round portrait's focus ring).
pub(crate) fn circle(c: &mut Canvas, cx: f32, cy: f32, r: f32, w: f32, color: u32) {
    arc(c, cx, cy, r, r, 0., 360., w, color);
}

/// `glyph` in a `size` box centred on (cx, cy).
pub(crate) fn draw(c: &mut Canvas, glyph: Glyph, cx: f32, cy: f32, size: f32, color: u32) {
    let u = size / 24.;
    let p = |x: f32, y: f32| (cx + (x - 12.) * u, cy + (y - 12.) * u);
    let w = 2. * u;
    match glyph {
        Glyph::Compass => {
            let (ox, oy) = p(12., 12.);
            arc(c, ox, oy, 8.5 * u, 8.5 * u, 0., 360., w, color);
            // The needle: a lozenge from the north-east tip to the
            // south-west one, filled with discs that swell to the middle.
            let (a, b) = (p(15.5, 8.5), p(8.5, 15.5));
            let n = 16;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let r = (2.6 * u * (1. - (2. * t - 1.).abs())).max(0.5 * u);
                dot(
                    c,
                    a.0 + (b.0 - a.0) * t,
                    a.1 + (b.1 - a.1) * t,
                    2. * r,
                    color,
                );
            }
        }
        Glyph::Grid => {
            // A three by three wall of tiles, on whole pixels.
            let tile = (5. * u).round().max(2.);
            let gap = u.round().max(1.);
            let span = 3. * tile + 2. * gap;
            let x0 = (cx - span / 2.).round();
            let y0 = (cy - span / 2.).round();
            for i in 0..3 {
                for j in 0..3 {
                    c.rrect(
                        x0 + i as f32 * (tile + gap),
                        y0 + j as f32 * (tile + gap),
                        tile,
                        tile,
                        1.2 * u,
                        color,
                    );
                }
            }
        }
        Glyph::Person => {
            // A head over the arc of the shoulders.
            let (hx, hy) = p(12., 7.5);
            arc(c, hx, hy, 4.2 * u, 4.2 * u, 0., 360., w, color);
            let (sx, sy) = p(12., 21.);
            arc(c, sx, sy, 7.5 * u, 7. * u, 180., 360., w, color);
        }
        Glyph::Note => {
            // A filled head, its stem and a short flag.
            let (nx, ny) = p(9.2, 17.2);
            for (dx, dy, d) in [(0., 0., 7.2), (-1., 0.4, 5.6), (1., -0.4, 5.6)] {
                dot(c, nx + dx * u, ny + dy * u, d * u, color);
            }
            line(c, p(12.6, 17.), p(12.6, 3.5), w, color);
            line(c, p(12.6, 3.5), p(18.5, 6.8), w, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_stay_inside_their_box() {
        for glyph in [Glyph::Compass, Glyph::Grid, Glyph::Person, Glyph::Note] {
            let mut c = Canvas::new();
            draw(&mut c, glyph, 100., 50., 18., 0xFFFFFFFF);
            assert!(!c.draw.is_empty());
            for q in &c.draw {
                assert!(q.x >= 90. && q.x + q.w <= 110., "{glyph:?} {q:?}");
                assert!(q.y >= 40. && q.y + q.h <= 60., "{glyph:?} {q:?}");
            }
        }
    }
}

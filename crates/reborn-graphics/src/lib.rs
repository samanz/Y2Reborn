mod native;
pub use native::{Atlases, Renderer};
use serde::{Deserialize, Serialize};

/// Which texture a quad samples. Glyph pages, the shape/icon mask page and the
/// thumbnail atlas are addressed with `Quad::src`, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Tex {
    /// A flat colour.
    #[default]
    Solid,
    /// Glyph page 0 Regular, 1 SemiBold, 2 the fallback page, 3.. the
    /// `Atlases::extra_pages`; coverage masks tinted by `color`.
    Font(u8),
    /// Shapes, gradients and icons; coverage masks tinted by `color`.
    Mask,
    /// The current track's artwork (160x160).
    Art,
    /// The artwork of the album, artist or song being browsed (160x160).
    CollectionArt,
    /// A library thumbnail (96x96) identified by an opaque key; the renderer
    /// maps it to an atlas cell and skips it until its pixels are resident.
    Thumb(u64),
    /// A small, blurred copy of the current artwork (`BACKDROP_SIZE` square,
    /// see [`blur_backdrop`]), stretched with linear filtering as a backdrop.
    Backdrop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: u32,
    #[serde(default)]
    pub tex: Tex,
    /// Source rectangle `[x, y, w, h]` in texels (glyph, mask textures only).
    #[serde(default)]
    pub src: [u16; 4],
    /// Presentation metadata used by deterministic UI validation. Native
    /// rendering intentionally ignores this field.
    pub focus_target: bool,
    /// Draw into an arbitrary quadrilateral instead of the `x/y/w/h` box,
    /// with perspective-correct texturing and per-corner tints (Cover Flow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warp: Option<Warp>,
}
/// Corners in screen pixels, clockwise from top-left: TL, TR, BR, BL. The
/// texture's top-left maps to TL. `colors` tint each corner (RGBA), which
/// fades a reflection without a separate gradient texture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warp {
    pub corners: [[f32; 2]; 4],
    pub colors: [u32; 4],
}
impl Quad {
    pub fn rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            color,
            tex: Tex::Solid,
            src: [0; 4],
            focus_target: false,
            warp: None,
        }
    }
    /// A tinted region of a coverage-mask texture.
    pub fn masked(tex: Tex, src: [u16; 4], x: f32, y: f32, w: f32, h: f32, color: u32) -> Self {
        Self {
            tex,
            src,
            ..Self::rect(x, y, w, h, color)
        }
    }
    /// The box shaded from `color` at the top to `to` at the bottom: a warp
    /// over the box itself with per-corner tints.
    pub fn vertical(self, to: u32) -> Self {
        let c = self.color;
        self.shaded([c, c, to, to])
    }
    /// The box shaded from `color` at the left to `to` at the right.
    pub fn horizontal(self, to: u32) -> Self {
        let c = self.color;
        self.shaded([c, to, to, c])
    }
    fn shaded(mut self, colors: [u32; 4]) -> Self {
        let (x0, y0, x1, y1) = (self.x, self.y, self.x + self.w, self.y + self.h);
        self.warp = Some(Warp {
            corners: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            colors,
        });
        self
    }
}

/// Side of the backdrop texture.
pub const BACKDROP_SIZE: usize = 40;

/// A soft, blurred copy of 160x160 RGBA artwork for [`Tex::Backdrop`]: a 4x4
/// box downsample to 40x40 and two 7x7 box-blur passes, opaque. Runs once per
/// cover, never per frame.
pub fn blur_backdrop(rgba: &[u8]) -> Option<Vec<u8>> {
    const S: usize = BACKDROP_SIZE;
    const R: isize = 3;
    if rgba.len() != 160 * 160 * 4 {
        return None;
    }
    let mut img = vec![[0f32; 3]; S * S];
    for (i, px) in img.iter_mut().enumerate() {
        let (x, y) = (i % S, i / S);
        for dy in 0..4 {
            for dx in 0..4 {
                let o = ((y * 4 + dy) * 160 + x * 4 + dx) * 4;
                for (k, v) in px.iter_mut().enumerate() {
                    *v += f32::from(rgba[o + k]) / 16.;
                }
            }
        }
    }
    for _ in 0..2 {
        // Separable: rows, then columns, clamping at the edges.
        for pass in 0..2 {
            let src = img.clone();
            for (i, out) in img.iter_mut().enumerate() {
                let (x, y) = ((i % S) as isize, (i / S) as isize);
                let mut acc = [0f32; 3];
                for d in -R..=R {
                    let (xx, yy) = if pass == 0 {
                        ((x + d).clamp(0, S as isize - 1), y)
                    } else {
                        (x, (y + d).clamp(0, S as isize - 1))
                    };
                    for (a, v) in acc.iter_mut().zip(src[yy as usize * S + xx as usize]) {
                        *a += v;
                    }
                }
                *out = acc.map(|v| v / (2 * R + 1) as f32);
            }
        }
    }
    Some(
        img.into_iter()
            .flat_map(|p| {
                let [r, g, b] = p.map(|v| v.round().clamp(0., 255.) as u8);
                [r, g, b, 255]
            })
            .collect(),
    )
}

/// Texture geometry shared with `native/graphics.c`.
pub const FONT_PAGE_WIDTH: usize = 1024;
/// Glyph pages beyond the three built-in ones the renderer accepts.
pub const EXTRA_FONT_PAGES: usize = 8;
pub const THUMB_SIZE: usize = 96;
/// A thumbnail cell is the image with a replicated one-pixel gutter.
pub const THUMB_CELL: usize = THUMB_SIZE + 2;
pub const THUMB_COLUMNS: usize = 1024 / THUMB_CELL;
pub const THUMB_ROWS: usize = 512 / THUMB_CELL;
pub const THUMB_SLOTS: usize = THUMB_COLUMNS * THUMB_ROWS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaded_boxes_are_warps_over_themselves() {
        let v = Quad::rect(10., 20., 30., 40., 1).vertical(2);
        let w = v.warp.unwrap();
        assert_eq!(w.corners, [[10., 20.], [40., 20.], [40., 60.], [10., 60.]]);
        assert_eq!(w.colors, [1, 1, 2, 2]);
        assert_eq!((v.x, v.y, v.w, v.h), (10., 20., 30., 40.));
        let h = Quad::rect(0., 0., 1., 1., 1).horizontal(2);
        assert_eq!(h.warp.unwrap().colors, [1, 2, 2, 1]);
    }

    #[test]
    fn the_backdrop_is_small_opaque_and_smooth() {
        let mut art = vec![0u8; 160 * 160 * 4];
        for (i, px) in art.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let on = (i % 160 / 20 + i / 160 / 20) % 2 == 0;
            px.copy_from_slice(&[if on { 255 } else { 0 }, 40, 90, 255]);
        }
        let out = blur_backdrop(&art).unwrap();
        assert_eq!(out.len(), BACKDROP_SIZE * BACKDROP_SIZE * 4);
        assert!(out
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == 255 && p[1] == 40));
        // A hard checkerboard becomes gentle: neighbours differ little.
        let red: Vec<i32> = out
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| i32::from(p[0]))
            .collect();
        let steepest = red
            .as_chunks::<BACKDROP_SIZE>()
            .0
            .iter()
            .flat_map(|row| row.windows(2).map(|w| (w[0] - w[1]).abs()))
            .max()
            .unwrap();
        assert!(steepest < 64, "steepest step {steepest} of 255");
        assert!(blur_backdrop(&art[4..]).is_none());
    }
}

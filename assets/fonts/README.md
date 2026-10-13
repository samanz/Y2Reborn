# Reborn UI v3 type and shapes

Open Sans (SIL Open Font License 1.1, `OpenSans-OFL.txt`) is rendered once, at
build time, into coverage-mask pages — one byte per texel, sampled by the
renderer as luminance-alpha and tinted by the draw colour:

| Page | File | Contents |
| --- | --- | --- |
| 0 | `reborn-regular.a8` | Regular 11/13/15/17 px, Light 23 px, Light 44 px wordmark |
| 1 | `reborn-semibold.a8` | SemiBold 10/13/15/17/20/25/28/36 px |
| 2 | `reborn-fallback.a8` | Regular 24 px, scaled, for every character an exact set lacks |

Every `text::Face` is one weight at one pixel size, so glyphs are drawn 1:1 on
the pixel grid rather than scaled from one bitmap. The exact sets cover basic
and extended Latin (U+0020–017F), common punctuation and the euro sign; the
fallback page adds the rest of Latin Extended-B, Greek, Cyrillic, arrows and
U+FFFD, which stands in for anything else (CJK, emoji and shaping are not
supported). `provenance.json` records the font file hashes, set sizes and page
heights. `crates/reborn-ui/src/fontdata.rs` holds the matching metrics.

Regenerate everything (fonts and `assets/icons`) with
`tools/assets/build_ui_assets.py`. Build inputs: Pillow and the Open Sans TTFs;
no font file is installed on the player.

## The Gallery look's typefaces

The Gallery look sets its text in three more typefaces, each under the SIL Open
Font License 1.1, rendered the same way into extra pages (`Tex::Font(3..)`),
which the renderer only creates once a frame uses them:

| Page | File | Contents |
| --- | --- | --- |
| 3 | `reborn-page3.a8` | DM Sans 500 12/13/14/15/16 px, DM Sans 700 14 px |
| 4 | `reborn-page4.a8` | DM Sans 700 17 px, Syne 800 20/23 px |
| 5 | `reborn-page5.a8` | Syne 800 26/34 px, IBM Plex Mono Medium 10/11/12 px |

DM Sans (`DMSans-OFL.txt`) is the text face, Syne (`Syne-OFL.txt`) the titles
and IBM Plex Mono (`IBMPlexMono-OFL.txt`) the small spaced captions. The DM Sans
sets cover the same characters as the Open Sans text sets; Syne and the small
mono sizes cover basic Latin and Latin-1 (the 34 px titles basic Latin only).
`gallery-provenance.json` records the font files' hashes and axes, the sets and
the page heights; `crates/reborn-ui/src/galleryfonts.rs` holds the metrics.
Regenerate them with `tools/assets/build_gallery_fonts.py` from the TTFs in the
Google Fonts repository (`ofl/dmsans`, `ofl/syne`, `ofl/ibmplexmono`).

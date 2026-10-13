#!/usr/bin/env python3
"""Build the exact-size glyph sets the Gallery look sets its text in.

Same scheme as build_ui_assets.py: every (typeface, weight, pixel size) is
rendered once, so text is drawn 1:1 on the pixel grid. The sets are
shelf-packed into extra 1024-wide coverage pages:

  assets/fonts/reborn-page3.a8, reborn-page4.a8   (Tex::Font(3..))
  crates/reborn-ui/src/galleryfonts.rs           metrics and page data

Characters a typeface lacks are left out of its sets; the UI then draws them
from the shared fallback page, as it does for the standard faces.

Typefaces, all under the SIL Open Font License 1.1, as published in the
Google Fonts repository (github.com/google/fonts, ofl/):
  DM Sans 500 / 700       text       DMSans[opsz,wght].ttf   (DMSans-OFL.txt)
  Syne 800                titles     Syne[wght].ttf          (Syne-OFL.txt)
  IBM Plex Mono Medium    captions   IBMPlexMono-Medium.ttf  (IBMPlexMono-OFL.txt)

Usage: build_gallery_fonts.py --fonts DIR [DIR ...]   (searched for the files)
Build-only dependencies: Pillow (with FreeType variation support), fontTools.
No font file is installed on the player.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
PAGE_W = 1024
PAGE_MAX_H = 1024
FIRST_PAGE = 3

ASCII = list(range(32, 127))
LATIN1 = list(range(160, 256))
LATIN_A = list(range(0x100, 0x180))
PUNCT = [0x2022, 0x2026, 0x2013, 0x2014, 0x2018, 0x2019, 0x201C, 0x201D, 0xB7, 0x203A, 0x2039, 0x20AC]
CORE = sorted(set(ASCII + LATIN1 + LATIN_A + PUNCT))
NARROW = sorted(set(ASCII + LATIN1 + [0x2022, 0x2026, 0x2013, 0x2014, 0x2019]))

# family key -> (file, variation axes, label)
FAMILIES = {
    "dm": ("DMSans[opsz,wght].ttf", {"opsz": 14.0, "wght": 500.0}, "DM Sans 500"),
    "dmb": ("DMSans[opsz,wght].ttf", {"opsz": 14.0, "wght": 700.0}, "DM Sans 700"),
    "sy": ("Syne[wght].ttf", {"wght": 800.0}, "Syne 800"),
    "pm": ("IBMPlexMono-Medium.ttf", {}, "IBM Plex Mono Medium"),
}
# Rust name, family, pixel size, charset
SETS = [
    ("DM12", "dm", 12, CORE), ("DM13", "dm", 13, CORE), ("DM14", "dm", 14, CORE),
    ("DM15", "dm", 15, CORE), ("DM16", "dm", 16, CORE),
    ("DMB14", "dmb", 14, CORE), ("DMB17", "dmb", 17, CORE),
    ("SY20", "sy", 20, NARROW), ("SY23", "sy", 23, NARROW), ("SY26", "sy", 26, NARROW),
    # The greeting and the largest titles; ASCII keeps it small.
    ("SY34", "sy", 34, ASCII),
    ("PM10", "pm", 10, NARROW), ("PM11", "pm", 11, CORE), ("PM12", "pm", 12, NARROW),
]


class Family:
    def __init__(self, path: Path, axes: dict):
        self.path, self.axes = path, axes
        tt = TTFont(path)
        self.order = [(a.axisTag, a.defaultValue) for a in tt["fvar"].axes] if "fvar" in tt else []
        self.cmap = tt.getBestCmap()
        self.cache = {}

    def font(self, px: float):
        key = round(px * 64)
        if key not in self.cache:
            f = ImageFont.truetype(str(self.path), px)
            if self.order:
                f.set_variation_by_axes([self.axes.get(tag, default) for tag, default in self.order])
            self.cache[key] = f
        return self.cache[key]


def build_set(fam: Family, px: int, chars, ss: int):
    """Return (cell_w, cell_h, ascent, pad, [(ch, mask, advance64)])."""
    chars = [c for c in sorted(set(chars)) if c in fam.cmap or c == 32]
    probe = fam.font(px * 8)
    lo = hi = top = bot = 0
    for ch in chars:
        l, t, r, b = probe.getbbox(chr(ch), anchor="ls")
        lo, hi = min(lo, l / 8), max(hi, r / 8)
        top, bot = min(top, t / 8), max(bot, b / 8)
    pad = int(math.ceil(-lo)) + 1
    ascent = int(math.ceil(-top)) + 1
    cell_w = pad + int(math.ceil(hi)) + 2
    cell_h = ascent + int(math.ceil(bot)) + 2
    big = fam.font(px * ss)
    adv = fam.font(2048)
    glyphs = []
    for ch in chars:
        img = Image.new("L", (cell_w * ss, cell_h * ss), 0)
        ImageDraw.Draw(img).text((pad * ss, ascent * ss), chr(ch), font=big, fill=255, anchor="ls")
        mask = img.reduce(ss) if ss > 1 else img
        advance = adv.getlength(chr(ch)) / 2048 * px
        glyphs.append((ch, mask, int(round(advance * 64))))
    return cell_w, cell_h, ascent, pad, glyphs


def pack(sets):
    """Shelf-pack sets into pages; returns [page images], {name: (page, [(x, y)])}."""
    pages, layout = [], {}
    items, y, page = [], 0, FIRST_PAGE

    def flush():
        height = (y + 7) // 8 * 8
        img = Image.new("L", (PAGE_W, height), 0)
        for mask, x0, y0 in items:
            img.paste(mask, (x0, y0))
        pages.append(img)

    for name, cell_w, cell_h, masks in sets:
        cols = PAGE_W // cell_w
        rows = -(-len(masks) // cols)
        if y + rows * cell_h > PAGE_MAX_H:
            flush()
            items, y, page = [], 0, page + 1
        pos = []
        for i, mask in enumerate(masks):
            x0, y0 = (i % cols) * cell_w, y + (i // cols) * cell_h
            pos.append((x0, y0))
            items.append((mask, x0, y0))
        layout[name] = (page, pos)
        y += rows * cell_h
    flush()
    return pages, layout


def emit(meta, layout, heights, path):
    """galleryfonts.rs: the pages, a `Face` constant per set and the sets."""
    out = ["// Generated by tools/assets/build_gallery_fonts.py; do not edit.",
           "// Exact-size glyph sets of the Gallery look's typefaces, on glyph pages 3.. .",
           "#![allow(clippy::unreadable_literal, dead_code)]", "",
           "use crate::fontdata::SetData;", "",
           "/// The coverage pages, in page order from 3.",
           "pub const PAGES: [&[u8]; %d] = [" % len(heights)]
    for i in range(len(heights)):
        out.append(f'    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), '
                   f'"/../../assets/fonts/reborn-page{FIRST_PAGE + i}.a8")),')
    out += ["];", ""]
    names = list(meta)
    out.append("impl crate::text::Face {")
    for i, name in enumerate(names):
        m = meta[name]
        out.append(f"    /// {m['label']}, {m['size']} px.")
        out.append(f"    pub const {name}: Self = Self::Gallery({i});")
    out += ["}", "", f"pub const SETS: [SetData; {len(names)}] = [{', '.join(names)}];", ""]
    for name in names:
        m = meta[name]
        page, pos = layout[name]
        rows = ", ".join(f"({ch}, {x}, {y}, {adv})" for (ch, adv), (x, y) in zip(m["glyphs"], pos))
        out += ["#[rustfmt::skip]",
                f"const {name}: SetData = SetData {{ page: {page}, size: {m['size']}, cell_w: {m['cell_w']}, "
                f"cell_h: {m['cell_h']}, ascent: {m['ascent']}, pad: {m['pad']}, glyphs: &[{rows}] }};", ""]
    path.write_text("\n".join(out))


def find(dirs, name):
    for d in dirs:
        if (d / name).is_file():
            return d / name
    raise SystemExit(f"{name} not found in {', '.join(map(str, dirs))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--fonts", type=Path, nargs="+", required=True)
    parser.add_argument("--supersample", type=int, default=4)
    args = parser.parse_args()
    files = {key: find(args.fonts, spec[0]) for key, spec in FAMILIES.items()}
    fams = {key: Family(files[key], FAMILIES[key][1]) for key in FAMILIES}
    meta, packed = {}, []
    for name, fam, px, chars in SETS:
        cell_w, cell_h, ascent, pad, glyphs = build_set(fams[fam], px, chars, args.supersample)
        packed.append((name, cell_w, cell_h, [g[1] for g in glyphs]))
        meta[name] = dict(size=px, cell_w=cell_w, cell_h=cell_h, ascent=ascent, pad=pad,
                          glyphs=[(g[0], g[2]) for g in glyphs], label=FAMILIES[fam][2])
    pages, layout = pack(packed)
    for old in (ROOT / "assets/fonts").glob("reborn-page*.a8"):
        old.unlink()
    heights = []
    for i, img in enumerate(pages):
        (ROOT / f"assets/fonts/reborn-page{FIRST_PAGE + i}.a8").write_bytes(img.tobytes())
        heights.append(img.size[1])
    emit(meta, layout, heights, ROOT / "crates/reborn-ui/src/galleryfonts.rs")
    licences = {"dm": "DMSans-OFL.txt", "dmb": "DMSans-OFL.txt", "sy": "Syne-OFL.txt",
                "pm": "IBMPlexMono-OFL.txt"}
    provenance = {
        "families": {FAMILIES[k][2]: {"file": files[k].name,
                                      "sha256": hashlib.sha256(files[k].read_bytes()).hexdigest(),
                                      "axes": FAMILIES[k][1],
                                      "license": f"SIL Open Font License 1.1 (assets/fonts/{licences[k]})"}
                     for k in FAMILIES},
        "source": "github.com/google/fonts ofl/dmsans, ofl/syne, ofl/ibmplexmono; build input only",
        "sets": {n: {"page": layout[n][0], "px": m["size"], "font": m["label"], "glyphs": len(m["glyphs"]),
                     "cell": [m["cell_w"], m["cell_h"]]} for n, m in meta.items()},
        "page_heights": heights, "supersample": args.supersample, "runtime_font_files": False,
    }
    (ROOT / "assets/fonts/gallery-provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print("pages", heights, "sets", len(meta))


if __name__ == "__main__":
    main()

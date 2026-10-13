//! Drawing the podcast screens: two-line rows for shows and episodes (with
//! an episode's download or listening progress), and the episode page,
//! scrolled so the focused part is in view.
use crate::{
    canvas::{Canvas, Icon},
    components,
    podcasts::{self, Block},
    screens::screen_title,
    text::{self, Face},
    theme::{alpha, color, HEIGHT, WIDTH},
    Item, RowKind, Ui,
};
use reborn_core::{AppModel, Screen};

/// Rows of the lists.
const ROW_H: f32 = 56.;
/// The page's visible band, under the status bar.
const PAGE_TOP: f32 = 42.;
const PAGE_BOTTOM: f32 = HEIGHT - 8.;
/// Text column of the episode page.
const TEXT_X: f32 = 24.;
const TEXT_W: f32 = WIDTH - 48.;

pub(crate) fn draw(c: &mut Canvas, ui: &Ui, m: &AppModel, interactive: bool) {
    let v = &ui.podcasts;
    let title = match m.screen {
        Screen::PodcastShow => v.show(&m.navigation.filter).map(|s| s.title.clone()),
        Screen::PodcastEpisode => v
            .find(&m.navigation.filter)
            .map(|(show, _)| show.title.clone()),
        _ => None,
    }
    .unwrap_or_else(|| screen_title(m));
    components::status_bar(c, ui, m, &title.to_uppercase(), true);
    match m.screen {
        Screen::PodcastEpisode => draw_page(c, ui, m, interactive),
        _ => draw_list(c, ui, m, interactive),
    }
}

fn draw_list(c: &mut Canvas, ui: &Ui, m: &AppModel, interactive: bool) {
    let page = crate::pages::page(ui, m, &[]);
    let rows = &page.rows;
    let focus = m.navigation.focus.min(rows.len().saturating_sub(1));
    let mut top = 40.;
    if let Some(h) = &page.hero {
        top = empty_state(c, ui, &h.title, &h.body);
    }
    let visible = (((PAGE_BOTTOM - top) / ROW_H).floor() as usize).max(1);
    let start = focus
        .saturating_sub(visible - 1)
        .min(rows.len().saturating_sub(visible));
    for (slot, row) in rows.iter().enumerate().skip(start).take(visible) {
        let y = top + (slot - start) as f32 * ROW_H;
        list_row(c, ui, row, y, interactive && slot == focus);
    }
    components::scroll_rail(
        c,
        top,
        visible as f32 * ROW_H,
        start,
        rows.len(),
        visible,
        ui.accent(),
    );
}

/// A calm empty state: a ring with headphones, a title and a line; returns
/// where the rows start below it.
fn empty_state(c: &mut Canvas, ui: &Ui, title: &str, body: &str) -> f32 {
    c.ring(240., 84., 26., 1.5, color::GLASS_STRONG);
    c.icon(Icon::Headphones, 240., 84., 26., alpha(ui.accent(), 0xE0));
    c.text_center(
        240.,
        134. + Face::S17.mid_offset(),
        title,
        Face::S17,
        color::TEXT,
    );
    let mut y = 158.;
    for line in text::wrap(Face::R13, body, 400., 3) {
        c.text_center(
            240.,
            y + Face::R13.mid_offset(),
            &line,
            Face::R13,
            color::MUTED,
        );
        y += 18.;
    }
    y + 14.
}

/// One list row: an icon tile, two lines, and at the right a chevron or the
/// episode's progress.
fn list_row(c: &mut Canvas, ui: &Ui, item: &Item, y: f32, focused: bool) {
    let v = &ui.podcasts;
    let accent = ui.accent();
    let (x, w, h) = (10., WIDTH - 20., ROW_H - 4.);
    if focused {
        components::glass_row(c, x, y, w, h);
    }
    let cy = y + h / 2.;
    let (ax, size) = (x + 10., 42.);
    let ay = cy - size / 2.;
    let episode = item
        .key
        .strip_prefix("pod_ep:")
        .and_then(|id| v.find(id))
        .map(|(_, e)| e);
    let right = x + w - 12.;
    let mut tail = 0.;
    match (episode, item.kind) {
        (Some(e), _) => {
            let tile = if v.is_on_card(&e.id) {
                Icon::Headphones
            } else {
                Icon::Download
            };
            c.rrect(ax, ay, size, size, 8., color::GLASS_FAINT);
            match v.download_progress(&e.id) {
                Some(part) => {
                    c.arc(ax + size / 2., ay + size / 2., 13., 1., color::TRACK);
                    c.arc(
                        ax + size / 2.,
                        ay + size / 2.,
                        13.,
                        part.unwrap_or(0.05),
                        accent,
                    );
                }
                None => c.icon(tile, ax + size / 2., ay + size / 2., 18., color::MUTED),
            }
            let heard = v.heard(e);
            if heard >= 1. {
                c.icon(Icon::Check, right - 10., cy, 18., color::SUCCESS);
                tail = 24.;
            } else if heard > 0. {
                c.arc(right - 11., cy, 11., 1., color::TRACK);
                c.arc(right - 11., cy, 11., heard.max(0.04), accent);
                tail = 26.;
            }
        }
        (None, kind) => {
            let icon = if item.key == "pod_refresh" {
                Icon::Refresh
            } else {
                Icon::Headphones
            };
            c.rrect(ax, ay, size, size, 10., alpha(accent, 0x30));
            c.icon(icon, ax + size / 2., ay + size / 2., 20., accent);
            if kind == RowKind::Chevron {
                components::chevron(c, right + 2., cy);
                tail = 18.;
            }
        }
    }
    let tx = ax + size + 12.;
    let tw = right - tail - tx - 8.;
    let face = if focused { Face::S15 } else { Face::R15 };
    if item.secondary.is_empty() {
        c.text_mid(tx, cy, tw, &item.label, face, color::TEXT);
    } else {
        c.text_mid(tx, cy - 9., tw, &item.label, face, color::TEXT);
        let sub = if focused { color::TEXT } else { color::MUTED };
        c.text_mid(
            tx,
            cy + 11.,
            tw,
            &item.secondary,
            Face::R11,
            alpha(sub, 0xD0),
        );
    }
}

/// Height of each block at the page's width.
fn block_height(block: &Block) -> f32 {
    match block {
        Block::Header => 112.,
        Block::Actions(_) => 48.,
        Block::Heading(_) => 32.,
        Block::Text { item, first } => {
            text_lines(&item.label).len() as f32 * 21. + text_gap(item, *first)
        }
        Block::Note(_) => 30.,
    }
}

/// Space above a piece of the description: more before a paragraph than
/// before a list item or the rest of a paragraph.
fn text_gap(item: &Item, first: bool) -> f32 {
    match (first, item.label.starts_with("- ")) {
        (true, false) => 10.,
        (true, true) => 4.,
        _ => 2.,
    }
}

fn text_lines(text: &str) -> Vec<String> {
    text::wrap(Face::R15, text, TEXT_W, 12)
}

fn draw_page(c: &mut Canvas, ui: &Ui, m: &AppModel, interactive: bool) {
    let blocks = podcasts::episode_page(&ui.podcasts, &m.navigation.filter);
    let rows = podcasts::page_rows(&blocks);
    let focus = m.navigation.focus.min(rows.len().saturating_sub(1));
    // Where each block sits on the whole page, and which rows it holds.
    let mut laid = vec![];
    let (mut y, mut row) = (PAGE_TOP + 2., 0usize);
    for block in &blocks {
        let h = block_height(block);
        let n = block.rows().len();
        laid.push((y, h, row..row + n));
        y += h;
        row += n;
    }
    let total = y - PAGE_TOP;
    // Scroll so the focused block is in view (the header shows while the
    // buttons are focused).
    let focused = laid
        .iter()
        .position(|(_, _, r)| r.contains(&focus))
        .unwrap_or(0);
    let (fy, fh, _) = laid.get(focused).cloned().unwrap_or((PAGE_TOP, 0., 0..0));
    let mut offset = (fy + fh + 6. - PAGE_BOTTOM).max(0.);
    if fy - offset < PAGE_TOP {
        offset = fy - PAGE_TOP;
    }
    for (block, (by, h, rows_here)) in blocks.iter().zip(&laid) {
        let top = by - offset;
        if top < PAGE_TOP - 1. || top + h > PAGE_BOTTOM + 8. {
            continue;
        }
        let focus_here =
            (interactive && rows_here.contains(&focus)).then(|| focus - rows_here.start);
        draw_block(c, ui, m, block, top, *h, focus_here);
    }
    // Where the view is on a long page.
    let view = PAGE_BOTTOM - PAGE_TOP;
    if total > view + 1. {
        let thumb = (view * view / total).max(18.);
        let at = PAGE_TOP + (view - thumb) * (offset / (total - view)).clamp(0., 1.);
        c.rrect(WIDTH - 8., PAGE_TOP, 2., view, 1., color::TRACK);
        c.rrect(WIDTH - 8., at.round(), 2., thumb.round(), 1., ui.accent());
    }
}

fn draw_block(
    c: &mut Canvas,
    ui: &Ui,
    m: &AppModel,
    block: &Block,
    y: f32,
    h: f32,
    focus: Option<usize>,
) {
    let accent = ui.accent();
    match block {
        Block::Header => header(c, ui, m, y),
        Block::Actions(items) => actions(c, ui, items, y, focus),
        Block::Heading(title) => {
            c.rect(TEXT_X, y + 6., TEXT_W, 1., color::RULE);
            components::section_label(c, TEXT_X, y + 21., title);
        }
        Block::Text { item, first } => {
            let top = y + text_gap(item, *first);
            if focus.is_some() {
                let at = c.draw.len();
                c.rrect(
                    TEXT_X - 10.,
                    top - 3.,
                    TEXT_W + 20.,
                    h - (top - y) + 2.,
                    10.,
                    color::GLASS_FAINT,
                );
                c.mark_focus(at);
                c.rect(TEXT_X - 10., top + 2., 3., h - (top - y) - 8., accent);
            }
            for (i, line) in text_lines(&item.label).iter().enumerate() {
                c.text_mid(
                    TEXT_X,
                    top + 10. + i as f32 * 21.,
                    TEXT_W,
                    line,
                    Face::R15,
                    color::TEXT,
                );
            }
        }
        Block::Note(note) => {
            c.text_mid(TEXT_X, y + 15., TEXT_W, note, Face::R13, color::DIM);
        }
    }
}

/// A line of pill buttons; the first is the main one.
fn actions(c: &mut Canvas, ui: &Ui, items: &[Item], y: f32, focus: Option<usize>) {
    let accent = ui.accent();
    let mut x = TEXT_X;
    let h = 34.;
    for (i, item) in items.iter().enumerate() {
        let icon = match item.key.split(':').next().unwrap_or("") {
            "pod_play" | "now_playing" => Some(Icon::Play),
            "pod_download" | "pod_wait" => Some(Icon::Download),
            _ => None,
        };
        let iw = if icon.is_some() { 20. } else { 0. };
        let w = text::width(Face::S13, &item.label) + 32. + iw;
        let selected = focus == Some(i);
        let (fill, ink) = if selected {
            (0xF2F3F5FF, color::INK)
        } else if i == 0 {
            (alpha(accent, 0x55), color::TEXT)
        } else {
            (color::GLASS, color::TEXT)
        };
        let at = c.draw.len();
        c.rrect(x, y + 4., w, h, h / 2., fill);
        if selected {
            c.mark_focus(at);
        }
        if let Some(icon) = icon {
            c.icon(icon, x + 22., y + 4. + h / 2., 13., ink);
        }
        c.text_mid(
            x + 16. + iw,
            y + 4. + h / 2.,
            w - 28. - iw,
            &item.label,
            Face::S13,
            ink,
        );
        x += w + 10.;
    }
}

/// Title, when it came out and how long it is (or what is left), and how
/// much has been heard or downloaded.
fn header(c: &mut Canvas, ui: &Ui, m: &AppModel, y: f32) {
    let v = &ui.podcasts;
    let Some((_, e)) = v.find(&m.navigation.filter) else {
        return;
    };
    let mut cy = y + 14.;
    for line in text::wrap(Face::S20, &e.title, TEXT_W, 2) {
        c.text_mid(TEXT_X, cy + 2., TEXT_W, &line, Face::S20, color::TEXT);
        cy += 25.;
    }
    cy += 4.;
    let when = reborn_core::podcasts::age_text(e.published, v.now);
    let detail = v.episode_detail(e);
    let meta = if detail.starts_with(&when) || when.is_empty() {
        detail
    } else {
        format!("{when} · {detail}")
    };
    c.text_mid(TEXT_X, cy, TEXT_W, &meta, Face::R13, color::MUTED);
    let progress = match v.download_progress(&e.id) {
        Some(part) => Some((part.unwrap_or(0.), color::MUTED)),
        None => Some((v.heard(e), ui.accent())).filter(|(heard, _)| *heard > 0.),
    };
    if let Some((part, lit)) = progress {
        components::slider(c, TEXT_X, cy + 22., TEXT_W, part, lit, false);
    }
}

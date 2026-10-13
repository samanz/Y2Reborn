//! What Gallery draws above or around any screen: the status strip, dialogs
//! and option sheets as glass panels, and toasts.
use super::kit::{self, display, glass, top_right, top_text, TIGHT};
use super::palette::{Colors, DANGER, DIM, FG, MUTED};
use crate::{
    canvas::{Canvas, Icon},
    components::{SheetRow, Tone},
    text::{self, Face},
    theme::{alpha, fade, mix, HEIGHT, WIDTH},
    Ui, WifiStatus,
};
use reborn_core::{
    platform::{ChargingState, LowBattery},
    AppModel, PlaybackState,
};

fn battery_low(m: &AppModel) -> bool {
    let b = m.platform.battery;
    matches!(
        b.level,
        LowBattery::Low | LowBattery::Critical | LowBattery::ShuttingDown
    ) || b.percent.is_some_and(|p| p <= 15)
}

/// A battery: outline, nub and the charge level.
fn battery(c: &mut Canvas, m: &AppModel, x: f32, y: f32, tint: u32) {
    let low = battery_low(m);
    c.rrect_outline(x, y, 22., 12., 3., 1., tint);
    c.rect(x + 23., y + 4., 2., 4., tint);
    if let Some(p) = m.platform.battery.percent.filter(|p| *p <= 100) {
        let w = (18. * f32::from(p) / 100.).round().max(2.);
        c.rect(x + 2., y + 2., w, 8., if low { DANGER } else { FG });
    }
}

/// The status strip as on a phone: the clock on the left (with level bars
/// while something plays), the radios, the charge and the battery on the
/// right. Drawn over the room, not on a bar.
pub(crate) fn bar(c: &mut Canvas, ui: &Ui, m: &AppModel, colors: &Colors) {
    let accent = colors.accent;
    let mut x = 20.;
    if let Some(time) = ui.clock(m) {
        top_text(c, x, 7., 120., &time, Face::DMB14, FG);
        x += text::width(Face::DMB14, &time) + 10.;
    }
    if m.playback == PlaybackState::Playing {
        kit::eq_bars(c, x, 20., accent);
    }
    let tint = if battery_low(m) { DANGER } else { MUTED };
    battery(c, m, 436., 9., tint);
    let mut x = 428.;
    if matches!(
        m.platform.battery.charging,
        ChargingState::Charging | ChargingState::Full
    ) {
        c.icon(Icon::Bolt, x - 6., 15., 13., accent);
        x -= 16.;
    }
    if let Some(percent) = m.platform.battery.percent.filter(|p| *p <= 100) {
        let label = format!("{percent}%");
        top_right(c, x, 8., &label, Face::PM11, tint);
        x -= text::width(Face::PM11, &label) + 12.;
    }
    if ui.bluetooth.powered {
        let on = ui.bluetooth.devices.iter().any(|d| d.connected);
        c.icon(Icon::Bluetooth, x - 8., 15., 15., if on { FG } else { DIM });
        x -= 22.;
    }
    if ui.wifi.powered {
        let on = matches!(ui.wifi.status, WifiStatus::Connected(_));
        c.icon(Icon::Wifi, x - 8., 15., 15., if on { FG } else { DIM });
    }
}

/// The strip over a screen Gallery does not lay out itself: its name in
/// spaced mono in the middle.
pub(crate) fn status_title(c: &mut Canvas, title: &str) {
    let s = text::fit(Face::PM10, &title.to_uppercase(), 180.);
    c.text_center_tracked(
        WIDTH / 2.,
        8. + kit::baseline(Face::PM10),
        &s,
        Face::PM10,
        kit::SOFT,
        1.8,
    );
}

/// The focus on a glass panel: a soft glow of the accent under a lifted
/// panel with a glowing outline. Marks the focus.
#[allow(clippy::too_many_arguments)]
pub(crate) fn panel_focus(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, accent: u32) {
    c.glow(
        x + w / 2.,
        y + h / 2. + 3.,
        w / 2. + 18.,
        h / 2. + 22.,
        alpha(accent, 0x40),
    );
    c.rrect(x, y, w, h, r, alpha(mix(0x1E1F24FF, accent, 0.16), 0xE8));
    let at = c.draw.len();
    c.rrect_outline(
        x,
        y,
        w,
        h,
        r,
        2.,
        alpha(mix(accent, 0xFFFFFFFF, 0.15), 0xE0),
    );
    c.mark_focus(at);
}

/// A centred glass sheet: the title, the body and rows with the glowing
/// focus. Option sheets and dialogs both take this form.
pub(crate) fn dialog(
    c: &mut Canvas,
    colors: &Colors,
    title: &str,
    body: &str,
    rows: &[SheetRow],
    focus: usize,
) {
    const PITCH: f32 = 40.;
    let lines = text::wrap(Face::DM14, body, 376., 3);
    let heads = text::wrap(Face::SY20, title, 376. - TIGHT * 30., 2);
    let header = 56.
        + (heads.len().max(1) - 1) as f32 * 24.
        + lines.len() as f32 * 20.
        + if lines.is_empty() { 0. } else { 10. };
    let capacity = ((328. - header - 14.) / PITCH).floor().clamp(1., 6.) as usize;
    let visible = rows.len().clamp(1, capacity);
    let height = header + visible as f32 * PITCH + 14.;
    let top = ((HEIGHT - height) / 2.).max(16.).round();
    c.rect(0., 0., WIDTH, HEIGHT, alpha(colors.bg_deep, 0xD8));
    c.glow(240., top + 30., 220., 90., alpha(colors.accent, 0x30));
    c.rrect(
        32.,
        top,
        416.,
        height,
        22.,
        alpha(mix(colors.surface, 0x000000FF, 0.1), 0xF8),
    );
    c.rrect_outline(32., top, 416., height, 22., 1., 0xF2F3F71F);
    let mut y = top + 18.;
    for line in &heads {
        display(c, 52., y, 376., line, Face::SY20, FG);
        y += 24.;
    }
    y += 10.;
    for line in &lines {
        top_text(c, 52., y, 376., line, Face::DM14, MUTED);
        y += 20.;
    }
    let first = top + header;
    let start = focus.saturating_sub(visible - 1);
    for (i, row) in rows.iter().enumerate().skip(start).take(visible) {
        let y = first + (i - start) as f32 * PITCH;
        if i == focus && row.enabled {
            panel_focus(c, 46., y, 388., 36., 18., colors.accent);
        }
        let tail = text::fit(Face::PM11, row.detail, 150.);
        let tail_w = text::width(Face::PM11, &tail);
        top_text(
            c,
            64.,
            y + 10.,
            350. - tail_w,
            row.label,
            Face::DM15,
            if row.enabled { FG } else { DIM },
        );
        if !tail.is_empty() {
            top_right(c, 416., y + 12., &tail, Face::PM11, MUTED);
        }
    }
    if rows.len() > visible {
        // Where the visible rows sit among all of them.
        let track = visible as f32 * PITCH - 8.;
        let thumb = (track * visible as f32 / rows.len() as f32).max(14.);
        let at = (track - thumb) * start as f32 / (rows.len() - visible) as f32;
        c.rrect(440., first + 4. + at, 3., thumb, 1.5, fade(FG, 0.3));
    }
}

/// A glass pill under the status strip (clear of the mini player and the
/// tab pill), with a coloured dot for its tone.
pub(crate) fn toast(c: &mut Canvas, colors: &Colors, message: &str, tone: Tone) {
    const Y: f32 = 36.;
    let shown = text::fit(Face::DM14, message, 380.);
    let w = (text::width(Face::DM14, &shown) + 54.).max(160.);
    let x = ((WIDTH - w) / 2.).round();
    c.glow(240., Y + 20., w / 2. + 30., 34., 0x000000A0);
    c.rrect(x, Y, w, 36., 18., 0x25262DF4);
    c.rrect_outline(x, Y, w, 36., 18., 1., 0xF2F3F724);
    let dot = if tone == Tone::Error {
        DANGER
    } else {
        colors.accent
    };
    c.disc(x + 20., Y + 18., 4., dot);
    top_text(c, x + 34., Y + 9., w - 46., &shown, Face::DM14, FG);
}

/// A glass message panel: unused space is calmer than an empty list.
pub(crate) fn note(c: &mut Canvas, y: f32, message: &str) {
    glass(c, 16., y, 448., 44., 16.);
    top_text(c, 32., y + 15., 416., message, Face::DM13, MUTED);
}

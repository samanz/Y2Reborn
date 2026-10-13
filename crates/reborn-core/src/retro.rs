//! Retro games (Extras > Games > Retro games): the systems and which files
//! are their games, the pipe protocol spoken with the `reborn-retro`
//! frontend, the player's buttons as game pads (one layout table per
//! system), and how a frame fits the screen.
//!
//! The emulators are libretro cores installed by the image
//! (`/usr/lib/libretro/<core>_libretro.so`). Reborn never loads them: it
//! starts `reborn-retro` (its own small frontend, which does) and only
//! exchanges bytes with it. The frontend's stdout carries messages of an
//! 8-byte header ('R', 'R', kind, flags, length as a little-endian u32) and
//! a payload ([`Message`]); its stdin takes 2-byte messages
//! ([`Input::encode`]). Both sides use this module, so they stay in step.
use crate::{NormalizedInput, PhysicalControl};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

/// The folder on a card holding one folder per system (`Games/NES`, ...).
pub const CARD_FOLDER: &str = "Games";
/// `Games/BIOS`: system files some cores look for (the core's system
/// directory).
pub const BIOS_FOLDER: &str = "BIOS";
/// Where the image installs the cores.
pub const CORE_DIR: &str = "/usr/lib/libretro";

// ---------------------------------------------------------------------------
// Systems and games
// ---------------------------------------------------------------------------

/// A console family, played by one core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum System {
    Nes,
    GameBoy,
    Gba,
    Genesis,
    Snes,
    Arcade,
}

impl System {
    pub const ALL: [System; 6] = [
        Self::Nes,
        Self::GameBoy,
        Self::Gba,
        Self::Genesis,
        Self::Snes,
        Self::Arcade,
    ];

    /// Short stable name: navigation filters, save folders.
    pub fn key(self) -> &'static str {
        match self {
            Self::Nes => "nes",
            Self::GameBoy => "gb",
            Self::Gba => "gba",
            Self::Genesis => "genesis",
            Self::Snes => "snes",
            Self::Arcade => "arcade",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Nes => "NES",
            Self::GameBoy => "Game Boy and Color",
            Self::Gba => "Game Boy Advance",
            Self::Genesis => "Genesis and Master System",
            Self::Snes => "SNES",
            Self::Arcade => "Arcade",
        }
    }

    /// Folders under `Games/` on the card, the first being the one the
    /// player is told about.
    pub fn folders(self) -> &'static [&'static str] {
        match self {
            Self::Nes => &["NES", "Famicom"],
            Self::GameBoy => &["GB", "GBC", "Game Boy", "Game Boy Color"],
            Self::Gba => &["GBA", "Game Boy Advance"],
            Self::Genesis => &[
                "Genesis",
                "Mega Drive",
                "MegaDrive",
                "Master System",
                "SMS",
                "Game Gear",
                "GG",
            ],
            Self::Snes => &["SNES", "Super Nintendo", "Super Famicom"],
            Self::Arcade => &["Arcade", "FBNeo"],
        }
    }

    /// File extensions (lower case) that are this system's games.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Nes => &["nes"],
            Self::GameBoy => &["gb", "gbc"],
            Self::Gba => &["gba"],
            Self::Genesis => &["md", "gen", "bin", "smd", "sms", "gg"],
            Self::Snes => &["sfc", "smc"],
            Self::Arcade => &["zip"],
        }
    }

    /// The libretro core (its file is `<core>_libretro.so`).
    pub fn core(self) -> &'static str {
        match self {
            Self::Nes => "fceumm",
            Self::GameBoy => "gambatte",
            Self::Gba => "gpsp",
            Self::Genesis => "picodrive",
            Self::Snes => "snes9x2005",
            Self::Arcade => "fbneo",
        }
    }

    /// Where to put games, in the words the player reads.
    pub fn where_to_put(self) -> String {
        let ext: Vec<String> = self.extensions().iter().map(|e| format!(".{e}")).collect();
        format!(
            "Put {} files in {}/{} on the card.",
            ext.join(" "),
            CARD_FOLDER,
            self.folders()[0]
        )
    }
}

/// A game file the player can start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rom {
    pub system: System,
    /// The file name without its extension and its bracketed tags.
    pub title: String,
    /// The tags, e.g. "USA" from "Game (USA).nes", for the second line.
    pub tags: String,
    pub path: PathBuf,
}

/// Arcade sets that are BIOS or shared device files, not games.
const ARCADE_SUPPORT: [&str; 12] = [
    "neogeo", "pgm", "skns", "decocass", "isgsm", "nmk004", "ym2608", "cchip", "bubsys", "qsound",
    "midssio", "namcoc69",
];

/// The game for the file at `path`, when its extension is one of the
/// system's.
pub fn rom(system: System, path: PathBuf) -> Option<Rom> {
    let name = path.file_name()?.to_str()?;
    if name.starts_with('.') {
        return None; // macOS "._" sidecars and other hidden files
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !system.extensions().contains(&ext.as_str()) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?.to_owned();
    if system == System::Arcade && ARCADE_SUPPORT.contains(&stem.to_ascii_lowercase().as_str()) {
        return None;
    }
    let (title, tags) = split_tags(&stem);
    Some(Rom {
        system,
        title,
        tags,
        path,
    })
}

/// "Game (USA) [!]" -> ("Game", "USA, !").
fn split_tags(stem: &str) -> (String, String) {
    let mut title = String::new();
    let mut tags: Vec<String> = vec![];
    let mut depth = 0;
    let mut tag = String::new();
    let mut square = false;
    for c in stem.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                if depth == 1 {
                    tag.clear();
                    square = c == '[';
                    continue;
                }
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    // Square-bracket tags are dump notes ("[!]", "[b1]"):
                    // left out.
                    if !tag.trim().is_empty() && !square {
                        tags.push(tag.trim().to_owned());
                    }
                    continue;
                }
            }
            _ => {}
        }
        if depth > 0 {
            tag.push(c);
        } else {
            title.push(c);
        }
    }
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        (stem.to_owned(), String::new())
    } else {
        (title, tags.join(", "))
    }
}

/// The games to offer: one per file name (the first found wins), sorted by
/// title.
pub fn roms(found: Vec<Rom>) -> Vec<Rom> {
    let mut out: Vec<Rom> = vec![];
    for rom in found {
        let name = |r: &Rom| r.path.file_name().map(|n| n.to_ascii_lowercase());
        if !out.iter().any(|r| name(r) == name(&rom)) {
            out.push(rom);
        }
    }
    out.sort_by_key(|r| (r.title.to_lowercase(), r.tags.to_lowercase()));
    out
}

/// The name a game's save files share: its file name without the
/// extension (`<saves>/<system>/<stem>.srm`, `.state`).
pub fn save_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("game")
        .replace('/', "_")
}

/// What the app needs to start a game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub system: System,
    pub rom: PathBuf,
    pub title: String,
}

/// Things to ask of a running game (the in-game menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SaveState,
    LoadState,
    /// Press and release a pad button the buttons cannot reach.
    Tap(Pad),
}

/// The last thing the frontend said before stopping, worth showing.
pub fn failure_line(stderr_tail: &[String]) -> Option<String> {
    let lines = || {
        stderr_tail
            .iter()
            .rev()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
    };
    lines()
        .find(|l| l.starts_with("reborn-retro:"))
        .or_else(|| lines().next())
        .map(|l| {
            l.trim_start_matches("reborn-retro:")
                .trim_start()
                .chars()
                .take(120)
                .collect()
        })
}

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

pub const HEADER_BYTES: usize = 8;
/// Largest frame side the picture texture takes.
pub const MAX_SIDE: u16 = 1024;
const MAX_AUDIO_BYTES: usize = 64 * 1024;
const MAX_TEXT_BYTES: usize = 512;

/// libretro pixel formats, by their libretro numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 0RGB1555, the libretro default.
    Rgb1555 = 0,
    Xrgb8888 = 1,
    Rgb565 = 2,
}

impl PixelFormat {
    pub fn from_libretro(n: u32) -> Option<Self> {
        match n {
            0 => Some(Self::Rgb1555),
            1 => Some(Self::Xrgb8888),
            2 => Some(Self::Rgb565),
            _ => None,
        }
    }

    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Xrgb8888 => 4,
            _ => 2,
        }
    }
}

/// What the game is (sent once it loads, and again if it changes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Info {
    /// Frames a second, in thousandths.
    pub fps_milli: u32,
    /// The core's own sample rate (the frontend converts it).
    pub core_rate: u32,
    pub base_width: u16,
    pub base_height: u16,
}

/// What a status message reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    /// A message from the core itself.
    Core = 0,
    StateSaved = 1,
    StateLoaded = 2,
    /// Saving or loading a state did not work; the text says why.
    StateFailed = 3,
}

/// One message from the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Info(Info),
    /// One picture, rows packed (no padding), in its pixel format.
    Frame {
        width: u16,
        height: u16,
        format: PixelFormat,
        pixels: Vec<u8>,
    },
    /// Interleaved stereo signed 16-bit samples at the rate Reborn asked for.
    Audio(Vec<i16>),
    Status(StatusKind, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    BadMagic,
    UnknownKind(u8),
    BadLength { kind: u8, length: u32 },
    BadFrame,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "game stream out of step"),
            Self::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            Self::BadLength { kind, length } => {
                write!(f, "message {} of {length} bytes", *kind as char)
            }
            Self::BadFrame => write!(f, "a picture of an impossible size"),
        }
    }
}

fn header(kind: u8, flags: u8, length: usize) -> [u8; HEADER_BYTES] {
    let l = (length as u32).to_le_bytes();
    [b'R', b'R', kind, flags, l[0], l[1], l[2], l[3]]
}

impl Message {
    /// The bytes on the wire (the frontend's side).
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Info(i) => {
                let mut out = header(b'I', 0, 12).to_vec();
                out.extend_from_slice(&i.fps_milli.to_le_bytes());
                out.extend_from_slice(&i.core_rate.to_le_bytes());
                out.extend_from_slice(&i.base_width.to_le_bytes());
                out.extend_from_slice(&i.base_height.to_le_bytes());
                out
            }
            Self::Frame {
                width,
                height,
                format,
                pixels,
            } => {
                let mut out = Vec::with_capacity(HEADER_BYTES + 8 + pixels.len());
                encode_frame_header(&mut out, *width, *height, *format);
                out.extend_from_slice(pixels);
                out
            }
            Self::Audio(samples) => {
                let mut out = header(b'A', 0, samples.len() * 2).to_vec();
                for s in samples {
                    out.extend_from_slice(&s.to_le_bytes());
                }
                out
            }
            Self::Status(kind, text) => {
                let text: String = text.chars().take(MAX_TEXT_BYTES / 4).collect();
                let mut out = header(b'S', *kind as u8, text.len()).to_vec();
                out.extend_from_slice(text.as_bytes());
                out
            }
        }
    }
}

/// A frame's header and size fields, for a sender that writes the pixels
/// itself (without copying them into a message first).
pub fn encode_frame_header(out: &mut Vec<u8>, width: u16, height: u16, format: PixelFormat) {
    let bytes = usize::from(width) * usize::from(height) * format.bytes_per_pixel();
    out.extend_from_slice(&header(b'F', 0, 8 + bytes));
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&[format as u8, 0, 0, 0]);
}

/// Splits the frontend's output into messages, whatever sizes the reads
/// come in. A protocol error is final.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    at: usize,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        if self.at > 0 && self.at == self.buf.len() {
            self.buf.clear();
            self.at = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_message(&mut self) -> Result<Option<Message>, ProtocolError> {
        let rest = &self.buf[self.at..];
        if rest.len() < HEADER_BYTES {
            return Ok(None);
        }
        if &rest[..2] != b"RR" {
            return Err(ProtocolError::BadMagic);
        }
        let (kind, flags) = (rest[2], rest[3]);
        let length = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
        let len = length as usize;
        let max_frame = 8 + usize::from(MAX_SIDE) * usize::from(MAX_SIDE) * 4;
        let ok = match kind {
            b'I' => len == 12,
            b'F' => (8..=max_frame).contains(&len),
            b'A' => len.is_multiple_of(4) && len <= MAX_AUDIO_BYTES,
            b'S' => len <= MAX_TEXT_BYTES,
            other => return Err(ProtocolError::UnknownKind(other)),
        };
        if !ok {
            return Err(ProtocolError::BadLength { kind, length });
        }
        if rest.len() < HEADER_BYTES + len {
            return Ok(None);
        }
        let p = &rest[HEADER_BYTES..HEADER_BYTES + len];
        let u16_at = |i: usize| u16::from_le_bytes([p[i], p[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
        let message = match kind {
            b'I' => Message::Info(Info {
                fps_milli: u32_at(0),
                core_rate: u32_at(4),
                base_width: u16_at(8),
                base_height: u16_at(10),
            }),
            b'F' => {
                let (width, height) = (u16_at(0), u16_at(2));
                let format =
                    PixelFormat::from_libretro(u32::from(p[4])).ok_or(ProtocolError::BadFrame)?;
                let bytes = usize::from(width) * usize::from(height) * format.bytes_per_pixel();
                if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
                    return Err(ProtocolError::BadFrame);
                }
                if bytes != len - 8 {
                    return Err(ProtocolError::BadLength { kind, length });
                }
                Message::Frame {
                    width,
                    height,
                    format,
                    pixels: p[8..].to_vec(),
                }
            }
            b'A' => Message::Audio(
                p.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes(*b))
                    .collect(),
            ),
            _ => {
                let status = match flags {
                    1 => StatusKind::StateSaved,
                    2 => StatusKind::StateLoaded,
                    3 => StatusKind::StateFailed,
                    _ => StatusKind::Core,
                };
                Message::Status(status, String::from_utf8_lossy(p).into_owned())
            }
        };
        self.at += HEADER_BYTES + len;
        if self.at > self.buf.len() / 2 {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        Ok(Some(message))
    }
}

/// Turns a packed picture (`bpp` bytes a pixel) by `quarter_turns` x 90
/// degrees anticlockwise, as libretro's SET_ROTATION asks of the frontend
/// (a vertical arcade game draws its screen on its side and asks for 1).
/// Returns the new width and height.
pub fn rotate(
    pixels: &[u8],
    width: usize,
    height: usize,
    bpp: usize,
    quarter_turns: u32,
    out: &mut Vec<u8>,
) -> (usize, usize) {
    let turns = quarter_turns % 4;
    let (ow, oh) = if turns % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    };
    out.clear();
    out.resize(width * height * bpp, 0);
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = match turns {
                1 => (y, width - 1 - x),
                2 => (width - 1 - x, height - 1 - y),
                3 => (height - 1 - y, x),
                _ => (x, y),
            };
            let s = (y * width + x) * bpp;
            let d = (dy * ow + dx) * bpp;
            out[d..d + bpp].copy_from_slice(&pixels[s..s + bpp]);
        }
    }
    (ow, oh)
}

/// Colours a frame: `out` gets opaque RGBA pixels.
pub fn to_rgba(format: PixelFormat, pixels: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(pixels.len() / format.bytes_per_pixel() * 4);
    match format {
        PixelFormat::Xrgb8888 => {
            for p in pixels.as_chunks::<4>().0 {
                // Little-endian 0x00RRGGBB: B, G, R, X in memory.
                out.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
        }
        PixelFormat::Rgb565 => {
            for p in pixels.as_chunks::<2>().0 {
                let v = u16::from_le_bytes(*p);
                let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
                out.extend_from_slice(&[
                    ((r << 3) | (r >> 2)) as u8,
                    ((g << 2) | (g >> 4)) as u8,
                    ((b << 3) | (b >> 2)) as u8,
                    255,
                ]);
            }
        }
        PixelFormat::Rgb1555 => {
            for p in pixels.as_chunks::<2>().0 {
                let v = u16::from_le_bytes(*p);
                let (r, g, b) = ((v >> 10) & 31, (v >> 5) & 31, v & 31);
                let x = |c: u16| ((c << 3) | (c >> 2)) as u8;
                out.extend_from_slice(&[x(r), x(g), x(b), 255]);
            }
        }
    }
}

/// One message to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Down(Pad),
    Up(Pad),
    /// Press for a few frames, then release (a button reached by a gesture
    /// that has no "up", or one the menu presses).
    Tap(Pad),
    /// Stop running the game (menu open, screen off), and go on.
    Pause,
    Resume,
    SaveState,
    LoadState,
    /// Save the game's memory and leave (the frontend exits).
    Quit,
}

impl Input {
    pub fn encode(self) -> [u8; 2] {
        match self {
            Self::Down(p) => [1, p as u8],
            Self::Up(p) => [2, p as u8],
            Self::Tap(p) => [3, p as u8],
            Self::Pause => [4, 0],
            Self::Resume => [5, 0],
            Self::SaveState => [6, 0],
            Self::LoadState => [7, 0],
            Self::Quit => [8, 0],
        }
    }

    pub fn decode(b: [u8; 2]) -> Option<Self> {
        let pad = || Pad::from_id(b[1]);
        Some(match b[0] {
            1 => Self::Down(pad()?),
            2 => Self::Up(pad()?),
            3 => Self::Tap(pad()?),
            4 => Self::Pause,
            5 => Self::Resume,
            6 => Self::SaveState,
            7 => Self::LoadState,
            8 => Self::Quit,
            _ => return None,
        })
    }
}

/// Frames a tapped button stays down (long enough for any game to see it).
pub const TAP_FRAMES: u8 = 4;

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// Converts the core's sample rate to the output's by linear
/// interpolation, keeping its place between blocks (game sound, where
/// this is plenty).
#[derive(Debug, Clone)]
pub struct Resampler {
    /// Input frames per output frame.
    step: f64,
    /// Position of the next output frame, in input frames after `last`.
    pos: f64,
    last: [i16; 2],
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from.max(1)) / f64::from(to.max(1)),
            pos: 0.,
            last: [0, 0],
        }
    }

    /// Stereo interleaved in, stereo interleaved out (appended to `out`).
    pub fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        let at = |k: usize, c: usize| -> f64 {
            if k == 0 {
                f64::from(self.last[c])
            } else {
                f64::from(input[(k - 1) * 2 + c])
            }
        };
        let mut t = self.pos;
        while t < frames as f64 {
            let i = t as usize;
            let frac = t - i as f64;
            for c in 0..2 {
                let v = at(i, c) + (at(i + 1, c) - at(i, c)) * frac;
                out.push(v.round().clamp(-32768., 32767.) as i16);
            }
            t += self.step;
        }
        self.pos = t - frames as f64;
        self.last = [input[(frames - 1) * 2], input[(frames - 1) * 2 + 1]];
    }
}

/// Game sound waiting for the output: bounded so a stall cannot build up
/// latency (the oldest is dropped).
#[derive(Debug, Default)]
pub struct AudioQueue {
    samples: VecDeque<i16>,
    max_frames: usize,
    keep_frames: usize,
    /// Frames thrown away to stay in time.
    pub dropped: u64,
}

impl AudioQueue {
    /// At most `max_frames` waiting; past that, trimmed to `keep_frames`.
    pub fn new(max_frames: usize, keep_frames: usize) -> Self {
        Self {
            samples: VecDeque::new(),
            max_frames,
            keep_frames: keep_frames.min(max_frames),
            dropped: 0,
        }
    }

    pub fn push(&mut self, samples: &[i16]) {
        self.samples.extend(samples.iter().copied());
        let frames = self.samples.len() / 2;
        if frames > self.max_frames {
            let drop = frames - self.keep_frames;
            self.samples.drain(..drop * 2);
            self.dropped += drop as u64;
        }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Up to `frames` frames, oldest first.
    pub fn pop(&mut self, frames: usize, out: &mut Vec<i16>) {
        let n = frames.min(self.frames()) * 2;
        out.extend(self.samples.drain(..n));
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

/// The libretro joypad, by its libretro button ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pad {
    B = 0,
    Y = 1,
    Select = 2,
    Start = 3,
    Up = 4,
    Down = 5,
    Left = 6,
    Right = 7,
    A = 8,
    X = 9,
    L = 10,
    R = 11,
}

impl Pad {
    pub const ALL: [Pad; 12] = [
        Self::B,
        Self::Y,
        Self::Select,
        Self::Start,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::A,
        Self::X,
        Self::L,
        Self::R,
    ];

    pub fn from_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|p| *p as u8 == id)
    }

    /// The bit for this button in a libretro joypad bitmask.
    pub fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// How a player's control reaches a pad button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Down while the control is down.
    Held(PhysicalControl),
    /// A short press. When the same control also has a [`Trigger::Hold`],
    /// it fires on release (if the hold did not); otherwise on press.
    Tap(PhysicalControl),
    /// The control held for [`HOLD_MS`]: a tap of the button then.
    Hold(PhysicalControl),
    /// A flick of the touch ring clockwise: one tap per flick.
    FlickClockwise,
    FlickCounterClockwise,
    /// A flick either way.
    Flick,
    /// A second flick straight after one (within [`DOUBLE_FLICK_MS`]),
    /// either way: a tap instead of that flick's own.
    DoubleFlick,
    /// The ring held as a steering wheel: each detent clockwise holds the
    /// button down for [`TURN_MS`] more (a fast spin, further).
    TurnClockwise,
    TurnCounterClockwise,
}

/// One row of a layout table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub trigger: Trigger,
    pub pad: Pad,
    /// The button's name on that system's controller ("A", "Start",
    /// "Thrust").
    pub name: &'static str,
}

/// A system's (or one game's) controls: the table, and the buttons the
/// in-game menu presses for the player (those the buttons do not reach).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub bindings: &'static [Binding],
    pub menu_buttons: &'static [(Pad, &'static str)],
}

/// A hold of the centre button this long is a hold (its second button).
pub const HOLD_MS: u64 = 500;
/// Ring movement after this much stillness (or turning the other way)
/// starts a new flick.
pub const FLICK_GAP_MS: u64 = 250;
/// A flick starting this soon after the last one ended is a double flick.
pub const DOUBLE_FLICK_MS: u64 = 400;
/// How long one detent of the ring holds a turning button down.
pub const TURN_MS: u64 = 60;
/// The most turning queued up at once.
pub const TURN_MAX_MS: u64 = 600;

const fn b(trigger: Trigger, pad: Pad, name: &'static str) -> Binding {
    Binding { trigger, pad, name }
}

use PhysicalControl as C;
use Trigger::{
    DoubleFlick, Flick, FlickClockwise, FlickCounterClockwise, Held, Hold, Tap, TurnClockwise,
    TurnCounterClockwise,
};

// The layout tables. The volume buttons and Power are on the right side,
// so a finger works them while the thumb is on the wheel:
// - the wheel's compass buttons are the D-pad (Menu up, Play down,
//   Previous left, Next right);
// - the two most used action buttons are the volume buttons, so they work
//   while moving: Volume up the primary (jump), Volume down the secondary
//   (run, fire);
// - the centre button has the rare ones: tapped, Start; held, Select or
//   Coin. Where the centre is a third action button (Genesis C, SNES A),
//   held it is Start;
// - Power opens Reborn's game menu (never a held D-pad button: games hold
//   up and down for seconds).
// Change a row here and the Controls page follows.

const NES: Layout = Layout {
    bindings: &[
        b(Held(C::Back), Pad::Up, "Up"),
        b(Held(C::PlayPause), Pad::Down, "Down"),
        b(Held(C::Previous), Pad::Left, "Left"),
        b(Held(C::Next), Pad::Right, "Right"),
        b(Held(C::VolumeUp), Pad::A, "A"),
        b(Held(C::VolumeDown), Pad::B, "B"),
        b(Tap(C::Select), Pad::Start, "Start"),
        b(Hold(C::Select), Pad::Select, "Select"),
    ],
    menu_buttons: &[],
};

const GAME_BOY: Layout = NES;

const GBA: Layout = Layout {
    bindings: &[
        b(Held(C::Back), Pad::Up, "Up"),
        b(Held(C::PlayPause), Pad::Down, "Down"),
        b(Held(C::Previous), Pad::Left, "Left"),
        b(Held(C::Next), Pad::Right, "Right"),
        b(Held(C::VolumeUp), Pad::A, "A"),
        b(Held(C::VolumeDown), Pad::B, "B"),
        b(Tap(C::Select), Pad::Start, "Start"),
        b(Hold(C::Select), Pad::Select, "Select"),
        b(FlickCounterClockwise, Pad::L, "L"),
        b(FlickClockwise, Pad::R, "R"),
    ],
    menu_buttons: &[],
};

// picodrive's pad: libretro B is Genesis B, Y is A, A is C. On the Master
// System and Game Gear, B is button 1, A (C here) button 2, Start pause.
const GENESIS: Layout = Layout {
    bindings: &[
        b(Held(C::Back), Pad::Up, "Up"),
        b(Held(C::PlayPause), Pad::Down, "Down"),
        b(Held(C::Previous), Pad::Left, "Left"),
        b(Held(C::Next), Pad::Right, "Right"),
        b(Held(C::VolumeUp), Pad::B, "B (button 1)"),
        b(Held(C::VolumeDown), Pad::Y, "A"),
        b(Tap(C::Select), Pad::A, "C (button 2)"),
        b(Hold(C::Select), Pad::Start, "Start"),
    ],
    menu_buttons: &[],
};

const SNES: Layout = Layout {
    bindings: &[
        b(Held(C::Back), Pad::Up, "Up"),
        b(Held(C::PlayPause), Pad::Down, "Down"),
        b(Held(C::Previous), Pad::Left, "Left"),
        b(Held(C::Next), Pad::Right, "Right"),
        b(Held(C::VolumeUp), Pad::B, "B"),
        b(Held(C::VolumeDown), Pad::Y, "Y"),
        b(Tap(C::Select), Pad::A, "A"),
        b(Hold(C::Select), Pad::Start, "Start"),
        b(FlickCounterClockwise, Pad::L, "L"),
        b(FlickClockwise, Pad::R, "R"),
        b(DoubleFlick, Pad::X, "X"),
    ],
    menu_buttons: &[(Pad::Select, "Select")],
};

// FBNeo's pad: libretro B is fire 1, A fire 2, Y fire 3, X fire 4; Select
// is the coin.
const ARCADE: Layout = Layout {
    bindings: &[
        b(Held(C::Back), Pad::Up, "Up"),
        b(Held(C::PlayPause), Pad::Down, "Down"),
        b(Held(C::Previous), Pad::Left, "Left"),
        b(Held(C::Next), Pad::Right, "Right"),
        b(Held(C::VolumeUp), Pad::B, "Fire 1"),
        b(Held(C::VolumeDown), Pad::A, "Fire 2"),
        b(Tap(C::Select), Pad::Start, "Start"),
        b(Hold(C::Select), Pad::Select, "Coin"),
        b(FlickCounterClockwise, Pad::Y, "Fire 3"),
        b(FlickClockwise, Pad::X, "Fire 4"),
    ],
    menu_buttons: &[],
};

// Asteroids: the ring steers the ship like a spinner. FBNeo: fire is fire
// 1 (B), thrust fire 2 (A), hyperspace (Deluxe: shield) fire 3 (Y).
const ASTEROIDS: Layout = Layout {
    bindings: &[
        b(TurnCounterClockwise, Pad::Left, "Turn left"),
        b(TurnClockwise, Pad::Right, "Turn right"),
        b(Held(C::Previous), Pad::Left, "Turn left"),
        b(Held(C::Next), Pad::Right, "Turn right"),
        b(Held(C::VolumeUp), Pad::A, "Thrust"),
        b(Held(C::VolumeDown), Pad::B, "Fire"),
        b(Tap(C::Select), Pad::Y, "Hyperspace"),
        b(Hold(C::Select), Pad::Select, "Coin"),
        b(Tap(C::PlayPause), Pad::Start, "Start"),
    ],
    menu_buttons: &[],
};

// Pac-Man: only the stick, a coin and start.
const PAC_MAN: Layout = Layout {
    bindings: &[
        b(Held(C::Back), Pad::Up, "Up"),
        b(Held(C::PlayPause), Pad::Down, "Down"),
        b(Held(C::Previous), Pad::Left, "Left"),
        b(Held(C::Next), Pad::Right, "Right"),
        b(Tap(C::Select), Pad::Start, "Start"),
        b(Hold(C::Select), Pad::Select, "Coin"),
    ],
    menu_buttons: &[],
};

/// Games with controls of their own: (system, file names without the
/// extension, the game's name, its layout).
pub const GAME_LAYOUTS: [(System, &[&str], &str, &Layout); 2] = [
    (
        System::Arcade,
        &[
            "asteroid",
            "asteroid1",
            "asteroid2",
            "astdelux",
            "astdelux1",
            "astdelux2",
        ],
        "Asteroids",
        &ASTEROIDS,
    ),
    (
        System::Arcade,
        &[
            "puckman", "pacman", "mspacman", "pacmanf", "mspacmnf", "puckmod",
        ],
        "Pac-Man",
        &PAC_MAN,
    ),
];

impl System {
    /// The system's own controls.
    pub fn layout(self) -> &'static Layout {
        match self {
            Self::Nes => &NES,
            Self::GameBoy => &GAME_BOY,
            Self::Gba => &GBA,
            Self::Genesis => &GENESIS,
            Self::Snes => &SNES,
            Self::Arcade => &ARCADE,
        }
    }
}

/// The controls for one game: its own when it has some (by file name),
/// else its system's.
pub fn layout_for(system: System, game: &Path) -> &'static Layout {
    game_layout(system, game).map_or(system.layout(), |(_, l)| l)
}

/// A game's own controls and its name, when it has some.
pub fn game_layout(system: System, game: &Path) -> Option<(&'static str, &'static Layout)> {
    let stem = game.file_stem()?.to_str()?.to_ascii_lowercase();
    GAME_LAYOUTS
        .iter()
        .find(|(s, names, _, _)| *s == system && names.contains(&stem.as_str()))
        .map(|(_, _, name, layout)| (*name, *layout))
}

/// A control's name as the player knows it.
pub fn control_name(c: PhysicalControl) -> &'static str {
    match c {
        C::Select => "Centre button",
        C::Back => "Menu (top)",
        C::Previous => "Previous (left)",
        C::Next => "Next (right)",
        C::PlayPause => "Play (bottom)",
        C::Power => "Power",
        C::VolumeUp => "Volume up",
        C::VolumeDown => "Volume down",
        C::WheelClockwise | C::WheelCounterClockwise => "Touch ring",
    }
}

/// How a binding is done, for the Controls page.
pub fn trigger_words(t: Trigger) -> String {
    match t {
        Held(c) => control_name(c).into(),
        Tap(c) => format!("Tap {}", control_name(c).to_lowercase()),
        Hold(c) => format!("Hold {}", control_name(c).to_lowercase()),
        FlickClockwise => "Flick the ring clockwise".into(),
        FlickCounterClockwise => "Flick the ring anticlockwise".into(),
        Flick => "Flick the ring".into(),
        DoubleFlick => "Flick the ring twice".into(),
        TurnClockwise => "Turn the ring clockwise".into(),
        TurnCounterClockwise => "Turn the ring anticlockwise".into(),
    }
}

/// The player's buttons as pad buttons for one layout. Time is in
/// milliseconds from any fixed start; [`Controls::tick`] fires holds and
/// ends turns.
#[derive(Debug, Clone)]
pub struct Controls {
    layout: &'static Layout,
    /// Controls down now: when pressed, and whether their hold has fired.
    pressed: Vec<(PhysicalControl, u64, bool)>,
    /// Pad buttons held down now, by a control.
    down: Vec<Pad>,
    /// The ring's current (or last) flick: its direction, when it last
    /// moved, and whether it was a double flick.
    ring: Option<(i8, u64, bool)>,
    /// A turn in progress: the button held and until when.
    turn: Option<(Pad, u64)>,
    /// Controls pressed during play whose release must not reach the menu
    /// shown after a pause.
    swallow: Vec<PhysicalControl>,
}

impl Default for Controls {
    fn default() -> Self {
        Self::new(&NES)
    }
}

impl Controls {
    pub fn new(layout: &'static Layout) -> Self {
        Self {
            layout,
            pressed: vec![],
            down: vec![],
            ring: None,
            turn: None,
            swallow: vec![],
        }
    }

    fn has(&self, f: impl Fn(Trigger) -> bool) -> bool {
        self.layout.bindings.iter().any(|b| f(b.trigger))
    }

    fn pads(&self, f: impl Fn(Trigger) -> bool) -> Vec<Pad> {
        self.layout
            .bindings
            .iter()
            .filter(|b| f(b.trigger))
            .map(|b| b.pad)
            .collect()
    }

    /// A pad button held by a control (not by a turn).
    fn held_by_control(&self, pad: Pad) -> bool {
        self.down.contains(&pad)
    }

    fn ring(&mut self, dir: i8, detents: u8, now: u64) -> Vec<Input> {
        let mut out = vec![];
        let turning = self
            .pads(|t| (dir > 0 && t == TurnClockwise) || (dir < 0 && t == TurnCounterClockwise));
        if let Some(&pad) = turning.first() {
            match self.turn {
                Some((p, until)) if p == pad => {
                    let until = until.max(now) + TURN_MS * u64::from(detents.max(1));
                    self.turn = Some((pad, until.min(now + TURN_MAX_MS)));
                }
                other => {
                    if let Some((p, _)) = other {
                        if !self.held_by_control(p) {
                            out.push(Input::Up(p));
                        }
                    }
                    if !self.held_by_control(pad) {
                        out.push(Input::Down(pad));
                    }
                    let ms = (TURN_MS * u64::from(detents.max(1))).min(TURN_MAX_MS);
                    self.turn = Some((pad, now + ms));
                }
            }
            return out;
        }
        let (new_flick, was_double) = match self.ring {
            Some((d, at, double)) => (d != dir || now.saturating_sub(at) >= FLICK_GAP_MS, double),
            None => (true, false),
        };
        if !new_flick {
            self.ring = Some((dir, now, was_double));
            return out;
        }
        // A double flick's end does not start another double.
        let previous_end = self
            .ring
            .filter(|(_, _, double)| !double)
            .map(|(_, at, _)| at);
        let double = previous_end.is_some_and(|end| now.saturating_sub(end) <= DOUBLE_FLICK_MS)
            && self.has(|t| t == DoubleFlick);
        let pads = if double {
            self.pads(|t| t == DoubleFlick)
        } else {
            self.pads(|t| {
                t == Flick
                    || (dir > 0 && t == FlickClockwise)
                    || (dir < 0 && t == FlickCounterClockwise)
            })
        };
        out.extend(pads.into_iter().map(Input::Tap));
        self.ring = Some((dir, now, double));
        out
    }

    /// The messages for one physical event; `None` when the event is not
    /// the game's (Power keeps its meaning: it opens the game menu).
    pub fn event(&mut self, input: NormalizedInput, now: u64) -> Option<Vec<Input>> {
        use NormalizedInput as N;
        let mut out = vec![];
        let control = match input {
            N::WheelClockwise(n) => return Some(self.ring(1, n, now)),
            N::WheelCounterClockwise(n) => return Some(self.ring(-1, n, now)),
            N::Press(c) | N::Release(c) | N::Cancel(c) | N::LongPress(c) | N::Repeat(c) => c,
        };
        if matches!(
            control,
            C::Power | C::WheelClockwise | C::WheelCounterClockwise
        ) {
            return None;
        }
        match input {
            N::Press(c) => {
                if self.pressed.iter().any(|p| p.0 == c) {
                    return Some(out);
                }
                self.pressed.push((c, now, false));
                for pad in self.pads(|t| t == Held(c)) {
                    if !self.down.contains(&pad) {
                        self.down.push(pad);
                        if self.turn.is_none_or(|(p, _)| p != pad) {
                            out.push(Input::Down(pad));
                        }
                    }
                }
                if !self.has(|t| t == Hold(c)) {
                    out.extend(self.pads(|t| t == Tap(c)).into_iter().map(Input::Tap));
                }
            }
            N::Release(c) | N::Cancel(c) => {
                let Some(i) = self.pressed.iter().position(|p| p.0 == c) else {
                    return Some(out); // pressed before the game had the buttons
                };
                let (_, _, fired) = self.pressed.remove(i);
                for pad in self.pads(|t| t == Held(c)) {
                    if let Some(i) = self.down.iter().position(|d| *d == pad) {
                        self.down.remove(i);
                        if self.turn.is_none_or(|(p, _)| p != pad) {
                            out.push(Input::Up(pad));
                        }
                    }
                }
                if matches!(input, N::Release(_)) && !fired && self.has(|t| t == Hold(c)) {
                    out.extend(self.pads(|t| t == Tap(c)).into_iter().map(Input::Tap));
                }
            }
            // The input layer's long presses and repeats: holds are timed
            // here instead.
            _ => {}
        }
        Some(out)
    }

    /// Holds that have come due, and turns that have run out.
    pub fn tick(&mut self, now: u64) -> Vec<Input> {
        let layout = self.layout;
        let mut out = vec![];
        for (c, at, fired) in &mut self.pressed {
            if *fired || now.saturating_sub(*at) < HOLD_MS {
                continue;
            }
            let pads = layout.bindings.iter().filter(|b| b.trigger == Hold(*c));
            for b in pads {
                *fired = true;
                out.push(Input::Tap(b.pad));
            }
        }
        if let Some((pad, until)) = self.turn {
            if now >= until {
                self.turn = None;
                if !self.held_by_control(pad) {
                    out.push(Input::Up(pad));
                }
            }
        }
        out
    }

    /// Lets go of everything (pausing, leaving); the releases still to come
    /// for controls down now are swallowed.
    pub fn release_all(&mut self) -> Vec<Input> {
        self.swallow
            .extend(self.pressed.drain(..).map(|(c, _, _)| c));
        self.ring = None;
        let mut out: Vec<Input> = self.down.drain(..).map(Input::Up).collect();
        if let Some((pad, _)) = self.turn.take() {
            if !out.contains(&Input::Up(pad)) {
                out.push(Input::Up(pad));
            }
        }
        out
    }

    /// True for the rest of a press made during play, after the game lost
    /// the buttons (so its release cannot choose a menu row).
    pub fn swallows(&mut self, input: NormalizedInput) -> bool {
        use NormalizedInput as N;
        match input {
            N::Release(c) | N::Cancel(c) => {
                if let Some(i) = self.swallow.iter().position(|s| *s == c) {
                    self.swallow.remove(i);
                    true
                } else {
                    false
                }
            }
            N::LongPress(c) | N::Repeat(c) => self.swallow.contains(&c),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// The picture on screen
// ---------------------------------------------------------------------------

pub const SCREEN_W: f32 = 480.;
pub const SCREEN_H: f32 = 360.;

/// Where a frame goes and how it is sampled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Whole-pixel scaling: sharp (nearest) sampling. Other scales are
    /// sampled smoothly, so every source pixel keeps the same width.
    pub nearest: bool,
}

/// The scale for a `w` x `h` frame: the largest whole scale when that is
/// at least 2x; a vertical (portrait) picture the full screen height; else
/// 1.5x when it fits (NES, SNES, Genesis); else 1x; else
/// (larger than the screen) whatever fits. High-resolution modes (a SNES
/// 512-wide frame) are drawn at the size of their low-resolution
/// equivalent.
pub fn fit(w: u32, h: u32) -> Fit {
    let (mut ew, mut eh) = (w.max(1) as f32, h.max(1) as f32);
    if ew > SCREEN_W && ew >= 2. * eh {
        ew /= 2.;
    }
    if eh > SCREEN_H && ew > SCREEN_W / 2. {
        eh /= 2.;
        if ew > SCREEN_W {
            ew /= 2.;
        }
    }
    let k = (SCREEN_W / ew).min(SCREEN_H / eh);
    let whole = k.floor();
    let half = (k * 2.).floor() / 2.;
    let scale = if whole >= 2. {
        whole
    } else if eh > ew && k > 1. {
        k // a vertical game: the full height of the screen
    } else if half >= 1. {
        half
    } else {
        k
    };
    let (dw, dh) = ((ew * scale).round(), (eh * scale).round());
    Fit {
        x: ((SCREEN_W - dw) / 2.).floor(),
        y: ((SCREEN_H - dh) / 2.).floor(),
        w: dw,
        h: dh,
        nearest: scale.fract() == 0. && ew == w as f32 && eh == h as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use NormalizedInput as N;

    #[test]
    fn roms_are_found_by_extension_and_titled_without_tags() {
        let r = |s, p: &str| rom(s, PathBuf::from(p));
        let smb = r(System::Nes, "/sd/Games/NES/Super Game (USA) [!].nes").unwrap();
        assert_eq!(smb.title, "Super Game");
        assert_eq!(smb.tags, "USA", "dump notes left out");
        assert!(
            r(System::Nes, "/sd/Games/NES/game.NES").is_some(),
            "any case"
        );
        assert!(r(System::Nes, "/sd/Games/NES/notes.txt").is_none());
        assert!(
            r(System::Nes, "/sd/Games/NES/._game.nes").is_none(),
            "sidecar"
        );
        assert!(r(System::GameBoy, "/x/a.gbc").is_some());
        assert!(r(System::GameBoy, "/x/a.gb").is_some());
        for ext in ["md", "gen", "bin", "smd", "sms", "gg"] {
            assert!(
                r(System::Genesis, &format!("/x/a.{ext}")).is_some(),
                "{ext}"
            );
        }
        assert!(r(System::Snes, "/x/a.sfc").is_some() && r(System::Snes, "/x/a.smc").is_some());
        assert!(r(System::Arcade, "/x/sf2.zip").is_some());
        assert!(r(System::Arcade, "/x/neogeo.zip").is_none(), "a BIOS");
        assert!(r(System::Nes, "/x/a.zip").is_none(), "zips only for arcade");
        assert_eq!(
            r(System::Snes, "/x/(Beta).sfc").unwrap().title,
            "(Beta)",
            "all tags: the file name"
        );
    }

    #[test]
    fn each_rom_once_in_title_order_and_save_names() {
        let r = |p: &str| rom(System::Nes, PathBuf::from(p)).unwrap();
        let list = roms(vec![
            r("/sd/Games/NES/zelda.nes"),
            r("/sd/Games/NES/Alpha (Europe).nes"),
            r("/sd/Games/Famicom/zelda.nes"),
            r("/sd/Games/NES/alpha (USA).nes"),
        ]);
        let names: Vec<(&str, &str)> = list
            .iter()
            .map(|r| (r.title.as_str(), r.tags.as_str()))
            .collect();
        assert_eq!(
            names,
            [("Alpha", "Europe"), ("alpha", "USA"), ("zelda", "")]
        );
        assert_eq!(list[2].path, PathBuf::from("/sd/Games/NES/zelda.nes"));
        assert_eq!(save_stem(Path::new("/sd/x/Game (USA).nes")), "Game (USA)");
        assert_eq!(save_stem(Path::new("/")), "game");
    }

    #[test]
    fn every_system_has_a_core_folder_and_complete_dpad() {
        for s in System::ALL {
            assert_eq!(System::from_key(s.key()), Some(s));
            assert!(!s.folders().is_empty() && !s.extensions().is_empty());
            assert!(s.where_to_put().contains("Games/"));
            let pads: Vec<Pad> = s.layout().bindings.iter().map(|b| b.pad).collect();
            for p in [Pad::Up, Pad::Down, Pad::Left, Pad::Right] {
                assert!(pads.contains(&p), "{s:?} {p:?}");
            }
            // Start is reachable: a binding or the menu.
            assert!(
                pads.contains(&Pad::Start)
                    || s.layout()
                        .menu_buttons
                        .iter()
                        .any(|(p, _)| *p == Pad::Start),
                "{s:?} start"
            );
            // No control is both held and tapped (a tap would wait for
            // its release).
            for b in s.layout().bindings {
                if let Trigger::Tap(c) = b.trigger {
                    assert!(
                        !s.layout().bindings.iter().any(|o| o.trigger == Held(c)),
                        "{s:?}: {c:?} held and tapped"
                    );
                }
            }
        }
        assert_eq!(System::Genesis.core(), "picodrive");
        assert!(System::Genesis.extensions().contains(&"sms"));
        assert!(System::Genesis.extensions().contains(&"gg"));
    }

    fn message_bytes(kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut m = header(kind, flags, payload.len()).to_vec();
        m.extend_from_slice(payload);
        m
    }

    #[test]
    fn messages_round_trip_whatever_the_read_sizes() {
        let info = Message::Info(Info {
            fps_milli: 60_099,
            core_rate: 32_040,
            base_width: 256,
            base_height: 224,
        });
        let frame = Message::Frame {
            width: 3,
            height: 2,
            format: PixelFormat::Rgb565,
            pixels: (0..12).collect(),
        };
        let big = Message::Frame {
            width: 320,
            height: 224,
            format: PixelFormat::Xrgb8888,
            pixels: (0..320 * 224 * 4).map(|i| (i % 253) as u8).collect(),
        };
        let audio = Message::Audio(vec![1, -1, 32767, -32768]);
        let status = Message::Status(StatusKind::StateSaved, "Saved".into());
        let all = [info, frame, big, audio, status];
        let stream: Vec<u8> = all.iter().flat_map(|m| m.encode()).collect();
        for chunk in [1, 5, 8, 9, 4096, stream.len()] {
            let mut d = Decoder::default();
            let mut got = vec![];
            for piece in stream.chunks(chunk) {
                d.push(piece);
                while let Some(m) = d.next_message().unwrap() {
                    got.push(m);
                }
            }
            assert_eq!(got, all, "chunk {chunk}");
            assert!(d.buf.len() < 2 * (320 * 224 * 4 + 16), "buffer stays small");
        }
        // The frame header alone, then pixels (how the frontend sends).
        let mut out = vec![];
        encode_frame_header(&mut out, 3, 2, PixelFormat::Rgb565);
        out.extend(0..12u8);
        assert_eq!(out, all[1].encode());
    }

    #[test]
    fn broken_streams_are_errors() {
        let mut d = Decoder::default();
        d.push(b"XX\0\0\0\0\0\0");
        assert_eq!(d.next_message(), Err(ProtocolError::BadMagic));
        let mut d = Decoder::default();
        d.push(&message_bytes(b'Q', 0, &[]));
        assert_eq!(d.next_message(), Err(ProtocolError::UnknownKind(b'Q')));
        let mut d = Decoder::default();
        d.push(&message_bytes(b'A', 0, &[0; 3]));
        assert!(matches!(
            d.next_message(),
            Err(ProtocolError::BadLength { .. })
        ));
        // Pixels that do not match the size given.
        let mut d = Decoder::default();
        let mut payload = vec![4, 0, 4, 0, 2, 0, 0, 0];
        payload.extend([0; 10]);
        d.push(&message_bytes(b'F', 0, &payload));
        assert!(matches!(
            d.next_message(),
            Err(ProtocolError::BadLength { .. })
        ));
        let mut d = Decoder::default();
        let mut payload = vec![0, 0, 4, 0, 2, 0, 0, 0];
        payload.extend([0; 0]);
        d.push(&message_bytes(b'F', 0, &payload));
        assert_eq!(d.next_message(), Err(ProtocolError::BadFrame));
    }

    #[test]
    fn pixels_of_every_format_become_rgba() {
        let mut out = vec![];
        // RGB565: pure red, pure green, white.
        to_rgba(
            PixelFormat::Rgb565,
            &[0x00, 0xF8, 0xE0, 0x07, 0xFF, 0xFF],
            &mut out,
        );
        assert_eq!(out, [255, 0, 0, 255, 0, 255, 0, 255, 255, 255, 255, 255]);
        // XRGB8888 0x00123456.
        to_rgba(PixelFormat::Xrgb8888, &[0x56, 0x34, 0x12, 0xFF], &mut out);
        assert_eq!(out, [0x12, 0x34, 0x56, 255]);
        // 0RGB1555: blue.
        to_rgba(PixelFormat::Rgb1555, &[0x1F, 0x00], &mut out);
        assert_eq!(out, [0, 0, 255, 255]);
        assert_eq!(PixelFormat::from_libretro(2), Some(PixelFormat::Rgb565));
        assert_eq!(PixelFormat::from_libretro(3), None);
    }

    #[test]
    fn inputs_encode_and_decode() {
        for input in [
            Input::Down(Pad::A),
            Input::Up(Pad::Left),
            Input::Tap(Pad::Start),
            Input::Pause,
            Input::Resume,
            Input::SaveState,
            Input::LoadState,
            Input::Quit,
        ] {
            assert_eq!(Input::decode(input.encode()), Some(input));
        }
        assert_eq!(Input::Down(Pad::A).encode(), [1, 8]);
        assert_eq!(Input::decode([1, 99]), None);
        assert_eq!(Input::decode([0, 0]), None);
        assert_eq!(Pad::R.bit(), 1 << 11);
    }

    #[test]
    fn resampling_keeps_time_and_level() {
        // 32040 Hz (SNES) to 48000: over a second, a second's worth out.
        let mut r = Resampler::new(32_040, 48_000);
        let mut out = vec![];
        for _ in 0..60 {
            r.process(&[1000i16; 534 * 2], &mut out);
        }
        let frames = out.len() / 2;
        let expected = 534. * 60. * 48_000. / 32_040.;
        assert!(
            (frames as f64 - expected).abs() <= 2.,
            "{frames} {expected}"
        );
        assert!(
            out[100..].iter().all(|&s| s == 1000),
            "a steady level stays"
        );
        // Same rate: the input, one frame late.
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = vec![];
        r.process(&[1, 2, 3, 4, 5, 6], &mut out);
        assert_eq!(out, [0, 0, 1, 2, 3, 4]);
        r.process(&[7, 8], &mut out);
        assert_eq!(&out[6..], [5, 6]);
        r.process(&[], &mut out);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn the_audio_queue_drops_the_oldest_to_stay_in_time() {
        let mut q = AudioQueue::new(4, 2);
        q.push(&[1, 1, 2, 2, 3, 3]);
        assert_eq!(q.frames(), 3);
        q.push(&[4, 4, 5, 5]);
        assert_eq!(q.frames(), 2, "trimmed to the newest");
        assert_eq!(q.dropped, 3);
        let mut out = vec![];
        q.pop(10, &mut out);
        assert_eq!(out, [4, 4, 5, 5]);
        assert_eq!(q.frames(), 0);
    }

    fn controls(s: System) -> Controls {
        Controls::new(s.layout())
    }

    #[test]
    fn the_dpad_and_the_volume_buttons_are_held() {
        let mut c = controls(System::Nes);
        assert_eq!(
            c.event(N::Press(C::Back), 0),
            Some(vec![Input::Down(Pad::Up)])
        );
        assert_eq!(c.event(N::LongPress(C::Back), 700), Some(vec![]));
        assert_eq!(c.event(N::Repeat(C::Back), 800), Some(vec![]));
        assert!(c.tick(5_000).is_empty(), "holding up is just up");
        assert_eq!(
            c.event(N::Press(C::VolumeUp), 10),
            Some(vec![Input::Down(Pad::A)])
        );
        assert_eq!(
            c.event(N::Press(C::VolumeDown), 10),
            Some(vec![Input::Down(Pad::B)])
        );
        assert_eq!(c.event(N::Repeat(C::VolumeDown), 900), Some(vec![]));
        assert_eq!(
            c.event(N::Release(C::Back), 900),
            Some(vec![Input::Up(Pad::Up)])
        );
        assert_eq!(
            c.event(N::Cancel(C::VolumeUp), 900),
            Some(vec![Input::Up(Pad::A)])
        );
        assert_eq!(c.event(N::Release(C::VolumeUp), 901), Some(vec![]));
        assert!(
            c.event(N::Press(C::Power), 0).is_none(),
            "Power is Reborn's"
        );
        assert!(c.event(N::Release(C::Power), 0).is_none());
    }

    #[test]
    fn the_centre_is_start_tapped_and_select_held() {
        let mut c = controls(System::Nes);
        assert_eq!(c.event(N::Press(C::Select), 1_000), Some(vec![]));
        assert!(c.tick(1_300).is_empty());
        assert_eq!(
            c.event(N::Release(C::Select), 1_350),
            Some(vec![Input::Tap(Pad::Start)])
        );
        c.event(N::Press(C::Select), 2_000);
        assert_eq!(c.tick(2_000 + HOLD_MS), [Input::Tap(Pad::Select)]);
        assert!(c.tick(3_000).is_empty(), "once per hold");
        assert_eq!(
            c.event(N::Release(C::Select), 3_100),
            Some(vec![]),
            "no Start after the hold"
        );
        // A release without its press (pressed in the menu) does nothing.
        assert_eq!(c.event(N::Release(C::Select), 4_000), Some(vec![]));
    }

    #[test]
    fn genesis_and_snes_put_a_third_button_on_the_centre() {
        let mut g = controls(System::Genesis);
        assert_eq!(
            g.event(N::Press(C::VolumeUp), 0),
            Some(vec![Input::Down(Pad::B)])
        );
        g.event(N::Press(C::Select), 0);
        assert_eq!(
            g.event(N::Release(C::Select), 100),
            Some(vec![Input::Tap(Pad::A)]),
            "C"
        );
        g.event(N::Press(C::Select), 200);
        assert_eq!(g.tick(200 + HOLD_MS), [Input::Tap(Pad::Start)]);
        let mut s = controls(System::Snes);
        assert_eq!(
            s.event(N::Press(C::VolumeUp), 0),
            Some(vec![Input::Down(Pad::B)]),
            "B jumps"
        );
        assert_eq!(
            s.event(N::Press(C::VolumeDown), 0),
            Some(vec![Input::Down(Pad::Y)]),
            "Y runs"
        );
        s.event(N::Press(C::Select), 0);
        assert_eq!(s.tick(HOLD_MS), [Input::Tap(Pad::Start)]);
        assert_eq!(
            System::Snes.layout().menu_buttons,
            [(Pad::Select, "Select")]
        );
    }

    #[test]
    fn ring_flicks_tap_once_per_flick() {
        let mut c = controls(System::Gba);
        assert_eq!(
            c.event(N::WheelClockwise(1), 0),
            Some(vec![Input::Tap(Pad::R)])
        );
        assert_eq!(
            c.event(N::WheelClockwise(3), 80),
            Some(vec![]),
            "same flick"
        );
        assert_eq!(c.event(N::WheelClockwise(1), 200), Some(vec![]));
        assert_eq!(
            c.event(N::WheelClockwise(1), 200 + FLICK_GAP_MS),
            Some(vec![Input::Tap(Pad::R)]),
            "after a pause, a new flick"
        );
        assert_eq!(
            c.event(N::WheelCounterClockwise(1), 500),
            Some(vec![Input::Tap(Pad::L)]),
            "turning back is a new flick"
        );
        // NES has nothing on the ring: still the game's (not volume).
        let mut n = controls(System::Nes);
        assert_eq!(n.event(N::WheelClockwise(1), 0), Some(vec![]));
    }

    #[test]
    fn a_snes_double_flick_is_x() {
        let mut c = controls(System::Snes);
        assert_eq!(
            c.event(N::WheelClockwise(1), 0),
            Some(vec![Input::Tap(Pad::R)])
        );
        c.event(N::WheelClockwise(1), 50);
        // Still, then a second flick soon after: X.
        assert_eq!(
            c.event(N::WheelCounterClockwise(1), 50 + FLICK_GAP_MS),
            Some(vec![Input::Tap(Pad::X)])
        );
        assert_eq!(
            c.event(N::WheelCounterClockwise(2), 80 + FLICK_GAP_MS),
            Some(vec![]),
            "the rest of that flick"
        );
        // A third flick straight after is a plain flick again.
        assert_eq!(
            c.event(N::WheelClockwise(1), 400 + FLICK_GAP_MS),
            Some(vec![Input::Tap(Pad::R)])
        );
        // Long after: a plain flick.
        assert_eq!(
            c.event(N::WheelCounterClockwise(1), 5_000),
            Some(vec![Input::Tap(Pad::L)])
        );
    }

    #[test]
    fn asteroids_steers_with_the_ring() {
        let game = Path::new("/sd/Games/Arcade/asteroid.zip");
        let layout = layout_for(System::Arcade, game);
        assert_eq!(game_layout(System::Arcade, game).unwrap().0, "Asteroids");
        assert_eq!(
            layout_for(System::Arcade, Path::new("/x/sf2.zip")),
            System::Arcade.layout()
        );
        assert_eq!(
            layout_for(System::Nes, game),
            System::Nes.layout(),
            "by system too"
        );
        let mut c = Controls::new(layout);
        assert_eq!(
            c.event(N::WheelClockwise(1), 0),
            Some(vec![Input::Down(Pad::Right)])
        );
        assert_eq!(c.event(N::WheelClockwise(2), 20), Some(vec![]), "more turn");
        assert!(c.tick(20 + 2 * TURN_MS).is_empty(), "still turning");
        assert_eq!(c.tick(3 * TURN_MS), [Input::Up(Pad::Right)]);
        // A fast spin turns further, up to a limit.
        c.event(N::WheelCounterClockwise(50), 1_000);
        assert!(c.tick(1_000 + TURN_MAX_MS - 1).is_empty());
        assert_eq!(c.tick(1_000 + TURN_MAX_MS), [Input::Up(Pad::Left)]);
        // Turning back lets go of the other way at once.
        c.event(N::WheelCounterClockwise(1), 2_000);
        assert_eq!(
            c.event(N::WheelClockwise(1), 2_010),
            Some(vec![Input::Up(Pad::Left), Input::Down(Pad::Right)])
        );
        assert_eq!(c.release_all(), [Input::Up(Pad::Right)]);
        // Thrust, fire, hyperspace, coin, start.
        assert_eq!(
            c.event(N::Press(C::VolumeUp), 3_000),
            Some(vec![Input::Down(Pad::A)])
        );
        c.event(N::Press(C::Select), 3_000);
        assert_eq!(
            c.event(N::Release(C::Select), 3_100),
            Some(vec![Input::Tap(Pad::Y)])
        );
        assert_eq!(
            c.event(N::Press(C::PlayPause), 3_200),
            Some(vec![Input::Tap(Pad::Start)])
        );
        let pac = layout_for(System::Arcade, Path::new("/x/MSPACMAN.zip"));
        assert_eq!(
            game_layout(System::Arcade, Path::new("/x/puckman.zip"))
                .unwrap()
                .0,
            "Pac-Man"
        );
        assert!(pac
            .bindings
            .iter()
            .all(|b| !matches!(b.trigger, Trigger::Held(C::VolumeUp | C::VolumeDown))));
    }

    #[test]
    fn pausing_releases_everything_and_swallows_the_rest_of_the_press() {
        let mut c = controls(System::Nes);
        c.event(N::Press(C::Next), 0);
        c.event(N::Press(C::VolumeUp), 0);
        c.event(N::Press(C::Select), 0);
        assert_eq!(c.release_all(), [Input::Up(Pad::Right), Input::Up(Pad::A)]);
        assert!(c.swallows(N::LongPress(C::Select)));
        assert!(c.swallows(N::Release(C::Select)), "not a menu Select");
        assert!(!c.swallows(N::Release(C::Select)), "only that press");
        assert!(c.swallows(N::Cancel(C::Next)));
        assert!(!c.swallows(N::Release(C::Back)));
        assert_eq!(
            c.event(N::Release(C::Next), 10),
            Some(vec![]),
            "nothing left down"
        );
    }

    #[test]
    fn pictures_turn_anticlockwise_by_quarter_turns() {
        // 3x2, one byte a pixel:
        //   a b c
        //   d e f
        let p = b"abcdef";
        let mut out = vec![];
        assert_eq!(rotate(p, 3, 2, 1, 0, &mut out), (3, 2));
        assert_eq!(out, b"abcdef");
        // A quarter turn anticlockwise: the right column becomes the top row.
        assert_eq!(rotate(p, 3, 2, 1, 1, &mut out), (2, 3));
        assert_eq!(out, b"cfbead");
        assert_eq!(rotate(p, 3, 2, 1, 2, &mut out), (3, 2));
        assert_eq!(out, b"fedcba");
        assert_eq!(rotate(p, 3, 2, 1, 3, &mut out), (2, 3));
        assert_eq!(out, b"daebfc");
        // Whole pixels move together.
        assert_eq!(rotate(b"AaBb", 2, 1, 2, 1, &mut out), (1, 2));
        assert_eq!(out, b"BbAa");
    }

    #[test]
    fn frames_fit_at_the_scales_that_read_best() {
        let f = |w, h| {
            let f = fit(w, h);
            (f.x, f.y, f.w, f.h, f.nearest)
        };
        assert_eq!(f(256, 240), (48., 0., 384., 360., false), "NES 1.5x");
        assert_eq!(f(160, 144), (80., 36., 320., 288., true), "GB 2x");
        assert_eq!(f(240, 160), (0., 20., 480., 320., true), "GBA 2x");
        assert_eq!(f(320, 224), (0., 12., 480., 336., false), "Genesis 1.5x");
        assert_eq!(f(256, 224), (48., 12., 384., 336., false), "SNES 1.5x");
        assert_eq!(f(512, 224), (48., 12., 384., 336., false), "SNES hi-res");
        assert_eq!(
            f(512, 448),
            (48., 12., 384., 336., false),
            "SNES interlaced"
        );
        assert_eq!(f(384, 224), (48., 68., 384., 224., true), "arcade 1x");
        assert_eq!(f(256, 192), (48., 36., 384., 288., false), "Master System");
        assert_eq!(
            f(224, 288),
            (100., 0., 280., 360., false),
            "Pac-Man, upright"
        );
        for (w, h) in [(1024, 1024), (1, 1), (700, 300), (224, 288)] {
            let r = fit(w, h);
            assert!(r.x >= 0. && r.y >= 0. && r.x + r.w <= 480. && r.y + r.h <= 360.);
        }
    }

    #[test]
    fn the_frontend_error_line_is_the_one_shown() {
        let tail: Vec<String> = [
            "[libretro] loading",
            "reborn-retro: the core could not load this game",
            "",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            failure_line(&tail).as_deref(),
            Some("the core could not load this game")
        );
        assert_eq!(failure_line(&["x".into()]).as_deref(), Some("x"));
        assert_eq!(failure_line(&[]), None);
    }
}

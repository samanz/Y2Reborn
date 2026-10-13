//! Doom (Extras > Games): the pipe protocol spoken with the `reborn-doom`
//! engine process, the player's buttons as Doom keys, and which WAD files
//! are games.
//!
//! The engine (doomgeneric, GPL) is a separate executable installed by the
//! system (Y2Linux's `reborn-doom` package), not part of Reborn; Reborn
//! starts it with an IWAD and only exchanges bytes with it. Its stdout
//! carries messages of an 8-byte header ('D', 'G', kind, flags, length as a
//! little-endian u32) and a payload: kind 'P' is the palette (768 bytes of
//! RGB, sent when it changes), kind 'F' a frame (320x200 palette indices).
//! Its stdin takes 2-byte messages ([`Input::encode`]). The other side is
//! that package's platform layer, `doomgeneric_y2.c`; keep the two in step.
use crate::{NormalizedInput, PhysicalControl};
use std::path::PathBuf;

pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 200;
pub const FRAME_BYTES: usize = WIDTH * HEIGHT;
pub const PALETTE_BYTES: usize = 768;
const HEADER_BYTES: usize = 8;
/// Header flag: Doom's own menu is open.
pub const FLAG_MENU: u8 = 1;

/// One message from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// 256 RGB triples.
    Palette(Vec<u8>),
    /// 320x200 palette indices, row by row; `menu` when Doom's menu shows.
    Frame { pixels: Vec<u8>, menu: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// The stream is out of step (not a 'D' 'G' header).
    BadMagic,
    UnknownKind(u8),
    /// A palette or frame of the wrong size.
    BadLength {
        kind: u8,
        length: u32,
    },
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "frame stream out of step"),
            Self::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            Self::BadLength { kind, length } => {
                write!(f, "message {} of {length} bytes", *kind as char)
            }
        }
    }
}

/// Splits the engine's output into messages, whatever sizes the reads come
/// in. A protocol error is final: the stream cannot be trusted after it.
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

    /// The next whole message, if one has arrived.
    pub fn next_message(&mut self) -> Result<Option<Message>, ProtocolError> {
        let rest = &self.buf[self.at..];
        if rest.len() < HEADER_BYTES {
            return Ok(None);
        }
        if &rest[..2] != b"DG" {
            return Err(ProtocolError::BadMagic);
        }
        let (kind, flags) = (rest[2], rest[3]);
        let length = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
        let expected = match kind {
            b'P' => PALETTE_BYTES,
            b'F' => FRAME_BYTES,
            other => return Err(ProtocolError::UnknownKind(other)),
        };
        if length as usize != expected {
            return Err(ProtocolError::BadLength { kind, length });
        }
        if rest.len() < HEADER_BYTES + expected {
            return Ok(None);
        }
        let payload = rest[HEADER_BYTES..HEADER_BYTES + expected].to_vec();
        self.at += HEADER_BYTES + expected;
        // Keep the buffer from growing: drop what has been read once it is
        // the larger part.
        if self.at > self.buf.len() / 2 {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        Ok(Some(match kind {
            b'P' => Message::Palette(payload),
            _ => Message::Frame {
                pixels: payload,
                menu: flags & FLAG_MENU != 0,
            },
        }))
    }
}

/// Colours a frame: `out` gets 320x200 opaque RGBA pixels.
pub fn to_rgba(pixels: &[u8], palette: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(pixels.len() * 4);
    for &p in pixels {
        let i = usize::from(p) * 3;
        match palette.get(i..i + 3) {
            Some(rgb) => out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]),
            None => out.extend_from_slice(&[0, 0, 0, 255]),
        }
    }
}

/// Doom key codes (doomkeys.h) Reborn sends.
pub mod key {
    pub const RIGHT: u8 = 0xae;
    pub const LEFT: u8 = 0xac;
    pub const UP: u8 = 0xad;
    pub const DOWN: u8 = 0xaf;
    pub const USE: u8 = 0xa2;
    pub const FIRE: u8 = 0xa3;
    pub const ESCAPE: u8 = 27;
    pub const ENTER: u8 = 13;
    pub const TAB: u8 = 9;
}

/// One message to the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Down(u8),
    Up(u8),
    /// Wheel detents, clockwise positive: turning in play, moving through
    /// a menu while one is open (the engine knows which).
    Turn(i8),
    /// Next (+1) or previous (-1) weapon.
    Weapon(i8),
    /// Stop the game clock (the screen went off), and start it again.
    Pause,
    Resume,
    /// Leave the game (the engine exits).
    Quit,
}

impl Input {
    pub fn encode(self) -> [u8; 2] {
        match self {
            Self::Down(k) => [1, k],
            Self::Up(k) => [2, k],
            Self::Turn(n) => [3, n as u8],
            Self::Weapon(n) => [4, n as u8],
            Self::Pause => [5, 0],
            Self::Resume => [6, 0],
            Self::Quit => [7, 0],
        }
    }
}

/// What a button does while Doom is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Out {
    Send(Input),
    /// Back held: leave Doom for its list of games.
    Leave,
}

/// Most detents sent as one turn (a fast spin).
const MAX_TURN: u8 = 16;

/// The player's buttons as Doom keys. Held buttons are held keys: Next
/// walks forward, Previous backs up, Select fires, Play/Pause uses (opens
/// doors, presses switches). The wheel turns; the volume buttons change
/// weapon. Back opens and closes Doom's menu; held, it leaves Doom; held
/// Play/Pause shows the map. In Doom's menus the engine reads the wheel as
/// up and down, Select as Enter and Back as "go back".
///
/// Power is not Doom's: [`Controls::event`] returns `None` for it.
#[derive(Debug, Default, Clone)]
pub struct Controls {
    /// Keys down now, so they can all be released (pause, leaving).
    held: Vec<u8>,
    /// Back was held to leave: its release (after Doom has gone) is
    /// swallowed instead of reaching the screen behind.
    leaving: Option<PhysicalControl>,
    back_long: bool,
}

fn held_key(control: PhysicalControl) -> Option<u8> {
    Some(match control {
        PhysicalControl::Next => key::UP,
        PhysicalControl::Previous => key::DOWN,
        PhysicalControl::Select => key::FIRE,
        PhysicalControl::PlayPause => key::USE,
        _ => return None,
    })
}

impl Controls {
    /// The messages for one physical event; `None` when the event is not
    /// Doom's (Power keeps its usual meaning).
    pub fn event(&mut self, input: NormalizedInput) -> Option<Vec<Out>> {
        use NormalizedInput as N;
        use PhysicalControl as C;
        let control = match input {
            N::WheelClockwise(n) => {
                return Some(vec![Out::Send(Input::Turn(n.clamp(1, MAX_TURN) as i8))])
            }
            N::WheelCounterClockwise(n) => {
                return Some(vec![Out::Send(Input::Turn(-(n.clamp(1, MAX_TURN) as i8)))])
            }
            N::Press(c) | N::Release(c) | N::Cancel(c) | N::LongPress(c) | N::Repeat(c) => c,
        };
        if matches!(
            control,
            C::Power | C::WheelClockwise | C::WheelCounterClockwise
        ) {
            return None;
        }
        let tap = |k| vec![Out::Send(Input::Down(k)), Out::Send(Input::Up(k))];
        Some(match (input, control) {
            (N::Press(C::VolumeUp), _) => vec![Out::Send(Input::Weapon(1))],
            (N::Press(C::VolumeDown), _) => vec![Out::Send(Input::Weapon(-1))],
            (N::Press(C::Back), _) => {
                self.back_long = false;
                vec![]
            }
            (N::LongPress(C::Back), _) => {
                self.back_long = true;
                self.leaving = Some(C::Back);
                let mut out: Vec<Out> = self.release_all().into_iter().map(Out::Send).collect();
                out.push(Out::Leave);
                out
            }
            (N::Release(C::Back), _) if !self.back_long => tap(key::ESCAPE),
            (N::LongPress(C::PlayPause), _) => tap(key::TAB),
            (N::Press(c), _) => match held_key(c) {
                Some(k) if !self.held.contains(&k) => {
                    self.held.push(k);
                    vec![Out::Send(Input::Down(k))]
                }
                _ => vec![],
            },
            (N::Release(c) | N::Cancel(c), _) => match held_key(c) {
                Some(k) if self.held.contains(&k) => {
                    self.held.retain(|&h| h != k);
                    vec![Out::Send(Input::Up(k))]
                }
                _ => vec![],
            },
            _ => vec![],
        })
    }

    /// Releases for every key held (before pausing or leaving).
    pub fn release_all(&mut self) -> Vec<Input> {
        self.held.drain(..).map(Input::Up).collect()
    }

    /// After leaving: true for the rest of the press that left (its
    /// release must not act on the screen behind).
    pub fn swallows(&mut self, input: NormalizedInput) -> bool {
        let Some(control) = self.leaving else {
            return false;
        };
        match input {
            NormalizedInput::Release(c) | NormalizedInput::Cancel(c) if c == control => {
                self.leaving = None;
                true
            }
            NormalizedInput::Repeat(c) | NormalizedInput::LongPress(c) => c == control,
            _ => false,
        }
    }
}

/// An IWAD the player can start.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wad {
    pub title: String,
    pub path: PathBuf,
}

/// The folder on a card (or internal storage) that holds WAD files.
pub const CARD_FOLDER: &str = "Doom";

/// Games the engine names by their file (doomgeneric's own list).
const KNOWN: [(&str, &str); 10] = [
    ("doom.wad", "Doom"),
    ("doom1.wad", "Doom (shareware)"),
    ("doom2.wad", "Doom II"),
    ("plutonia.wad", "Final Doom: The Plutonia Experiment"),
    ("tnt.wad", "Final Doom: TNT: Evilution"),
    ("freedoom1.wad", "Freedoom: Phase 1"),
    ("freedoom2.wad", "Freedoom: Phase 2"),
    ("freedm.wad", "FreeDM"),
    ("chex.wad", "Chex Quest"),
    ("hacx.wad", "Hacx"),
];

/// IWADs of other games on the same engine family, which this one cannot
/// play.
const OTHER_GAMES: [&str; 5] = [
    "heretic.wad",
    "heretic1.wad",
    "hexen.wad",
    "strife0.wad",
    "strife1.wad",
];

/// A game for the file at `path` whose first bytes are `header`: an
/// IWAD (a PWAD only adds levels to a game, and is left out), titled after
/// the game when it is a known one, else after the file.
pub fn game(path: PathBuf, header: &[u8]) -> Option<Wad> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if !name.ends_with(".wad")
        || !header.starts_with(b"IWAD")
        || OTHER_GAMES.contains(&name.as_str())
    {
        return None;
    }
    let title = KNOWN
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, title)| (*title).to_owned())
        .unwrap_or_else(|| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&name);
            stem.to_owned()
        });
    Some(Wad { title, path })
}

/// The games to offer: one per file name (the first found wins, so a card
/// copy is preferred over one in internal storage), sorted by title.
pub fn games(found: Vec<Wad>) -> Vec<Wad> {
    let mut out: Vec<Wad> = vec![];
    for wad in found {
        let name = |w: &Wad| w.path.file_name().map(|n| n.to_ascii_lowercase());
        if !out.iter().any(|w| name(w) == name(&wad)) {
            out.push(wad);
        }
    }
    out.sort_by_key(|w| w.title.to_lowercase());
    out
}

/// Where a game keeps its saves: a folder named after its file, so each
/// game's saves stay apart.
pub fn save_folder(wad: &Wad) -> String {
    let stem = wad
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doom")
        .to_ascii_lowercase();
    let clean: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "doom".into()
    } else {
        clean
    }
}

/// The last thing the engine said before stopping, worth showing: its
/// error line when it has one.
pub fn failure_line(stderr_tail: &[String]) -> Option<String> {
    let lines = || {
        stderr_tail
            .iter()
            .rev()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
    };
    lines()
        .find(|l| l.contains("Error") || l.contains("error"))
        .or_else(|| lines().next())
        .map(|l| {
            l.trim_start_matches("Error:")
                .trim_start()
                .chars()
                .take(120)
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut m = vec![b'D', b'G', kind, flags];
        m.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        m.extend_from_slice(payload);
        m
    }

    #[test]
    fn messages_come_out_whole_whatever_the_read_sizes() {
        let palette: Vec<u8> = (0..PALETTE_BYTES).map(|i| i as u8).collect();
        let frame: Vec<u8> = (0..FRAME_BYTES).map(|i| (i % 251) as u8).collect();
        let mut stream = message(b'P', 0, &palette);
        stream.extend(message(b'F', FLAG_MENU, &frame));
        stream.extend(message(b'F', 0, &frame));
        for chunk in [1, 7, 8, 9, 1000, 65_536, stream.len()] {
            let mut d = Decoder::default();
            let mut got = vec![];
            for piece in stream.chunks(chunk) {
                d.push(piece);
                while let Some(m) = d.next_message().unwrap() {
                    got.push(m);
                }
            }
            assert_eq!(got.len(), 3, "chunk {chunk}");
            assert_eq!(got[0], Message::Palette(palette.clone()));
            assert_eq!(
                got[1],
                Message::Frame {
                    pixels: frame.clone(),
                    menu: true
                }
            );
            assert!(matches!(&got[2], Message::Frame { menu: false, .. }));
            assert!(
                d.buf.len() < 2 * (FRAME_BYTES + HEADER_BYTES),
                "buffer stays small"
            );
        }
    }

    #[test]
    fn a_broken_stream_is_an_error() {
        let mut d = Decoder::default();
        d.push(b"XX");
        assert_eq!(d.next_message(), Ok(None), "waits for a whole header");
        d.push(b"P\0\0\0\0\0");
        assert_eq!(d.next_message(), Err(ProtocolError::BadMagic));
        let mut d = Decoder::default();
        d.push(&message(b'F', 0, &[0; 10]));
        assert_eq!(
            d.next_message(),
            Err(ProtocolError::BadLength {
                kind: b'F',
                length: 10
            })
        );
        let mut d = Decoder::default();
        d.push(&message(b'Q', 0, &[]));
        assert_eq!(d.next_message(), Err(ProtocolError::UnknownKind(b'Q')));
    }

    #[test]
    fn frames_are_coloured_through_the_palette() {
        let mut palette = vec![0u8; PALETTE_BYTES];
        palette[3..6].copy_from_slice(&[10, 20, 30]);
        palette[765..].copy_from_slice(&[200, 210, 220]);
        let mut out = vec![];
        to_rgba(&[1, 255, 0], &palette, &mut out);
        assert_eq!(out, [10, 20, 30, 255, 200, 210, 220, 255, 0, 0, 0, 255]);
        to_rgba(&[7], &[], &mut out);
        assert_eq!(out, [0, 0, 0, 255], "no palette yet: black");
    }

    #[test]
    fn inputs_encode_as_two_bytes() {
        assert_eq!(Input::Down(key::FIRE).encode(), [1, 0xa3]);
        assert_eq!(Input::Up(key::UP).encode(), [2, 0xad]);
        assert_eq!(Input::Turn(-3).encode(), [3, 0xfd]);
        assert_eq!(Input::Weapon(1).encode(), [4, 1]);
        assert_eq!(Input::Pause.encode(), [5, 0]);
        assert_eq!(Input::Resume.encode(), [6, 0]);
        assert_eq!(Input::Quit.encode(), [7, 0]);
    }

    fn sent(out: Option<Vec<Out>>) -> Vec<Input> {
        out.unwrap()
            .into_iter()
            .map(|o| match o {
                Out::Send(i) => i,
                Out::Leave => panic!("left"),
            })
            .collect()
    }

    #[test]
    fn held_buttons_are_held_keys() {
        use NormalizedInput as N;
        use PhysicalControl as C;
        let mut c = Controls::default();
        assert_eq!(sent(c.event(N::Press(C::Next))), [Input::Down(key::UP)]);
        assert_eq!(sent(c.event(N::LongPress(C::Next))), [], "still held");
        assert_eq!(sent(c.event(N::Repeat(C::Next))), []);
        assert_eq!(sent(c.event(N::Press(C::Select))), [Input::Down(key::FIRE)]);
        assert_eq!(sent(c.event(N::Release(C::Next))), [Input::Up(key::UP)]);
        assert_eq!(sent(c.event(N::Cancel(C::Select))), [Input::Up(key::FIRE)]);
        assert_eq!(sent(c.event(N::Release(C::Select))), [], "released once");
        assert_eq!(
            sent(c.event(N::Press(C::Previous))),
            [Input::Down(key::DOWN)]
        );
        assert_eq!(
            sent(c.event(N::Press(C::PlayPause))),
            [Input::Down(key::USE)]
        );
        assert_eq!(
            c.release_all(),
            [Input::Up(key::DOWN), Input::Up(key::USE)],
            "pausing lets go of everything"
        );
        assert_eq!(sent(c.event(N::Release(C::Previous))), []);
    }

    #[test]
    fn wheel_turns_and_volume_changes_weapon() {
        use NormalizedInput as N;
        use PhysicalControl as C;
        let mut c = Controls::default();
        assert_eq!(sent(c.event(N::WheelClockwise(1))), [Input::Turn(1)]);
        assert_eq!(
            sent(c.event(N::WheelCounterClockwise(3))),
            [Input::Turn(-3)]
        );
        assert_eq!(sent(c.event(N::WheelClockwise(200))), [Input::Turn(16)]);
        assert_eq!(sent(c.event(N::Press(C::VolumeUp))), [Input::Weapon(1)]);
        assert_eq!(
            sent(c.event(N::Repeat(C::VolumeUp))),
            [],
            "one weapon per press"
        );
        assert_eq!(sent(c.event(N::Press(C::VolumeDown))), [Input::Weapon(-1)]);
        assert_eq!(sent(c.event(N::Release(C::VolumeDown))), []);
        assert_eq!(
            sent(c.event(N::LongPress(C::PlayPause))),
            [Input::Down(key::TAB), Input::Up(key::TAB)],
            "held Play/Pause shows the map"
        );
        assert!(c.event(N::Press(C::Power)).is_none(), "Power is not Doom's");
        assert!(c.event(N::Release(C::Power)).is_none());
    }

    #[test]
    fn back_opens_the_menu_and_held_back_leaves() {
        use NormalizedInput as N;
        use PhysicalControl as C;
        let mut c = Controls::default();
        assert_eq!(sent(c.event(N::Press(C::Back))), []);
        assert_eq!(
            sent(c.event(N::Release(C::Back))),
            [Input::Down(key::ESCAPE), Input::Up(key::ESCAPE)]
        );
        c.event(N::Press(C::Next));
        c.event(N::Press(C::Back));
        let out = c.event(N::LongPress(C::Back)).unwrap();
        assert_eq!(out, [Out::Send(Input::Up(key::UP)), Out::Leave]);
        // Doom has gone; the rest of that press must not reach the list.
        assert!(c.swallows(N::Repeat(C::Back)));
        assert!(!c.swallows(N::Press(C::Select)));
        assert!(c.swallows(N::Release(C::Back)));
        assert!(!c.swallows(N::Release(C::Back)), "only that press");
        // A later short press is the menu again.
        c.event(N::Press(C::Back));
        assert_eq!(sent(c.event(N::Release(C::Back))).len(), 2);
    }

    #[test]
    fn iwads_are_games_and_pwads_are_not() {
        let wad = |p: &str, h: &[u8]| game(PathBuf::from(p), h);
        let f1 = wad("/sd/Doom/freedoom1.wad", b"IWAD\x01").unwrap();
        assert_eq!(f1.title, "Freedoom: Phase 1");
        assert_eq!(wad("/sd/Doom/DOOM2.WAD", b"IWAD").unwrap().title, "Doom II");
        assert_eq!(wad("/sd/Doom/mygame.wad", b"IWAD").unwrap().title, "mygame");
        assert!(wad("/sd/Doom/sigil.wad", b"PWAD").is_none(), "levels only");
        assert!(
            wad("/sd/Doom/heretic.wad", b"IWAD").is_none(),
            "another game"
        );
        assert!(wad("/sd/Doom/readme.txt", b"IWAD").is_none());
        assert!(wad("/sd/Doom/short.wad", b"IW").is_none());
    }

    #[test]
    fn each_game_is_offered_once_in_title_order() {
        let w = |p: &str, t: &str| Wad {
            title: t.into(),
            path: PathBuf::from(p),
        };
        let list = games(vec![
            w("/sd/Doom/freedoom2.wad", "Freedoom: Phase 2"),
            w("/sd/Doom/DOOM.WAD", "Doom"),
            w("/data/reborn/doom/freedoom2.wad", "Freedoom: Phase 2"),
            w("/sd/Doom/freedoom1.wad", "Freedoom: Phase 1"),
        ]);
        let titles: Vec<&str> = list.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["Doom", "Freedoom: Phase 1", "Freedoom: Phase 2"]);
        assert_eq!(
            list[2].path,
            PathBuf::from("/sd/Doom/freedoom2.wad"),
            "card first"
        );
        assert_eq!(save_folder(&list[0]), "doom");
        assert_eq!(save_folder(&w("/x/My Game!.wad", "")), "my_game_");
    }

    #[test]
    fn the_engine_error_line_is_the_one_shown() {
        let tail: Vec<String> = [
            "W_Init: Init WADfiles.",
            "Error: W_InitFiles: no files found",
            "",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            failure_line(&tail).as_deref(),
            Some("W_InitFiles: no files found"),
            "without the engine's prefix"
        );
        assert_eq!(failure_line(&["Z_Init".into()]).as_deref(), Some("Z_Init"));
        assert_eq!(failure_line(&[]), None);
    }
}

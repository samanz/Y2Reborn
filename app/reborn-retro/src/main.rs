//! reborn-retro: a small libretro frontend Reborn runs for Games > Retro
//! games, spoken to over pipes (`reborn_core::retro` is the protocol). It
//! contains no emulator: the cores are separate shared libraries the
//! system installs, each under its own licence, opened with dlopen.
//!
//! `reborn-retro <core.so> <game> --system <dir> --saves <dir> --rate <hz>`
//!
//! It opens the core, loads the game and runs it at the core's frame
//! rate. Each new picture goes to stdout as it is (RGB565, XRGB8888 or
//! 0RGB1555, rows packed), turned upright when the core asks for a
//! rotation (vertical arcade games); a picture identical to the last is
//! not sent.
//! The core's sound is converted to `--rate` (0: no sound) and sent as
//! stereo 16-bit samples; Reborn plays it through its own audio output.
//! Stdin carries button presses and releases, taps, pause/resume, save
//! and load state, and quit. The game's battery memory is kept in
//! `<saves>/<game>.srm` (loaded at start, written when it changes, on
//! pause and on quit) and one save state in `<saves>/<game>.state`.
//!
//! The core's own printing is moved to stderr so it cannot disturb the
//! picture stream; this program's errors are stderr lines starting
//! `reborn-retro:`.
mod libretro;

use libretro as lr;
use reborn_core::retro::{
    encode_frame_header, rotate, save_stem, Info, Input, Message, PixelFormat, Resampler,
    StatusKind, TAP_FRAMES,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::{c_char, c_int, c_uint, c_void, CStr, CString},
    fs::{self, File},
    io::{BufWriter, Read, Write},
    os::fd::FromRawFd,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

/// Battery memory is compared with the file this often while playing.
const SRAM_EVERY: Duration = Duration::from_secs(10);
/// Samples per audio message (well under the protocol's limit).
const AUDIO_CHUNK: usize = 8 * 1024;

struct Args {
    core: PathBuf,
    game: PathBuf,
    system: PathBuf,
    saves: PathBuf,
    rate: u32,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut positional = vec![];
        let (mut system, mut saves, mut rate) = (None, None, 0);
        let mut args = std::env::args_os().skip(1);
        while let Some(a) = args.next() {
            match a.to_str() {
                Some("--system") => system = args.next().map(PathBuf::from),
                Some("--saves") => saves = args.next().map(PathBuf::from),
                Some("--rate") => {
                    rate = args
                        .next()
                        .and_then(|r| r.to_str()?.parse().ok())
                        .ok_or("--rate needs a number")?
                }
                _ => positional.push(PathBuf::from(a)),
            }
        }
        let [core, game] = <[PathBuf; 2]>::try_from(positional).map_err(|_| {
            "usage: reborn-retro <core.so> <game> --system <dir> --saves <dir> --rate <hz>"
        })?;
        let saves = saves.ok_or("--saves is needed")?;
        Ok(Self {
            core,
            game,
            system: system.unwrap_or_else(|| saves.clone()),
            saves,
            rate,
        })
    }
}

/// Everything the core's callbacks reach.
struct Front {
    format: PixelFormat,
    system_dir: CString,
    save_dir: CString,
    core_path: CString,
    variables: HashMap<String, CString>,
    held: u16,
    taps: [u8; 16],
    audio_in: Vec<i16>,
    /// The picture being packed, and the last one sent.
    frame: Vec<u8>,
    last: Vec<u8>,
    last_shape: (u16, u16, PixelFormat),
    /// Quarter turns anticlockwise the core asked for, and the buffer
    /// pictures are turned in.
    rotation: c_uint,
    turned: Vec<u8>,
    out: BufWriter<File>,
    /// Writing failed: Reborn has gone.
    gone: bool,
    statuses: Vec<(StatusKind, String)>,
    av: lr::SystemAvInfo,
    info_dirty: bool,
    shutdown: bool,
}

thread_local! {
    static FRONT: RefCell<Option<Front>> = const { RefCell::new(None) };
}

/// Runs `f` on the frontend state. Never called while the core is running
/// from inside another `with` (the core is only called outside it).
fn with<R>(f: impl FnOnce(&mut Front) -> R) -> Option<R> {
    FRONT.with(|s| s.borrow_mut().as_mut().map(f))
}

impl Front {
    fn send(&mut self, bytes: &[u8]) {
        if !self.gone && self.out.write_all(bytes).is_err() {
            self.gone = true;
        }
    }

    fn flush(&mut self) {
        if !self.gone && self.out.flush().is_err() {
            self.gone = true;
        }
    }

    fn mask(&self) -> u16 {
        let mut m = self.held;
        for (id, n) in self.taps.iter().enumerate() {
            if *n > 0 {
                m |= 1 << id;
            }
        }
        m
    }

    /// SAFETY: `data` is what the core passed for `cmd` (libretro.h).
    unsafe fn environment(&mut self, cmd: c_uint, data: *mut c_void) -> bool {
        use lr::*;
        let set_dir = |data: *mut c_void, dir: &CString| {
            if !data.is_null() {
                *(data as *mut *const c_char) = dir.as_ptr();
            }
            !data.is_null()
        };
        let put = |data: *mut c_void, v: c_uint| {
            if !data.is_null() {
                *(data as *mut c_uint) = v;
            }
            !data.is_null()
        };
        let put_bool = |data: *mut c_void, v: bool| {
            if !data.is_null() {
                *(data as *mut bool) = v;
            }
            !data.is_null()
        };
        match cmd {
            ENV_GET_CAN_DUPE => put_bool(data, true),
            ENV_GET_OVERSCAN | ENV_GET_VARIABLE_UPDATE | ENV_GET_FASTFORWARDING => {
                put_bool(data, false)
            }
            ENV_SET_MESSAGE => {
                let m = data as *const lr::Message;
                if !m.is_null() && !(*m).msg.is_null() {
                    let text = CStr::from_ptr((*m).msg).to_string_lossy().into_owned();
                    self.statuses.push((StatusKind::Core, text));
                }
                true
            }
            ENV_SHUTDOWN => {
                self.shutdown = true;
                true
            }
            ENV_GET_SYSTEM_DIRECTORY | ENV_GET_CORE_ASSETS_DIRECTORY => {
                set_dir(data, &self.system_dir)
            }
            ENV_GET_SAVE_DIRECTORY => set_dir(data, &self.save_dir),
            ENV_GET_LIBRETRO_PATH => set_dir(data, &self.core_path),
            ENV_SET_PIXEL_FORMAT => {
                let Some(format) = (!data.is_null())
                    .then(|| PixelFormat::from_libretro(*(data as *const c_uint)))
                    .flatten()
                else {
                    return false;
                };
                self.format = format;
                true
            }
            ENV_SET_VARIABLES => {
                let mut v = data as *const lr::Variable;
                while !v.is_null() && !(*v).key.is_null() {
                    let key = CStr::from_ptr((*v).key).to_string_lossy().into_owned();
                    if !(*v).value.is_null() {
                        let value = CStr::from_ptr((*v).value).to_string_lossy();
                        if let Some(default) = default_option(&value) {
                            self.variables
                                .insert(key, CString::new(default).unwrap_or_default());
                        }
                    }
                    v = v.add(1);
                }
                true
            }
            ENV_GET_VARIABLE => {
                let v = data as *mut lr::Variable;
                if v.is_null() || (*v).key.is_null() {
                    return false;
                }
                let key = CStr::from_ptr((*v).key).to_string_lossy();
                match self.variables.get(key.as_ref()) {
                    Some(value) => {
                        (*v).value = value.as_ptr();
                        true
                    }
                    None => {
                        (*v).value = std::ptr::null();
                        false
                    }
                }
            }
            ENV_SET_GEOMETRY => {
                if data.is_null() {
                    return false;
                }
                self.av.geometry = *(data as *const lr::GameGeometry);
                self.info_dirty = true;
                true
            }
            ENV_SET_SYSTEM_AV_INFO => {
                if data.is_null() {
                    return false;
                }
                self.av = *(data as *const lr::SystemAvInfo);
                self.info_dirty = true;
                true
            }
            // Version 0: cores fall back to plain SET_VARIABLES.
            ENV_GET_CORE_OPTIONS_VERSION | ENV_GET_MESSAGE_INTERFACE_VERSION | ENV_GET_LANGUAGE => {
                put(data, 0)
            }
            ENV_GET_INPUT_MAX_USERS => put(data, 1),
            ENV_GET_AUDIO_VIDEO_ENABLE => {
                if !data.is_null() {
                    *(data as *mut c_int) = 3;
                }
                !data.is_null()
            }
            ENV_GET_INPUT_BITMASKS => true,
            ENV_SET_PERFORMANCE_LEVEL
            | ENV_SET_INPUT_DESCRIPTORS
            | ENV_SET_SUPPORT_NO_GAME
            | ENV_SET_SUBSYSTEM_INFO
            | ENV_SET_CONTROLLER_INFO
            | ENV_SET_MEMORY_MAPS
            | ENV_SET_SUPPORT_ACHIEVEMENTS
            | ENV_SET_SERIALIZATION_QUIRKS
            | ENV_SET_MINIMUM_AUDIO_LATENCY
            | ENV_SET_CONTENT_INFO_OVERRIDE => true,
            // A vertical arcade game draws its screen on its side and asks
            // for it to be turned (FBNeo: Pac-Man asks for 1); pictures are
            // turned here before they are sent, so Reborn sees them upright.
            ENV_SET_ROTATION => {
                if data.is_null() {
                    return false;
                }
                self.rotation = *(data as *const c_uint) % 4;
                self.info_dirty = true;
                true
            }
            // No hardware rendering, logging interface or the rest: the
            // core manages without (or says so).
            _ => false,
        }
    }

    /// SAFETY: `data` holds `height` rows of `pitch` bytes.
    unsafe fn video(&mut self, data: *const u8, width: c_uint, height: c_uint, pitch: usize) {
        let row = width as usize * self.format.bytes_per_pixel();
        if width == 0
            || height == 0
            || width > c_uint::from(reborn_core::retro::MAX_SIDE)
            || height > c_uint::from(reborn_core::retro::MAX_SIDE)
            || pitch < row
        {
            return;
        }
        self.frame.clear();
        for y in 0..height as usize {
            self.frame
                .extend_from_slice(std::slice::from_raw_parts(data.add(y * pitch), row));
        }
        let (mut width, mut height) = (width as usize, height as usize);
        if self.rotation != 0 {
            let bpp = self.format.bytes_per_pixel();
            (width, height) = rotate(
                &self.frame,
                width,
                height,
                bpp,
                self.rotation,
                &mut self.turned,
            );
            std::mem::swap(&mut self.frame, &mut self.turned);
        }
        let shape = (width as u16, height as u16, self.format);
        if shape == self.last_shape && self.frame == self.last {
            return;
        }
        let mut head = Vec::with_capacity(16);
        encode_frame_header(&mut head, shape.0, shape.1, shape.2);
        self.send(&head);
        let frame = std::mem::take(&mut self.frame);
        self.send(&frame);
        self.frame = std::mem::replace(&mut self.last, frame);
        self.last_shape = shape;
    }
}

/// "Description; first|second|third" -> "first".
fn default_option(value: &str) -> Option<String> {
    let (_, options) = value.split_once("; ")?;
    options.split('|').next().map(str::to_owned)
}

unsafe extern "C" fn environment(cmd: c_uint, data: *mut c_void) -> bool {
    // SAFETY: forwarded as the core gave them.
    with(|f| unsafe { f.environment(cmd, data) }).unwrap_or(false)
}

unsafe extern "C" fn video_refresh(
    data: *const c_void,
    width: c_uint,
    height: c_uint,
    pitch: usize,
) {
    if data.is_null() {
        return; // the same picture again
    }
    // SAFETY: the core passes `height` rows of `pitch` bytes.
    with(|f| unsafe { f.video(data as *const u8, width, height, pitch) });
}

unsafe extern "C" fn audio_sample(left: i16, right: i16) {
    with(|f| f.audio_in.extend_from_slice(&[left, right]));
}

unsafe extern "C" fn audio_sample_batch(data: *const i16, frames: usize) -> usize {
    if data.is_null() {
        return 0;
    }
    // SAFETY: the core passes `frames` stereo frames.
    let samples = unsafe { std::slice::from_raw_parts(data, frames * 2) };
    with(|f| f.audio_in.extend_from_slice(samples));
    frames
}

unsafe extern "C" fn input_poll() {}

unsafe extern "C" fn input_state(port: c_uint, device: c_uint, _index: c_uint, id: c_uint) -> i16 {
    if port != 0 || device != lr::DEVICE_JOYPAD {
        return 0;
    }
    let mask = with(|f| f.mask()).unwrap_or(0);
    if id == lr::DEVICE_ID_JOYPAD_MASK {
        mask as i16
    } else if id < 16 {
        ((mask >> id) & 1) as i16
    } else {
        0
    }
}

/// Reborn's messages, read on their own thread; the end of input is Quit.
fn read_inputs() -> Receiver<Input> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut b = [0u8; 2];
        while stdin.read_exact(&mut b).is_ok() {
            if let Some(input) = Input::decode(b) {
                if tx.send(input).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(Input::Quit);
    });
    rx
}

fn cstring(p: &Path) -> Result<CString, String> {
    CString::new(p.as_os_str().as_encoded_bytes()).map_err(|_| format!("a NUL in {}", p.display()))
}

/// Writes through a temporary file, so a crash cannot leave half a save.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// The game's battery memory, as the core holds it now.
fn sram(core: &lr::Core) -> Option<&'static mut [u8]> {
    // SAFETY: the core owns this memory for as long as the game is loaded;
    // it is only touched between frames.
    unsafe {
        let size = (core.get_memory_size)(lr::MEMORY_SAVE_RAM);
        let data = (core.get_memory_data)(lr::MEMORY_SAVE_RAM) as *mut u8;
        (size > 0 && !data.is_null()).then(|| std::slice::from_raw_parts_mut(data, size))
    }
}

struct Saves {
    srm: PathBuf,
    state: PathBuf,
    /// The battery memory as last written.
    written: Vec<u8>,
}

impl Saves {
    fn load_sram(&mut self, core: &lr::Core) {
        let (Some(mem), Ok(bytes)) = (sram(core), fs::read(&self.srm)) else {
            return;
        };
        let n = mem.len().min(bytes.len());
        mem[..n].copy_from_slice(&bytes[..n]);
        self.written = mem.to_vec();
    }

    fn save_sram(&mut self, core: &lr::Core) {
        let Some(mem) = sram(core) else {
            return;
        };
        if mem == self.written.as_slice() {
            return;
        }
        match write_atomic(&self.srm, mem) {
            Ok(()) => self.written = mem.to_vec(),
            Err(e) => eprintln!("reborn-retro: saving {} failed: {e}", self.srm.display()),
        }
    }

    fn save_state(&self, core: &lr::Core) -> (StatusKind, String) {
        // SAFETY: plain calls between frames; the buffer is the size the
        // core asked for.
        let result = unsafe {
            let size = (core.serialize_size)();
            let mut buf = vec![0u8; size];
            if size == 0 || !(core.serialize)(buf.as_mut_ptr() as *mut c_void, size) {
                Err("this game cannot save its state".to_owned())
            } else {
                write_atomic(&self.state, &buf).map_err(|e| e.to_string())
            }
        };
        match result {
            Ok(()) => (StatusKind::StateSaved, "State saved".into()),
            Err(e) => (StatusKind::StateFailed, format!("Could not save: {e}")),
        }
    }

    fn load_state(&self, core: &lr::Core) -> (StatusKind, String) {
        let Ok(buf) = fs::read(&self.state) else {
            return (StatusKind::StateFailed, "No saved state yet".into());
        };
        // SAFETY: the buffer is live for the call.
        if unsafe { (core.unserialize)(buf.as_ptr() as *const c_void, buf.len()) } {
            (StatusKind::StateLoaded, "State loaded".into())
        } else {
            (
                StatusKind::StateFailed,
                "That saved state does not fit this game".into(),
            )
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("reborn-retro: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse()?;
    // The protocol goes to a private copy of stdout; the core's own
    // printing (to stdout) goes to stderr instead.
    // SAFETY: plain descriptor calls at start-up, before any other thread.
    let protocol = unsafe {
        let fd = libc::dup(1);
        if fd < 0 || libc::dup2(2, 1) < 0 {
            return Err("standard output".into());
        }
        File::from_raw_fd(fd)
    };
    fs::create_dir_all(&args.saves).map_err(|e| format!("{}: {e}", args.saves.display()))?;
    let inputs = read_inputs();
    let core = lr::Core::open(&args.core)?;
    // SAFETY: the libretro calls, in the order the API asks for.
    let version = unsafe { (core.api_version)() };
    if version != lr::API_VERSION {
        return Err(format!("the core speaks libretro version {version}"));
    }
    FRONT.with(|s| {
        *s.borrow_mut() = Some(Front {
            format: PixelFormat::Rgb1555,
            system_dir: cstring(&args.system).unwrap_or_default(),
            save_dir: cstring(&args.saves).unwrap_or_default(),
            core_path: cstring(&args.core).unwrap_or_default(),
            variables: HashMap::new(),
            held: 0,
            taps: [0; 16],
            audio_in: Vec::with_capacity(4096),
            frame: vec![],
            last: vec![],
            last_shape: (0, 0, PixelFormat::Rgb1555),
            rotation: 0,
            turned: vec![],
            out: BufWriter::with_capacity(512 * 1024, protocol),
            gone: false,
            statuses: vec![],
            av: lr::SystemAvInfo::default(),
            info_dirty: false,
            shutdown: false,
        })
    });
    // SAFETY: see above; the game's path and bytes outlive load_game.
    let av = unsafe {
        (core.set_environment)(environment);
        (core.set_video_refresh)(video_refresh);
        (core.set_audio_sample)(audio_sample);
        (core.set_audio_sample_batch)(audio_sample_batch);
        (core.set_input_poll)(input_poll);
        (core.set_input_state)(input_state);
        (core.init)();
        let mut sys: lr::SystemInfo = std::mem::zeroed();
        (core.get_system_info)(&mut sys);
        let path = cstring(&args.game)?;
        let data = if sys.need_fullpath {
            vec![]
        } else {
            fs::read(&args.game).map_err(|e| format!("{}: {e}", args.game.display()))?
        };
        let game = lr::GameInfo {
            path: path.as_ptr(),
            data: if data.is_empty() {
                std::ptr::null()
            } else {
                data.as_ptr() as *const c_void
            },
            size: data.len(),
            meta: std::ptr::null(),
        };
        if !(core.load_game)(&game) {
            return Err("the core could not load this game".into());
        }
        let mut av = lr::SystemAvInfo::default();
        (core.get_system_av_info)(&mut av);
        (core.set_controller_port_device)(0, lr::DEVICE_JOYPAD);
        av
    };
    with(|f| {
        f.av = av;
        f.info_dirty = true;
    });
    let stem = save_stem(&args.game);
    let mut saves = Saves {
        srm: args.saves.join(format!("{stem}.srm")),
        state: args.saves.join(format!("{stem}.state")),
        written: vec![],
    };
    saves.load_sram(&core);
    play(&core, &args, &inputs, &mut saves);
    saves.save_sram(&core);
    // SAFETY: the game is unloaded once, after the last frame.
    unsafe {
        (core.unload_game)();
        (core.deinit)();
    }
    with(|f| f.flush());
    Ok(())
}

/// The frame loop, until Quit, the core shuts down or Reborn goes.
fn play(core: &lr::Core, args: &Args, inputs: &Receiver<Input>, saves: &mut Saves) {
    let mut paused = false;
    let mut next = Instant::now();
    let mut sram_due = Instant::now() + SRAM_EVERY;
    let mut resampler: Option<(u32, Resampler)> = None;
    let mut audio_out: Vec<i16> = Vec::with_capacity(4096);
    loop {
        // Reborn's messages. While paused, wait for them.
        let first = if paused {
            match inputs.recv_timeout(Duration::from_millis(200)) {
                Ok(i) => Some(i),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            None
        };
        for input in first.into_iter().chain(inputs.try_iter()) {
            match input {
                Input::Down(p) => {
                    with(|f| f.held |= p.bit());
                }
                Input::Up(p) => {
                    with(|f| f.held &= !p.bit());
                }
                Input::Tap(p) => {
                    with(|f| f.taps[p as usize] = TAP_FRAMES);
                }
                Input::Pause => {
                    paused = true;
                    with(|f| f.held = 0);
                    saves.save_sram(core);
                }
                Input::Resume => {
                    paused = false;
                    next = Instant::now();
                }
                Input::SaveState => {
                    let s = saves.save_state(core);
                    with(|f| f.statuses.push(s));
                }
                Input::LoadState => {
                    let s = saves.load_state(core);
                    with(|f| f.statuses.push(s));
                }
                Input::Quit => return,
            }
        }
        if !paused {
            // SAFETY: one frame; the callbacks borrow the state themselves.
            unsafe { (core.run)() };
            with(|f| {
                for t in f.taps.iter_mut() {
                    *t = t.saturating_sub(1);
                }
            });
        }
        // Everything the frame produced, then the frame's messages.
        let (fps, core_rate, info) = with(|f| {
            let mut av = f.av;
            if f.rotation % 2 == 1 {
                // Upright, as the pictures are sent.
                let g = &mut av.geometry;
                (g.base_width, g.base_height) = (g.base_height, g.base_width);
            }
            let info = std::mem::take(&mut f.info_dirty).then_some(av);
            (f.av.timing.fps, f.av.timing.sample_rate, info)
        })
        .unwrap_or((60., 0., None));
        let mut bytes = vec![];
        if let Some(av) = info {
            bytes.extend(
                Message::Info(Info {
                    fps_milli: (av.timing.fps * 1000.).round() as u32,
                    core_rate: av.timing.sample_rate.round() as u32,
                    base_width: av.geometry.base_width as u16,
                    base_height: av.geometry.base_height as u16,
                })
                .encode(),
            );
        }
        let samples = with(|f| std::mem::take(&mut f.audio_in)).unwrap_or_default();
        let core_rate = core_rate.round() as u32;
        if args.rate > 0 && core_rate > 0 && !samples.is_empty() {
            if resampler.as_ref().is_none_or(|(r, _)| *r != core_rate) {
                resampler = Some((core_rate, Resampler::new(core_rate, args.rate)));
            }
            if let Some((_, r)) = resampler.as_mut() {
                audio_out.clear();
                r.process(&samples, &mut audio_out);
                for chunk in audio_out.chunks(AUDIO_CHUNK) {
                    bytes.extend(Message::Audio(chunk.to_vec()).encode());
                }
            }
        }
        let gone = with(|f| {
            for (kind, text) in std::mem::take(&mut f.statuses) {
                bytes.extend(Message::Status(kind, text).encode());
            }
            f.send(&bytes);
            f.flush();
            f.gone || f.shutdown
        })
        .unwrap_or(true);
        if gone {
            return;
        }
        if paused {
            continue;
        }
        if Instant::now() >= sram_due {
            saves.save_sram(core);
            sram_due = Instant::now() + SRAM_EVERY;
        }
        // Keep to the core's frame rate; after a stall, start afresh
        // rather than racing to catch up.
        let period = Duration::from_secs_f64(1. / if fps > 1. { fps } else { 60. });
        next += period;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now - next > period * 4 {
            next = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::retro::Pad;

    #[test]
    fn option_defaults_are_the_first_choice() {
        assert_eq!(
            default_option("Region; Auto|NTSC|PAL").as_deref(),
            Some("Auto")
        );
        assert_eq!(
            default_option("Turbo; disabled").as_deref(),
            Some("disabled")
        );
        assert_eq!(default_option("no separator"), None);
    }

    #[test]
    fn pads_are_bits_of_the_mask() {
        assert_eq!(Pad::B.bit(), 1);
        assert_eq!(Pad::A.bit(), 1 << 8);
    }
}

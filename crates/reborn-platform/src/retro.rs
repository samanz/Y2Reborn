//! Retro games: finding the cores and the games on the card, and the
//! frontend process (`reborn-retro`, its own executable, which loads the
//! libretro core): started on a game, its pictures read on a thread and
//! coloured for the GPU, its sound handed to the caller, button messages
//! written to its input. The protocol itself is `reborn_core::retro`.
use reborn_core::retro::{self, Decoder, Input, Message, Rom, StatusKind, System};
use std::{
    collections::VecDeque,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// Lines of the frontend's (and core's) own output kept to explain a
/// failure.
const STDERR_LINES: usize = 24;

/// The frontend: `REBORN_RETRO` when set, else beside Reborn's own binary
/// (`/usr/bin/reborn-retro` on the player), else the libexec path.
pub fn binary() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("reborn-retro")));
    [
        std::env::var_os("REBORN_RETRO").map(PathBuf::from),
        beside,
        Some(PathBuf::from("/usr/libexec/reborn-retro")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
}

/// A system's core when it is installed: in `REBORN_RETRO_CORES` when set
/// (host testing), else the image's libretro folder.
pub fn core(system: System) -> Option<PathBuf> {
    let dir = std::env::var_os("REBORN_RETRO_CORES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(retro::CORE_DIR));
    let path = dir.join(format!("{}_libretro.so", system.core()));
    path.is_file().then_some(path)
}

/// The system's games under these `Games` folders (one per card), in that
/// order of preference.
pub fn find_roms(system: System, games_dirs: &[PathBuf]) -> Vec<Rom> {
    let mut found = vec![];
    for games in games_dirs {
        for folder in system.folders() {
            let Ok(entries) = fs::read_dir(games.join(folder)) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for path in paths {
                if path.is_file() {
                    found.extend(retro::rom(system, path));
                }
            }
        }
    }
    retro::roms(found)
}

#[derive(Default)]
struct Shared {
    /// The newest picture, opaque RGBA, its size, and how many have come.
    rgba: Vec<u8>,
    size: (u32, u32),
    serial: u64,
    statuses: Vec<(StatusKind, String)>,
    info: Option<retro::Info>,
    /// The stream broke (not a clean exit).
    broken: Option<String>,
    stderr: VecDeque<String>,
}

/// What the frontend is told to run.
pub struct Game<'a> {
    pub core: &'a Path,
    pub rom: &'a Path,
    /// The core's system directory (`<card>/Games/BIOS`).
    pub system_dir: &'a Path,
    /// Battery saves and the save state.
    pub saves: &'a Path,
    /// The output's sample rate; 0 for no sound.
    pub rate: u32,
}

/// A running frontend. Dropping it kills the process.
pub struct Engine {
    child: Option<Child>,
    stdin: ChildStdin,
    shared: Arc<Mutex<Shared>>,
    taken: u64,
}

impl Engine {
    /// Starts `binary` on `game`. `wake` is called (from another thread)
    /// when a picture or message arrives or the stream ends; `audio` gets
    /// the sound as it comes.
    pub fn start(
        binary: &Path,
        game: &Game,
        wake: impl Fn() + Send + 'static,
        audio: impl Fn(&[i16]) + Send + 'static,
    ) -> io::Result<Self> {
        fs::create_dir_all(game.saves)?;
        let mut command = Command::new(binary);
        command
            .arg(game.core)
            .arg(game.rom)
            .arg("--system")
            .arg(game.system_dir)
            .arg("--saves")
            .arg(game.saves)
            .arg("--rate")
            .arg(game.rate.to_string())
            .current_dir(game.saves)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: nice() is async-signal-safe and touches only the new
        // process. A little below Reborn: the UI and its audio come first.
        unsafe {
            command.pre_exec(|| {
                libc::nice(5);
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("frontend pipes"));
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let frames = Arc::clone(&shared);
        thread::Builder::new()
            .name("retro-frames".into())
            .spawn(move || read_stream(stdout, &frames, &wake, &audio))?;
        let lines = Arc::clone(&shared);
        thread::Builder::new()
            .name("retro-log".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Ok(mut s) = lines.lock() {
                        if s.stderr.len() == STDERR_LINES {
                            s.stderr.pop_front();
                        }
                        s.stderr.push_back(line);
                    }
                }
            })?;
        Ok(Self {
            child: Some(child),
            stdin,
            shared,
            taken: 0,
        })
    }

    pub fn send(&mut self, inputs: &[Input]) -> io::Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }
        let bytes: Vec<u8> = inputs.iter().flat_map(|i| i.encode()).collect();
        self.stdin.write_all(&bytes)
    }

    /// Swaps the newest picture into `out` when one has come since the last
    /// call, giving its size.
    pub fn take_frame(&mut self, out: &mut Vec<u8>) -> Option<(u32, u32)> {
        let mut s = self.shared.lock().ok()?;
        if s.serial == self.taken || s.rgba.is_empty() {
            return None;
        }
        self.taken = s.serial;
        std::mem::swap(out, &mut s.rgba);
        Some(s.size)
    }

    /// What the frontend has said since the last call (state saved, ...).
    pub fn take_statuses(&mut self) -> Vec<(StatusKind, String)> {
        self.shared
            .lock()
            .map(|mut s| std::mem::take(&mut s.statuses))
            .unwrap_or_default()
    }

    /// The game's frame rate and sizes, once known.
    pub fn info(&self) -> Option<retro::Info> {
        self.shared.lock().ok().and_then(|s| s.info)
    }

    /// `Some` once the frontend has stopped: `Some(None)` for a clean exit,
    /// `Some(Some(why))` for a failure.
    pub fn ended(&mut self) -> Option<Option<String>> {
        let broken = self.shared.lock().ok().and_then(|s| s.broken.clone());
        let child = self.child.as_mut()?;
        let status = match (child.try_wait(), &broken) {
            (Ok(Some(status)), _) => status,
            (_, Some(_)) => {
                let _ = child.kill();
                child.wait().ok()?
            }
            _ => return None,
        };
        self.child = None;
        if status.success() && broken.is_none() {
            return Some(None);
        }
        // Let the log thread catch the last lines.
        thread::sleep(Duration::from_millis(20));
        let tail: Vec<String> = self
            .shared
            .lock()
            .map(|s| s.stderr.iter().cloned().collect())
            .unwrap_or_default();
        Some(Some(
            broken
                .or_else(|| retro::failure_line(&tail))
                .unwrap_or_else(|| format!("it stopped ({status})")),
        ))
    }

    /// Asks the frontend to save and quit; a background thread makes sure
    /// it has gone within three seconds.
    pub fn stop(mut self) {
        let _ = self.send(&[Input::Quit]);
        if let Some(mut child) = self.child.take() {
            let _ = thread::Builder::new()
                .name("retro-stop".into())
                .spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(3);
                    while Instant::now() < deadline {
                        if let Ok(Some(_)) = child.try_wait() {
                            return;
                        }
                        thread::sleep(Duration::from_millis(50));
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                });
        }
    }

    /// Stops and waits (tests, shutdown): true when it exited by itself.
    pub fn stop_and_wait(mut self, timeout: Duration) -> bool {
        let _ = self.send(&[Input::Quit]);
        let Some(mut child) = self.child.take() else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                return status.success();
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        false
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn read_stream(
    mut stdout: impl Read,
    shared: &Mutex<Shared>,
    wake: &dyn Fn(),
    audio: &dyn Fn(&[i16]),
) {
    let mut decoder = Decoder::default();
    let mut buf = vec![0u8; 256 * 1024];
    let mut rgba = vec![];
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        decoder.push(&buf[..n]);
        let mut woke = false;
        loop {
            match decoder.next_message() {
                Ok(Some(Message::Frame {
                    width,
                    height,
                    format,
                    pixels,
                })) => {
                    retro::to_rgba(format, &pixels, &mut rgba);
                    if let Ok(mut s) = shared.lock() {
                        std::mem::swap(&mut s.rgba, &mut rgba);
                        s.size = (u32::from(width), u32::from(height));
                        s.serial += 1;
                    }
                    woke = true;
                }
                Ok(Some(Message::Audio(samples))) => audio(&samples),
                Ok(Some(Message::Info(info))) => {
                    if let Ok(mut s) = shared.lock() {
                        s.info = Some(info);
                    }
                }
                Ok(Some(Message::Status(kind, text))) => {
                    if let Ok(mut s) = shared.lock() {
                        s.statuses.push((kind, text));
                    }
                    woke = true;
                }
                Ok(None) => break,
                Err(e) => {
                    if let Ok(mut s) = shared.lock() {
                        s.broken = Some(e.to_string());
                    }
                    wake();
                    return;
                }
            }
        }
        if woke {
            wake();
        }
    }
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::retro::{PixelFormat, StatusKind};

    #[test]
    fn games_are_found_in_every_folder_of_a_system() {
        let dir = std::env::temp_dir().join(format!("reborn-retro-{}", std::process::id()));
        let card = dir.join("card/Games");
        fs::create_dir_all(card.join("Genesis")).unwrap();
        fs::create_dir_all(card.join("SMS")).unwrap();
        fs::create_dir_all(card.join("NES")).unwrap();
        fs::write(card.join("Genesis/Ecco (USA).md"), b"x").unwrap();
        fs::write(card.join("SMS/Alex.sms"), b"x").unwrap();
        fs::write(card.join("Genesis/readme.txt"), b"x").unwrap();
        fs::write(card.join("NES/game.nes"), b"x").unwrap();
        fs::create_dir_all(card.join("Genesis/folder.md")).unwrap();
        let roms = find_roms(System::Genesis, &[card.clone(), dir.join("missing")]);
        let titles: Vec<&str> = roms.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["Alex", "Ecco"]);
        assert_eq!(find_roms(System::Nes, &[card]).len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_stream_reaches_the_ui_thread_coloured_and_sound_reaches_the_sink() {
        let mut stream = Message::Info(retro::Info {
            fps_milli: 60_000,
            core_rate: 48_000,
            base_width: 2,
            base_height: 1,
        })
        .encode();
        stream.extend(
            Message::Frame {
                width: 2,
                height: 1,
                format: PixelFormat::Rgb565,
                pixels: vec![0x00, 0xF8, 0x1F, 0x00],
            }
            .encode(),
        );
        stream.extend(Message::Audio(vec![5, -5]).encode());
        stream.extend(Message::Status(StatusKind::StateSaved, "State saved".into()).encode());
        let shared = Mutex::new(Shared::default());
        let heard = Mutex::new(vec![]);
        read_stream(&stream[..], &shared, &|| {}, &|s: &[i16]| {
            heard.lock().unwrap().extend_from_slice(s)
        });
        let s = shared.lock().unwrap();
        assert_eq!(s.serial, 1);
        assert_eq!(s.size, (2, 1));
        assert_eq!(s.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        assert_eq!(s.info.unwrap().fps_milli, 60_000);
        assert_eq!(s.statuses, [(StatusKind::StateSaved, "State saved".into())]);
        assert_eq!(*heard.lock().unwrap(), [5, -5]);
        assert!(s.broken.is_none());
        drop(s);
        let shared = Mutex::new(Shared::default());
        read_stream(&b"garbage!"[..], &shared, &|| {}, &|_: &[i16]| {});
        assert!(shared.lock().unwrap().broken.is_some());
    }
}

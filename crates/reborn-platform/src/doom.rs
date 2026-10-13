//! The Doom engine process (`reborn-doom`, GPL, its own executable): found
//! next to Reborn, started with an IWAD, its frames read on a thread and
//! coloured for the GPU, key messages written to its input. The protocol
//! itself is `reborn_core::doom`.
use reborn_core::doom::{self, Decoder, Input, Message, Wad};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// Lines of the engine's own output kept to explain a failure.
const STDERR_LINES: usize = 24;

/// The engine executable: `REBORN_DOOM` when set, else beside Reborn's own
/// binary (`/usr/bin/reborn-doom` on the player), else the libexec path.
pub fn binary() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("reborn-doom")));
    [
        std::env::var_os("REBORN_DOOM").map(PathBuf::from),
        beside,
        Some(PathBuf::from("/usr/libexec/reborn-doom")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
}

/// The games in these folders (`<card>/Doom`, `/data/reborn/doom`), in
/// that order of preference.
pub fn find_games(dirs: &[PathBuf]) -> Vec<Wad> {
    let mut found = vec![];
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let wad = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wad"));
            if !wad || !path.is_file() {
                continue;
            }
            let mut header = [0u8; 4];
            if File::open(&path)
                .and_then(|mut f| f.read_exact(&mut header))
                .is_err()
            {
                continue;
            }
            found.extend(doom::game(path, &header));
        }
    }
    doom::games(found)
}

#[derive(Default)]
struct Shared {
    /// The newest frame, opaque RGBA 320x200, and how many have come.
    rgba: Vec<u8>,
    serial: u64,
    /// The frame stream broke (not a clean exit).
    broken: Option<String>,
    stderr: VecDeque<String>,
}

/// A running engine. Dropping it kills the process.
pub struct Engine {
    child: Option<Child>,
    stdin: ChildStdin,
    shared: Arc<Mutex<Shared>>,
    taken: u64,
}

impl Engine {
    /// Starts `binary` on `iwad`, keeping its settings and saves in `home`.
    /// `wake` is called (from another thread) when a frame arrives or the
    /// stream ends.
    pub fn start(
        binary: &Path,
        iwad: &Path,
        home: &Path,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        fs::create_dir_all(home)?;
        let mut command = Command::new(binary);
        command
            .arg("-iwad")
            .arg(iwad)
            .current_dir(home)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: nice() is async-signal-safe and touches only the new
        // process. Below Reborn's priority, music and the UI come first.
        unsafe {
            command.pre_exec(|| {
                libc::nice(10);
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("engine pipes"));
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let frames = Arc::clone(&shared);
        thread::Builder::new()
            .name("doom-frames".into())
            .spawn(move || read_frames(stdout, &frames, &wake))?;
        let lines = Arc::clone(&shared);
        thread::Builder::new()
            .name("doom-log".into())
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
        let bytes: Vec<u8> = inputs.iter().flat_map(|i| i.encode()).collect();
        self.stdin.write_all(&bytes)
    }

    /// Swaps the newest frame into `out` when one has come since the last
    /// call.
    pub fn take_frame(&mut self, out: &mut Vec<u8>) -> bool {
        let Ok(mut s) = self.shared.lock() else {
            return false;
        };
        if s.serial == self.taken || s.rgba.is_empty() {
            return false;
        }
        self.taken = s.serial;
        std::mem::swap(out, &mut s.rgba);
        true
    }

    /// `Some` once the engine has stopped: `Some(None)` for a clean exit
    /// (Quit Game), `Some(Some(why))` for a failure.
    pub fn ended(&mut self) -> Option<Option<String>> {
        let broken = self.shared.lock().ok().and_then(|s| s.broken.clone());
        let child = self.child.as_mut()?;
        let status = match (child.try_wait(), &broken) {
            (Ok(Some(status)), _) => status,
            (_, Some(_)) => {
                // A garbled stream cannot be played: stop it.
                let _ = child.kill();
                child.wait().ok()?
            }
            _ => return None,
        };
        self.child = None;
        if status.success() && broken.is_none() {
            return Some(None);
        }
        let tail: Vec<String> = self
            .shared
            .lock()
            .map(|s| s.stderr.iter().cloned().collect())
            .unwrap_or_default();
        Some(Some(
            broken
                .or_else(|| doom::failure_line(&tail))
                .unwrap_or_else(|| format!("it stopped ({status})")),
        ))
    }

    /// Asks the engine to quit; a background thread makes sure it has gone
    /// within two seconds.
    pub fn stop(mut self) {
        let _ = self.send(&[Input::Quit]);
        if let Some(mut child) = self.child.take() {
            let _ = thread::Builder::new()
                .name("doom-stop".into())
                .spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(2);
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
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn read_frames(mut stdout: impl Read, shared: &Mutex<Shared>, wake: &dyn Fn()) {
    let mut decoder = Decoder::default();
    let mut buf = vec![0u8; 64 * 1024];
    let mut palette = vec![0u8; doom::PALETTE_BYTES];
    let mut rgba = Vec::with_capacity(doom::FRAME_BYTES * 4);
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        decoder.push(&buf[..n]);
        loop {
            match decoder.next_message() {
                Ok(Some(Message::Palette(p))) => palette = p,
                Ok(Some(Message::Frame { pixels, .. })) => {
                    doom::to_rgba(&pixels, &palette, &mut rgba);
                    if let Ok(mut s) = shared.lock() {
                        std::mem::swap(&mut s.rgba, &mut rgba);
                        s.serial += 1;
                    }
                    wake();
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
    }
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_are_found_by_their_header() {
        let dir = std::env::temp_dir().join(format!("reborn-doom-{}", std::process::id()));
        let card = dir.join("Doom");
        fs::create_dir_all(&card).unwrap();
        fs::write(card.join("freedoom1.wad"), b"IWAD\0\0\0\0").unwrap();
        fs::write(card.join("SIGIL.WAD"), b"PWAD\0\0\0\0").unwrap();
        fs::write(card.join("DOOM2.WAD"), b"IWAD\0\0\0\0").unwrap();
        fs::write(card.join("notes.txt"), b"IWAD").unwrap();
        fs::write(card.join("empty.wad"), b"").unwrap();
        let games = find_games(&[card.clone(), dir.join("missing")]);
        let titles: Vec<&str> = games.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["Doom II", "Freedoom: Phase 1"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn frames_from_the_stream_reach_the_ui_thread_coloured() {
        let mut stream = vec![];
        let mut palette = vec![0u8; doom::PALETTE_BYTES];
        palette[3..6].copy_from_slice(&[1, 2, 3]);
        for (kind, payload) in [(b'P', palette), (b'F', vec![1u8; doom::FRAME_BYTES])] {
            stream.extend_from_slice(&[b'D', b'G', kind, 0]);
            stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            stream.extend(payload);
        }
        let shared = Mutex::new(Shared::default());
        let woken = std::sync::atomic::AtomicUsize::new(0);
        read_frames(&stream[..], &shared, &|| {
            woken.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let s = shared.lock().unwrap();
        assert_eq!(s.serial, 1);
        assert_eq!(s.rgba.len(), doom::FRAME_BYTES * 4);
        assert_eq!(&s.rgba[..4], &[1, 2, 3, 255]);
        assert!(s.broken.is_none());
        assert_eq!(
            woken.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "a frame, then the end"
        );
        drop(s);
        let shared = Mutex::new(Shared::default());
        read_frames(&b"garbage!"[..], &shared, &|| {});
        assert!(shared.lock().unwrap().broken.is_some());
    }

    /// The real engine on a real IWAD (host build): REBORN_DOOM names the
    /// binary and REBORN_DOOM_WAD the IWAD. Run with `--ignored`.
    #[test]
    #[ignore]
    fn the_engine_runs_pauses_and_quits() {
        let (Some(bin), Some(wad)) = (binary(), std::env::var_os("REBORN_DOOM_WAD")) else {
            panic!("set REBORN_DOOM and REBORN_DOOM_WAD");
        };
        let home = std::env::temp_dir().join(format!("reborn-doom-run-{}", std::process::id()));
        let mut engine = Engine::start(&bin, Path::new(&wad), &home, || {}).unwrap();
        let mut frame = vec![];
        let wait_frame = |engine: &mut Engine, frame: &mut Vec<u8>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if engine.take_frame(frame) {
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            false
        };
        assert!(wait_frame(&mut engine, &mut frame), "a first frame");
        assert_eq!(frame.len(), doom::FRAME_BYTES * 4);
        // Select on the title opens Doom's menu: the picture changes.
        let title = frame.clone();
        engine
            .send(&[Input::Down(doom::key::FIRE), Input::Up(doom::key::FIRE)])
            .unwrap();
        thread::sleep(Duration::from_millis(300));
        assert!(wait_frame(&mut engine, &mut frame));
        assert_ne!(frame, title);
        engine.send(&[Input::Pause]).unwrap();
        thread::sleep(Duration::from_millis(200));
        engine.take_frame(&mut frame);
        thread::sleep(Duration::from_millis(500));
        assert!(!engine.take_frame(&mut frame), "no frames while paused");
        engine.send(&[Input::Resume, Input::Quit]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut ended = None;
        while ended.is_none() && Instant::now() < deadline {
            ended = engine.ended();
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(ended, Some(None), "a clean exit");
        // A missing IWAD is a failure with the engine's own words.
        let mut bad = Engine::start(&bin, Path::new("/nonexistent.wad"), &home, || {}).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut ended = None;
        while ended.is_none() && Instant::now() < deadline {
            ended = bad.ended();
            thread::sleep(Duration::from_millis(20));
        }
        let why = ended.flatten().expect("a failure");
        eprintln!("missing IWAD: {why}");
        fs::remove_dir_all(home).unwrap();
    }
}

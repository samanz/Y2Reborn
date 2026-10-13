//! reborn-retro end to end, through Reborn's own process wrapper
//! (`reborn_platform::retro::Engine`), on a stand-in core
//! (`tests/fake_core.c`, built here with the host C compiler): pictures in
//! every pixel format, only changed pictures sent, sound at the asked
//! rate, buttons, pause, save states, battery memory, and failures in
//! words.
//!
//! With `REBORN_RETRO_CAPTURE=<dir>` it also writes the stand-in core's
//! picture at each system's size as raw RGBA (`<w>x<h>.rgba`) for the
//! previews.
use reborn_core::retro::{Input, Pad, StatusKind};
use reborn_platform::retro::{Engine, Game};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reborn-retro-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The stand-in core, compiled once per test binary.
fn fake_core() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let dir = scratch("core");
            let so = dir.join("fake_libretro.so");
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_core.c");
            let status = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
                .args(["-shared", "-fPIC", "-O1", "-Wall", "-o"])
                .arg(&so)
                .arg(&source)
                .status()
                .expect("a C compiler");
            assert!(status.success(), "fake core builds");
            so
        })
        .clone()
}

struct Run {
    engine: Engine,
    sound: Arc<Mutex<Vec<i16>>>,
    saves: PathBuf,
}

fn start(name: &str, card: &str, rate: u32) -> Run {
    start_core(name, card, rate, &fake_core())
}

fn start_core(name: &str, card: &str, rate: u32, core: &Path) -> Run {
    let dir = scratch(name);
    let rom = dir.join("game.fake");
    fs::write(&rom, card).unwrap();
    let saves = dir.join("saves");
    let sound = Arc::new(Mutex::new(vec![]));
    let heard = Arc::clone(&sound);
    let engine = Engine::start(
        Path::new(env!("CARGO_BIN_EXE_reborn-retro")),
        &Game {
            core,
            rom: &rom,
            system_dir: &dir,
            saves: &saves,
            rate,
        },
        || {},
        move |s: &[i16]| heard.lock().unwrap().extend_from_slice(s),
    )
    .unwrap();
    Run {
        engine,
        sound,
        saves,
    }
}

fn frame(engine: &mut Engine, within: Duration) -> Option<(Vec<u8>, (u32, u32))> {
    let deadline = Instant::now() + within;
    let mut out = vec![];
    while Instant::now() < deadline {
        if let Some(size) = engine.take_frame(&mut out) {
            return Some((out, size));
        }
        thread::sleep(Duration::from_millis(5));
    }
    None
}

fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn statuses(engine: &mut Engine, within: Duration) -> Vec<(StatusKind, String)> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        let s = engine.take_statuses();
        if !s.is_empty() {
            return s;
        }
        thread::sleep(Duration::from_millis(5));
    }
    vec![]
}

#[test]
fn a_game_plays_pauses_saves_and_quits() {
    let mut run = start("play", "256 240 2 32040", 48_000);
    let (rgba, (w, h)) = frame(&mut run.engine, Duration::from_secs(5)).expect("a picture");
    assert_eq!((w, h), (256, 240));
    assert_eq!(rgba.len(), 256 * 240 * 4);
    assert_eq!(pixel(&rgba, w, 0, 0), [255, 255, 255], "the border");
    assert_eq!(pixel(&rgba, w, 128, 120), [123, 121, 123], "the box, off");
    // The option's default (blue) reached the core.
    assert_eq!(pixel(&rgba, w, 128, 200), [0, 0, 206]);
    // A still picture is not sent again.
    thread::sleep(Duration::from_millis(100));
    let _ = run.engine.take_frame(&mut vec![]);
    assert!(
        frame(&mut run.engine, Duration::from_millis(250)).is_none(),
        "nothing new"
    );
    // Holding A lights the box; letting go puts it out.
    run.engine.send(&[Input::Down(Pad::A)]).unwrap();
    let (lit, _) = frame(&mut run.engine, Duration::from_secs(2)).unwrap();
    assert_eq!(pixel(&lit, w, 128, 120), [255, 40, 41]);
    run.engine.send(&[Input::Up(Pad::A)]).unwrap();
    let (off, _) = frame(&mut run.engine, Duration::from_secs(2)).unwrap();
    assert_eq!(pixel(&off, w, 128, 120), [123, 121, 123]);
    // A tap is seen too (pressed for a few frames).
    run.engine.send(&[Input::Tap(Pad::A)]).unwrap();
    assert!(frame(&mut run.engine, Duration::from_secs(2)).is_some());
    // Sound at the asked rate: about 48000 frames a second.
    run.sound.lock().unwrap().clear();
    let t = Instant::now();
    thread::sleep(Duration::from_millis(1000));
    let frames = run.sound.lock().unwrap().len() / 2;
    let expected = 48_000. * t.elapsed().as_secs_f64();
    assert!(
        (frames as f64 - expected).abs() < expected * 0.15,
        "{frames} sound frames in {:?}",
        t.elapsed()
    );
    // Paused: no sound, no pictures.
    run.engine.send(&[Input::Pause]).unwrap();
    thread::sleep(Duration::from_millis(100));
    run.sound.lock().unwrap().clear();
    let _ = run.engine.take_frame(&mut vec![]); // from before the pause
    run.engine.send(&[Input::Down(Pad::A)]).unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(run.sound.lock().unwrap().is_empty(), "silent while paused");
    assert!(run.engine.take_frame(&mut vec![]).is_none());
    run.engine.send(&[Input::Resume]).unwrap();
    assert!(
        frame(&mut run.engine, Duration::from_secs(2)).is_some(),
        "goes on"
    );
    // States.
    run.engine.send(&[Input::LoadState]).unwrap();
    assert_eq!(
        statuses(&mut run.engine, Duration::from_secs(2)),
        [(StatusKind::StateFailed, "No saved state yet".into())]
    );
    run.engine.send(&[Input::SaveState]).unwrap();
    assert_eq!(
        statuses(&mut run.engine, Duration::from_secs(2)),
        [(StatusKind::StateSaved, "State saved".into())]
    );
    assert!(run.saves.join("game.state").is_file());
    run.engine.send(&[Input::LoadState]).unwrap();
    assert_eq!(
        statuses(&mut run.engine, Duration::from_secs(2)),
        [(StatusKind::StateLoaded, "State loaded".into())]
    );
    // Quit: a clean exit, battery memory written (A was pressed twice: the
    // tap and the press held over the pause).
    assert!(
        run.engine.stop_and_wait(Duration::from_secs(3)),
        "clean exit"
    );
    let srm = fs::read(run.saves.join("game.srm")).expect("battery memory");
    assert_eq!(srm.len(), 64);
    assert_eq!(srm[0], 3, "three presses of A");
    // The next start loads it back: one more press makes four.
    let mut again = start_with_saves(&run.saves, "256 240 2 32040");
    frame(&mut again.engine, Duration::from_secs(5)).unwrap();
    again.engine.send(&[Input::Tap(Pad::A)]).unwrap();
    frame(&mut again.engine, Duration::from_secs(2)).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(again.engine.stop_and_wait(Duration::from_secs(3)));
    assert_eq!(fs::read(again.saves.join("game.srm")).unwrap()[0], 4);
    assert!(again.sound.lock().unwrap().is_empty(), "rate 0: no sound");
}

fn start_with_saves(saves: &Path, card: &str) -> Run {
    let dir = saves.parent().unwrap().to_path_buf();
    let rom = dir.join("game.fake");
    fs::write(&rom, card).unwrap();
    let sound = Arc::new(Mutex::new(vec![]));
    let core = fake_core();
    let engine = Engine::start(
        Path::new(env!("CARGO_BIN_EXE_reborn-retro")),
        &Game {
            core: &core,
            rom: &rom,
            system_dir: &dir,
            saves,
            rate: 0,
        },
        || {},
        |_: &[i16]| {},
    )
    .unwrap();
    Run {
        engine,
        sound,
        saves: saves.to_path_buf(),
    }
}

#[test]
fn every_pixel_format_and_size_arrives_as_rgba() {
    let capture = std::env::var_os("REBORN_RETRO_CAPTURE").map(PathBuf::from);
    for (i, (w, h, format)) in [
        (256, 240, 2),
        (160, 144, 0),
        (240, 160, 1),
        (320, 224, 2),
        (256, 224, 1),
        (384, 224, 2),
    ]
    .into_iter()
    .enumerate()
    {
        let mut run = start(&format!("size{i}"), &format!("{w} {h} {format} 44100"), 0);
        let (rgba, size) = frame(&mut run.engine, Duration::from_secs(5)).expect("a picture");
        assert_eq!(size, (w, h));
        assert_eq!(pixel(&rgba, w, 0, h - 1), [255, 255, 255], "{format}");
        // The first colour bar is white, the last red, in every format.
        let bar_y = h / 8 + 1;
        assert_eq!(pixel(&rgba, w, 9, bar_y), [255, 255, 255], "{format}");
        let red = pixel(&rgba, w, w - 10, bar_y);
        assert!(
            red[0] > 200 && red[1] == 0 && red[2] == 0,
            "{format}: {red:?}"
        );
        if let Some(dir) = &capture {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join(format!("{w}x{h}.rgba")), &rgba).unwrap();
        }
        assert!(run.engine.stop_and_wait(Duration::from_secs(3)));
    }
}

#[test]
fn a_game_drawn_on_its_side_arrives_upright() {
    // Like FBNeo's Pac-Man: a 288x224 screen on its side, and a request to
    // turn it a quarter anticlockwise.
    let mut run = start("rotated", "288 224 2 44100 1", 0);
    let (rgba, (w, h)) = frame(&mut run.engine, Duration::from_secs(5)).expect("a picture");
    assert_eq!((w, h), (224, 288), "upright");
    // The colour bars ran left to right along the side's top; turned, they
    // run bottom to top down the left: white at the bottom, red at the top.
    let bar_x = 224 / 8 + 1;
    assert_eq!(pixel(&rgba, w, bar_x, 288 - 10), [255, 255, 255]);
    let red = pixel(&rgba, w, bar_x, 9);
    assert!(red[0] > 200 && red[1] == 0 && red[2] == 0, "{red:?}");
    let deadline = Instant::now() + Duration::from_secs(2);
    while run.engine.info().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let info = run.engine.info().expect("the game's sizes");
    assert_eq!((info.base_width, info.base_height), (224, 288));
    // The buttons are not turned: the game's up is the screen's up.
    run.engine.send(&[Input::Down(Pad::A)]).unwrap();
    let (lit, _) = frame(&mut run.engine, Duration::from_secs(2)).unwrap();
    assert_eq!(pixel(&lit, w, 112, 144), [255, 40, 41]);
    if let Some(dir) = std::env::var_os("REBORN_RETRO_CAPTURE").map(PathBuf::from) {
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("224x288.rgba"), &rgba).unwrap();
    }
    assert!(run.engine.stop_and_wait(Duration::from_secs(3)));
}

fn failure(mut engine: Engine) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(ended) = engine.ended() {
            return ended.expect("a failure, not a clean exit");
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("still running");
}

#[test]
fn failures_are_said_in_words() {
    let run = start("bad-game", "fail", 0);
    assert_eq!(failure(run.engine), "the core could not load this game");
    let run = start_core(
        "no-core",
        "256 240 2",
        0,
        Path::new("/nonexistent_libretro.so"),
    );
    assert!(
        failure(run.engine).starts_with("the core could not be opened"),
        "the dlopen error"
    );
}

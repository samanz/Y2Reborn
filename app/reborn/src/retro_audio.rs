//! A retro game's sound through Reborn's own audio output. The frontend
//! sends 16-bit stereo at the output's rate over its pipe; this worker owns
//! an ALSA sink (the same device and plan as music: the wired output or the
//! Bluetooth A2DP PCM) while a game runs, after music has let go of it, and
//! plays the samples at the user volume with music's gain curve.
use crate::volume::VolumeRamp;
use reborn_audio::{AlsaSink, AudioSink, SinkSpec};
use reborn_core::{retro::AudioQueue, PcmFormat};
use reborn_observability::{Level, Observer};
use serde_json::json;
use std::{
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Frames written per block (about 21 ms at 48 kHz).
const BLOCK: usize = 1024;
/// At most this much waiting in the queue (about 85 ms); past it the
/// oldest is dropped, so a stall never leaves the sound late.
const MAX_QUEUED: usize = 4096;
const KEEP_QUEUED: usize = 2048;

enum Control {
    /// Play (the game runs) or stop and drop what is queued (paused).
    Run(bool),
    /// Hold the device (or let go of it while the screen is off, so the
    /// player can sleep).
    Open(bool),
    /// User volume, 0-100.
    Volume(u8),
    Stop,
}

type Queue = Arc<(Mutex<AudioQueue>, Condvar)>;

pub struct GameAudio {
    queue: Queue,
    control: Sender<Control>,
    thread: Option<JoinHandle<()>>,
    /// The output's sample rate (what the frontend converts to).
    pub rate: u32,
    sent: (bool, bool, u8),
}

impl GameAudio {
    /// A worker for the sink `spec` (planned by the caller after music
    /// released the device). It opens the sink when first told to run.
    pub fn start(spec: SinkSpec, volume: u8, log: Observer, id: u64) -> Self {
        let queue: Queue = Arc::new((
            Mutex::new(AudioQueue::new(MAX_QUEUED, KEEP_QUEUED)),
            Condvar::new(),
        ));
        let (control, rx) = mpsc::channel();
        let rate = spec.rate;
        let worker_queue = Arc::clone(&queue);
        let thread = thread::Builder::new()
            .name("retro-audio".into())
            .spawn(move || {
                Worker {
                    ramp: VolumeRamp::new(volume, spec.rate),
                    spec,
                    log,
                    id,
                    sink: None,
                    open: true,
                    failed: false,
                    running: false,
                }
                .run(&worker_queue, &rx)
            })
            .ok();
        Self {
            queue,
            control,
            thread,
            rate,
            sent: (false, true, volume),
        }
    }

    /// What the frontend's reader thread calls with each block of sound.
    pub fn feeder(&self) -> impl Fn(&[i16]) + Send + 'static {
        let queue = Arc::clone(&self.queue);
        move |samples| {
            let (lock, ready) = &*queue;
            if let Ok(mut q) = lock.lock() {
                q.push(samples);
            }
            ready.notify_one();
        }
    }

    /// Follows the game and the screen; sends only changes.
    pub fn follow(&mut self, running: bool, open: bool, volume: u8) {
        let (r, o, v) = self.sent;
        if open != o {
            let _ = self.control.send(Control::Open(open));
        }
        if running != r {
            let _ = self.control.send(Control::Run(running));
        }
        if volume != v {
            let _ = self.control.send(Control::Volume(volume));
        }
        self.sent = (running, open, volume);
    }
}

impl Drop for GameAudio {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
        self.queue.1.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    spec: SinkSpec,
    log: Observer,
    id: u64,
    sink: Option<AlsaSink>,
    /// The user volume, applied to the samples as music's sink does.
    ramp: VolumeRamp,
    /// Hold the device.
    open: bool,
    /// Opening or writing failed: the game goes on silently.
    failed: bool,
    running: bool,
}

impl Worker {
    fn run(mut self, queue: &Queue, rx: &Receiver<Control>) {
        let mut block = Vec::with_capacity(BLOCK * 2);
        let mut bytes = vec![];
        loop {
            loop {
                match rx.try_recv() {
                    Ok(Control::Stop) | Err(TryRecvError::Disconnected) => return,
                    Ok(c) => self.control(c, queue),
                    Err(TryRecvError::Empty) => break,
                }
            }
            if self.running && self.open && self.sink.is_none() && !self.failed {
                self.open_sink();
            }
            let Some(sink) = self.sink.as_mut().filter(|_| self.running) else {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(Control::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    Ok(c) => self.control(c, queue),
                    Err(RecvTimeoutError::Timeout) => {}
                }
                continue;
            };
            block.clear();
            {
                let (lock, ready) = &**queue;
                let Ok(mut q) = lock.lock() else {
                    return;
                };
                if q.frames() == 0 {
                    q = match ready.wait_timeout(q, Duration::from_millis(20)) {
                        Ok((q, _)) => q,
                        Err(_) => return,
                    };
                }
                q.pop(BLOCK, &mut block);
            }
            if block.is_empty() {
                continue;
            }
            let format = sink.parameters().format;
            convert(&block, format, &mut bytes);
            let frame_bytes = format.bytes_per_frame();
            let mut at = 0;
            while at < bytes.len() {
                let pcm = self.ramp.render(&bytes[at..], format);
                match sink.write(&pcm) {
                    Ok(n) => {
                        self.ramp.advance(n);
                        at += n * frame_bytes;
                    }
                    Err(error) => {
                        self.log.emit(
                            Level::Warn,
                            "retro",
                            "audio_failed",
                            &error,
                            Some(self.id),
                            json!({}),
                        );
                        self.sink = None;
                        self.failed = true;
                        break;
                    }
                }
            }
        }
    }

    fn control(&mut self, c: Control, queue: &Queue) {
        match c {
            Control::Run(run) => {
                self.running = run;
                if !run {
                    if let Some(sink) = self.sink.as_mut() {
                        let _ = sink.discard();
                    }
                    if let Ok(mut q) = queue.0.lock() {
                        q.clear();
                    }
                }
            }
            Control::Open(open) => {
                self.open = open;
                if !open {
                    self.sink = None;
                }
            }
            Control::Volume(volume) => self.ramp.target(volume),
            Control::Stop => {}
        }
    }

    fn open_sink(&mut self) {
        match AlsaSink::open_spec(&self.spec, self.log.clone(), self.id) {
            Ok(sink) => {
                self.sink = Some(sink);
                self.log.emit(
                    Level::Info,
                    "retro",
                    "audio_open",
                    "Game sound on the audio output",
                    Some(self.id),
                    json!({"device":self.spec.device,"rate":self.spec.rate}),
                );
            }
            Err(error) => {
                self.failed = true;
                self.log.emit(
                    Level::Warn,
                    "retro",
                    "audio_failed",
                    &error,
                    Some(self.id),
                    json!({"device":self.spec.device}),
                );
            }
        }
    }
}

/// 16-bit stereo to the sink's format, at full scale.
fn convert(samples: &[i16], format: PcmFormat, out: &mut Vec<u8>) {
    out.clear();
    for &s in samples {
        match format {
            PcmFormat::S16LE => out.extend_from_slice(&s.to_le_bytes()),
            PcmFormat::S24LE => out.extend_from_slice(&(i32::from(s) << 8).to_le_bytes()),
            PcmFormat::S32LE => out.extend_from_slice(&(i32::from(s) << 16).to_le_bytes()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::AudioOutput;

    #[test]
    fn samples_fill_each_container_at_full_scale() {
        let mut out = vec![];
        convert(&[1, -1], PcmFormat::S16LE, &mut out);
        assert_eq!(out, [1, 0, 0xFF, 0xFF]);
        convert(&[0x1234], PcmFormat::S32LE, &mut out);
        assert_eq!(out, [0, 0, 0x34, 0x12]);
        convert(&[0x1234], PcmFormat::S24LE, &mut out);
        assert_eq!(out, [0, 0x34, 0x12, 0]);
    }

    /// The worker on ALSA's null device: opens when the game runs, plays
    /// what comes, lets go when paused or the screen goes off, stops
    /// cleanly.
    #[test]
    fn the_worker_plays_into_a_sink_and_lets_go() {
        let dir = std::env::temp_dir().join(format!("reborn-retro-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = Observer::new(&dir).unwrap();
        let spec = SinkSpec {
            output: AudioOutput::Bluetooth("01:02:03:04:05:06".into()),
            rate: 48_000,
            format: PcmFormat::S16LE,
            physical_bits: 16,
            valid_bits: 16,
            channels: 2,
            layout: "stereo".into(),
            device: "null".into(),
            codec: None,
            transport_object: Some("o".into()),
            transport_device: Some("d".into()),
            transport: Some("t".into()),
            mode: Some("m".into()),
            transport_generation: 1,
            fallback: false,
            fallback_reason: String::new(),
        };
        let mut audio = GameAudio::start(spec, 50, log, 1);
        assert_eq!(audio.rate, 48_000);
        let feed = audio.feeder();
        audio.follow(true, true, 50);
        for _ in 0..20 {
            feed(&[100; 1600]);
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            audio.queue.0.lock().unwrap().frames(),
            0,
            "the sink opened and took it all"
        );
        audio.follow(false, true, 50);
        audio.follow(false, false, 50);
        audio.follow(true, true, 30);
        feed(&[100; 1600]);
        thread::sleep(Duration::from_millis(50));
        drop(audio);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

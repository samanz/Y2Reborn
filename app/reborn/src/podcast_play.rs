//! The runtime side of Podcasts: following the SD card, refreshing feeds
//! and downloading over Wi-Fi through the worker, playing downloaded
//! episodes through the normal player, and keeping where listening stopped.
use crate::{podcasts, Runtime};
use reborn_core::{podcasts::FOLDER, PlaybackState, Screen, Track};
use reborn_observability::Level;
use reborn_ui::WifiStatus;
use serde_json::json;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

/// While an episode plays, its position is saved this often.
const SAVE_EVERY: Duration = Duration::from_secs(20);

pub(crate) struct Podcasts {
    worker: Option<podcasts::Worker>,
    /// The listening state was read from the data folder.
    loaded: bool,
    /// The card whose shows are shown.
    card: Option<PathBuf>,
    /// The episode downloading and, once it began, its partial file.
    download: Option<(String, Option<PathBuf>)>,
    /// Downloads that failed in this session; tried again after a refresh.
    failed: HashSet<String>,
    saved: Instant,
    /// The episode last seen playing and its position.
    playing: Option<(String, u64)>,
}

impl Podcasts {
    pub(crate) fn new(headless: bool) -> Result<Self, String> {
        Ok(Self {
            worker: if headless {
                None
            } else {
                Some(podcasts::Worker::spawn().map_err(|e| e.to_string())?)
            },
            loaded: false,
            card: None,
            download: None,
            failed: HashSet::new(),
            saved: Instant::now(),
            playing: None,
        })
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A track id for an episode, apart from the library's (positive) ids.
fn episode_track_id(id: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    -((h >> 2) as i64) - 1
}

impl Runtime {
    /// Refreshing or downloading keeps Wi-Fi up and the player awake.
    pub(crate) fn podcast_busy(&self) -> bool {
        self.ui.podcasts.refreshing || self.podcasts.download.is_some()
    }

    fn podcast_online(&self) -> bool {
        matches!(self.ui.wifi.status, WifiStatus::Connected(_))
    }

    /// The SD card, when one is in.
    fn podcast_card(&self) -> Option<PathBuf> {
        self.model
            .sources
            .iter()
            .find(|s| s.online && matches!(s.kind, reborn_core::MediaSource::SdCard(_)))
            .map(|s| s.root.clone())
    }

    pub(crate) fn podcast_refresh(&mut self) {
        let Some(card) = self.podcast_card() else {
            self.ui.flash("Insert the SD card with your podcasts");
            return;
        };
        let v = &mut self.ui.podcasts;
        if v.refreshing {
            return;
        }
        if let Some(worker) = &self.podcasts.worker {
            if worker.refresh(card) {
                v.refreshing = true;
                v.problem = None;
            }
        }
    }

    /// Starts the next waiting download, when on Wi-Fi and none runs.
    pub(crate) fn podcast_next_download(&mut self) {
        if self.podcasts.download.is_some() || !self.podcast_online() {
            return;
        }
        let Some(card) = self.podcasts.card.clone() else {
            return;
        };
        let v = &self.ui.podcasts;
        let failed = &self.podcasts.failed;
        let Some((show, episode)) = v
            .state
            .next_download(&v.shows, |id| v.is_on_card(id) || failed.contains(id))
        else {
            return;
        };
        let id = episode.id.clone();
        if self
            .podcasts
            .worker
            .as_ref()
            .is_some_and(|w| w.download(card, show, episode.clone()))
        {
            self.ui.podcasts.downloading = Some((id.clone(), 0));
            self.podcasts.download = Some((id, None));
        }
    }

    /// Plays a downloaded episode from where listening left off.
    pub(crate) fn play_episode(&mut self, id: &str) -> Result<(), String> {
        let card = self.podcasts.card.clone().ok_or("Insert the SD card")?;
        let source = self
            .model
            .sources
            .iter()
            .find(|s| s.online && s.root == card)
            .map(|s| s.id.clone())
            .ok_or("Insert the SD card")?;
        let v = &self.ui.podcasts;
        let (show, e) = v.find(id).ok_or("That episode is no longer on the card")?;
        let file = e.file_name();
        let path = card.join(FOLDER).join(&show.folder).join(&file);
        let size = fs::metadata(&path)
            .map_err(|_| "That episode is missing from the card")?
            .len();
        let track = Track {
            id: episode_track_id(id),
            source_id: source,
            filename: file,
            path,
            size,
            title: e.title.clone(),
            artist: show.title.clone(),
            album: show.title.clone(),
            album_artist: show.title.clone(),
            duration_ms: e.duration_ms(),
            online: true,
            ..Default::default()
        };
        let position = v.state.resume_ms(id);
        self.model.replace_queue(vec![track], 0)?;
        self.model.position_ms = position;
        self.podcasts.playing = Some((id.to_owned(), position));
        self.load()
    }

    /// Deletes an episode's audio (stopping it if it plays), or cancels its
    /// download.
    pub(crate) fn delete_episode(&mut self, id: &str) {
        if self.current_episode().as_deref() == Some(id) {
            self.model.invalidate();
            self.playback.stop(self.model.generation);
            self.model.queue.clear();
            self.model.queue_entry_ids.clear();
            self.model.queue_position = 0;
            self.model.position_ms = 0;
            self.model.playback = PlaybackState::Stopped;
            self.podcasts.playing = None;
            self.dirty.mark_both();
            self.checkpoint();
        }
        // A download in progress finishes in the background; its file is
        // deleted when it arrives, as it is no longer wanted.
        let (card, v) = (self.podcasts.card.clone(), &mut self.ui.podcasts);
        if let (Some(card), Some((show, e))) = (card, v.find(id)) {
            podcasts::delete(&card, &show.folder, e);
        }
        v.on_card.remove(id);
        v.state.forget(id);
        v.dirty = true;
        self.log
            .emit(Level::Info, "podcasts", "deleted", id, None, json!({}));
    }

    /// The episode the current track is, by its file in a show's folder.
    fn current_episode(&self) -> Option<String> {
        let track = self.model.current()?;
        let show = track.path.parent()?;
        if show.parent()?.file_name()? != FOLDER {
            return None;
        }
        let id = track.path.file_stem()?.to_str()?;
        self.ui.podcasts.find(id).map(|_| id.to_owned())
    }

    /// The current episode played to its end.
    pub(crate) fn episode_ended(&mut self) {
        if let Some(id) = self.current_episode() {
            let v = &mut self.ui.podcasts;
            v.dirty |= v.state.mark_played(&id);
        }
    }

    /// Keeps the playing episode's position, and saves listening now and
    /// then while playing, at once otherwise.
    fn follow_episode(&mut self) {
        let current = self.current_episode();
        let position = self.model.position_ms;
        let v = &mut self.ui.podcasts;
        if v.playing != current {
            v.playing.clone_from(&current);
            self.dirty.mark_render();
        }
        if let Some(id) = current {
            let moved = self
                .podcasts
                .playing
                .as_ref()
                .is_none_or(|(was, at)| *was != id || *at != position);
            if moved {
                let duration = self.model.current().map_or(0, |t| t.duration_ms);
                v.dirty |= v.state.listened(&id, position, duration);
                self.podcasts.playing = Some((id, position));
            }
        }
        let playing = matches!(
            self.model.playback,
            PlaybackState::Playing | PlaybackState::Buffering
        );
        if v.dirty && (!playing || self.podcasts.saved.elapsed() >= SAVE_EVERY) {
            let saved = serde_json::to_vec(&v.state)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    reborn_core::atomic_write(&podcasts::state_path(&self.root), &bytes)
                        .map_err(|e| e.to_string())
                });
            if let Err(e) = saved {
                self.log
                    .emit(Level::Warn, "podcasts", "save_failed", &e, None, json!({}));
            }
            v.dirty = false;
            self.podcasts.saved = Instant::now();
        }
    }

    /// Once per main-loop pass: follows the card, applies the worker's
    /// replies and download progress, keeps listening, and starts the next
    /// download when on Wi-Fi.
    pub(crate) fn pump_podcasts(&mut self) {
        if !self.podcasts.loaded {
            self.podcasts.loaded = true;
            self.ui.podcasts.state = podcasts::load_state(&self.root);
        }
        self.ui.podcasts.now = unix_now();
        let card = self.podcast_card();
        if card != self.podcasts.card {
            let v = &mut self.ui.podcasts;
            v.card = card.is_some();
            v.shows.clear();
            v.on_card.clear();
            if let (Some(card), Some(worker)) = (&card, &self.podcasts.worker) {
                worker.read(card.clone());
            }
            self.podcasts.card = card;
            self.dirty.mark_render();
        }
        let replies: Vec<podcasts::Reply> = self
            .podcasts
            .worker
            .as_ref()
            .map(|w| w.replies.try_iter().collect())
            .unwrap_or_default();
        for reply in replies {
            self.podcast_reply(reply);
            self.dirty.mark_render();
        }
        if let Some((id, Some(part))) = &self.podcasts.download {
            let got = fs::metadata(part).map_or(0, |m| m.len());
            let shown = self.ui.podcasts.downloading.as_ref().map_or(0, |d| d.1);
            if got / 1_000_000 != shown / 1_000_000 {
                self.ui.podcasts.downloading = Some((id.clone(), got));
                let visible = matches!(
                    self.model.screen,
                    Screen::Podcasts | Screen::PodcastShow | Screen::PodcastEpisode
                );
                if visible && !self.model.screen_off {
                    self.dirty.mark_render();
                }
            }
        }
        self.follow_episode();
        self.podcast_next_download();
    }

    fn podcast_reply(&mut self, reply: podcasts::Reply) {
        let current = self.podcasts.card.clone();
        let v = &mut self.ui.podcasts;
        match reply {
            podcasts::Reply::Read(card, read) => {
                if Some(&card) == current.as_ref() {
                    v.shows = read.shows;
                    v.on_card = read.on_card;
                    let on_card = &v.on_card;
                    v.state.wanted.retain(|id| !on_card.contains(id));
                }
            }
            podcasts::Reply::Refreshed(card, result) => {
                v.refreshing = false;
                match result {
                    Ok((read, failed)) if Some(&card) == current.as_ref() => {
                        let known: HashSet<&str> = v
                            .shows
                            .iter()
                            .flat_map(|s| s.episodes.iter().map(|e| e.id.as_str()))
                            .collect();
                        let new = read
                            .shows
                            .iter()
                            .flat_map(|s| &s.episodes)
                            .filter(|e| !known.contains(e.id.as_str()))
                            .count();
                        v.shows = read.shows;
                        v.on_card = read.on_card;
                        let on_card = &v.on_card;
                        v.state
                            .refreshed(&v.shows, |id| on_card.contains(id), unix_now());
                        v.dirty = true;
                        v.problem = match failed.len() {
                            0 => None,
                            1 => Some("1 feed couldn't be read".into()),
                            n => Some(format!("{n} feeds couldn't be read")),
                        };
                        self.ui.flash(match new {
                            0 => "No new episodes".to_owned(),
                            1 => "1 new episode".to_owned(),
                            n => format!("{n} new episodes"),
                        });
                        self.podcasts.failed.clear();
                    }
                    Ok(_) => {}
                    Err(e) => {
                        self.log.emit(
                            Level::Warn,
                            "podcasts",
                            "refresh_failed",
                            &e,
                            None,
                            json!({}),
                        );
                        v.problem = Some(e);
                    }
                }
            }
            podcasts::Reply::Started { id, part } => {
                self.podcasts.download = Some((id, Some(part)));
            }
            podcasts::Reply::Downloaded { id, result } => {
                self.podcasts.download = None;
                v.downloading = None;
                match result {
                    Ok(()) if v.state.wanted.contains(&id) => {
                        v.state.wanted.retain(|w| *w != id);
                        v.on_card.insert(id.clone());
                        v.dirty = true;
                        self.log
                            .emit(Level::Info, "podcasts", "downloaded", &id, None, json!({}));
                    }
                    // Deleted (cancelled) while it downloaded.
                    Ok(()) => {
                        if let (Some(card), Some((show, e))) = (current, v.find(&id)) {
                            podcasts::delete(&card, &show.folder, e);
                        }
                    }
                    Err(error) => {
                        self.log.emit(
                            Level::Warn,
                            "podcasts",
                            "download_failed",
                            &error,
                            None,
                            json!({"id":id}),
                        );
                        v.problem = Some(error);
                        self.podcasts.failed.insert(id);
                    }
                }
            }
        }
    }
}

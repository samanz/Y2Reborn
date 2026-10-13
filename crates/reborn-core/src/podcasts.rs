//! Podcasts from plain RSS feeds. The feeds are listed on the SD card in
//! `Podcasts/feeds.txt` (and any `Podcasts/*.opml`); over Wi-Fi the player
//! reads them, keeps each show's index in `Podcasts/<show>/show.json` and
//! downloads the episodes chosen into the same folder, so the card carries
//! the shows and their audio. Listening (positions, played, what is waiting
//! to download) is kept by the application in `PodcastState`.
//!
//! Everything here is pure; the application does the I/O.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The folder on the SD card that holds the feed list and the shows.
pub const FOLDER: &str = "Podcasts";
/// One feed address per line; `#` starts a comment.
pub const FEEDS_FILE: &str = "feeds.txt";
/// A show's index inside its folder.
pub const INDEX_FILE: &str = "show.json";
/// Marks `FOLDER` as the player's, so the library scanner leaves the
/// episodes out of Songs.
pub const MARKER: &str = ".reborn-podcasts";
/// Episodes kept in a show's index (the newest; older ones stay while their
/// audio is on the card).
pub const MAX_EPISODES: usize = 50;
/// When a feed has new episodes, this many of the newest download by
/// themselves; others are downloaded from their page.
pub const AUTO_DOWNLOAD: usize = 1;
/// Listening within this much of the end counts as played.
const END_SLACK_MS: u64 = 30_000;
/// Resuming steps back a little so the last sentence is heard again.
const RESUME_REWIND_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Episode {
    /// Stable across feed refreshes (`episode_id`).
    pub id: String,
    pub title: String,
    /// Published, unix seconds (0 when the feed did not say).
    pub published: i64,
    /// Length in seconds, as the feed says (0 when unknown).
    pub duration_s: u64,
    /// The enclosure: audio address, size in bytes (0 unknown) and type.
    pub url: String,
    pub length: u64,
    pub mime: String,
    /// As plain paragraphs (`podcast_text::clean`).
    pub description: String,
}

impl Episode {
    pub fn duration_ms(&self) -> u64 {
        self.duration_s.saturating_mul(1000)
    }

    /// The audio file's name in the show's folder: the id and the audio's
    /// extension (from the address, else the type).
    pub fn file_name(&self) -> String {
        let path = self.url.split(['?', '#']).next().unwrap_or("");
        let from_path = path
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, ext)| ext.to_ascii_lowercase());
        let from_mime = match self.mime.as_str() {
            "audio/mp4" | "audio/x-m4a" | "audio/m4a" => Some("m4a"),
            "audio/aac" => Some("aac"),
            "audio/ogg" => Some("ogg"),
            "audio/opus" => Some("opus"),
            "video/mp4" => Some("mp4"),
            _ => None,
        };
        let ext = from_path
            .filter(|ext| matches!(ext.as_str(), "mp3" | "m4a" | "aac" | "ogg" | "opus" | "mp4"))
            .or(from_mime.map(str::to_owned))
            .unwrap_or_else(|| "mp3".into());
        format!("{}.{ext}", self.id)
    }
}

/// A show as its folder on the card describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Show {
    /// The feed's address, as listed in `feeds.txt`.
    pub feed: String,
    pub title: String,
    pub author: String,
    /// The show's artwork address, as the feed gives it.
    pub image: String,
    pub description: String,
    /// Newest first.
    pub episodes: Vec<Episode>,
    /// Its folder's name under `FOLDER` (where it was read from).
    #[serde(skip)]
    pub folder: String,
}

impl Show {
    pub fn episode(&self, id: &str) -> Option<&Episode> {
        self.episodes.iter().find(|e| e.id == id)
    }

    /// This show after a refresh that read `fresh`: the feed's episodes,
    /// plus those it no longer lists whose audio is still on the card.
    pub fn refreshed(&self, mut fresh: Show, on_card: impl Fn(&str) -> bool) -> Show {
        for old in &self.episodes {
            if on_card(&old.id) && fresh.episode(&old.id).is_none() {
                fresh.episodes.push(old.clone());
            }
        }
        fresh
            .episodes
            .sort_by_key(|e| std::cmp::Reverse(e.published));
        fresh.folder.clone_from(&self.folder);
        fresh
    }
}

/// An episode's id: a hash of its feed and its guid (or audio address), so
/// two shows can share guids and a feed can be listed again later.
pub fn episode_id(feed: &str, guid: &str) -> String {
    // FNV-1a 64.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in feed.bytes().chain([0]).chain(guid.trim().bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// An http(s) address with nothing that would break a command line.
pub fn is_web_address(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() > 10
        && !url.contains(|c: char| c.is_whitespace() || c.is_control())
}

/// The address starts with `http://` (any case).
pub fn is_plain_http(url: &str) -> bool {
    url.get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
}

/// The addresses to try for `url`, in order: an `http://` address as
/// `https://` first, then as written only when `plain_ok` (its feed was
/// listed as `http://`); anything else as written.
pub fn https_first(url: &str, plain_ok: bool) -> Vec<String> {
    if !is_plain_http(url) {
        return vec![url.to_owned()];
    }
    let secure = format!("https://{}", &url[7..]);
    if plain_ok {
        vec![secure, url.to_owned()]
    } else {
        vec![secure]
    }
}

/// The feed addresses in `feeds.txt`: one per line, blank lines and `#`
/// comments skipped, repeats once.
pub fn feed_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}');
        let url = line.split('#').next().unwrap_or("").trim();
        if is_web_address(url) && !out.iter().any(|u| u == url) {
            out.push(url.to_owned());
        }
    }
    out
}

/// A folder name for a show: its title in characters every card's file
/// system takes, different from the names in `taken`.
pub fn folder_name(title: &str, taken: &[&str]) -> String {
    let mut base: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    base = base.trim_matches('.').chars().take(60).collect();
    if base.trim().is_empty() {
        base = "Podcast".into();
    }
    let base = base.trim().to_owned();
    let is_taken = |name: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(name));
    if !is_taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !is_taken(name))
        .unwrap_or(base)
}

/// What has been heard of one episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Listening {
    pub position_ms: u64,
    pub played: bool,
}

/// Listening and downloads, kept by the application between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PodcastState {
    /// By episode id.
    pub listening: BTreeMap<String, Listening>,
    /// Episodes chosen for download and not on the card yet, in the order
    /// chosen.
    pub wanted: Vec<String>,
    /// Episodes already considered for automatic download.
    pub seen: BTreeSet<String>,
    /// When the feeds were last read, unix seconds.
    pub checked: Option<i64>,
}

impl PodcastState {
    pub fn listening(&self, id: &str) -> Listening {
        self.listening.get(id).copied().unwrap_or_default()
    }

    /// Where playback of `id` starts: a little before where it stopped, or
    /// the beginning once played.
    pub fn resume_ms(&self, id: &str) -> u64 {
        let l = self.listening(id);
        if l.played {
            0
        } else {
            l.position_ms.saturating_sub(RESUME_REWIND_MS)
        }
    }

    /// Records the playing position of `id`; near the end (of `duration_ms`,
    /// when known) it counts as played. True when something changed.
    pub fn listened(&mut self, id: &str, position_ms: u64, duration_ms: u64) -> bool {
        let l = self.listening.entry(id.to_owned()).or_default();
        let before = *l;
        l.position_ms = position_ms;
        if duration_ms > END_SLACK_MS && position_ms + END_SLACK_MS >= duration_ms {
            l.played = true;
        }
        *l != before
    }

    /// Played to the end (or marked so); listening again starts over.
    pub fn mark_played(&mut self, id: &str) -> bool {
        let l = self.listening.entry(id.to_owned()).or_default();
        let changed = !l.played;
        *l = Listening {
            position_ms: 0,
            played: true,
        };
        changed
    }

    /// Choose `id` for download; false when it already waits.
    pub fn want(&mut self, id: &str) -> bool {
        if self.wanted.iter().any(|w| w == id) {
            return false;
        }
        self.wanted.push(id.to_owned());
        true
    }

    /// The episode's audio left the card (or its download was cancelled).
    pub fn forget(&mut self, id: &str) {
        self.wanted.retain(|w| w != id);
        self.listening.remove(id);
    }

    /// After the feeds were read at `now`: each show's newest episodes
    /// that were never considered before wait for download
    /// (`AUTO_DOWNLOAD` of them), and what refers to episodes no show lists
    /// any more is let go.
    pub fn refreshed(&mut self, shows: &[Show], on_card: impl Fn(&str) -> bool, now: i64) {
        for show in shows {
            let new: Vec<&Episode> = show
                .episodes
                .iter()
                .filter(|e| !self.seen.contains(&e.id))
                .collect();
            for e in new.iter().take(AUTO_DOWNLOAD) {
                if !on_card(&e.id) && !self.listening(&e.id).played {
                    self.want(&e.id);
                }
            }
            self.seen.extend(new.iter().map(|e| e.id.clone()));
        }
        let listed: BTreeSet<&str> = shows
            .iter()
            .flat_map(|s| s.episodes.iter().map(|e| e.id.as_str()))
            .collect();
        self.seen.retain(|id| listed.contains(id.as_str()));
        self.listening.retain(|id, _| listed.contains(id.as_str()));
        self.wanted.retain(|id| listed.contains(id.as_str()));
        self.checked = Some(now);
    }

    /// The next episode to download: the first chosen that is not on the
    /// card, with its show.
    pub fn next_download<'a>(
        &self,
        shows: &'a [Show],
        on_card: impl Fn(&str) -> bool,
    ) -> Option<(&'a Show, &'a Episode)> {
        self.wanted
            .iter()
            .filter(|id| !on_card(id))
            .find_map(|id| shows.iter().find_map(|s| s.episode(id).map(|e| (s, e))))
    }
}

/// A length in plain words: "42 min", "1 h 5 min", "2 h" (empty unknown).
pub fn length_text(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, 0) => String::new(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// When something came out, from `now`: "Today", "Yesterday", "3 days
/// ago", "2 weeks ago", "5 months ago", "2 years ago" (empty when unknown).
pub fn age_text(published: i64, now: i64) -> String {
    if published <= 0 {
        return String::new();
    }
    let days = (now - published).max(0) / 86_400;
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(id: &str, published: i64) -> Episode {
        Episode {
            id: id.into(),
            title: format!("Episode {id}"),
            published,
            duration_s: 600,
            url: format!("https://media.example.org/{id}.mp3"),
            ..Default::default()
        }
    }

    fn show(feed: &str, ids: &[(&str, i64)]) -> Show {
        Show {
            feed: feed.into(),
            title: feed.into(),
            episodes: ids.iter().map(|(id, p)| episode(id, *p)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn feed_lists_skip_comments_blank_lines_and_repeats() {
        let text = "\u{feff}# My shows\n\nhttps://feeds.example.org/a.xml\n  \
                    http://example.org/b.rss   # the kitchen one\n\
                    #https://example.org/off.xml\nhttps://feeds.example.org/a.xml\n\
                    not an address\nftp://example.org/c\n";
        assert_eq!(
            feed_urls(text),
            [
                "https://feeds.example.org/a.xml",
                "http://example.org/b.rss"
            ]
        );
        assert!(feed_urls("").is_empty());
    }

    #[test]
    fn https_is_tried_first_and_plain_http_only_for_http_feeds() {
        assert_eq!(
            https_first("HTTP://example.org/feed.xml", true),
            [
                "https://example.org/feed.xml",
                "HTTP://example.org/feed.xml"
            ]
        );
        assert_eq!(
            https_first("http://media.example.org/a.mp3", false),
            ["https://media.example.org/a.mp3"]
        );
        assert_eq!(
            https_first("https://media.example.org/a.mp3", true),
            ["https://media.example.org/a.mp3"]
        );
        assert!(is_plain_http("http://x") && !is_plain_http("https://x"));
    }

    #[test]
    fn episodes_have_stable_ids_and_file_names() {
        let a = episode_id("https://feeds.example.org/a.xml", "guid-1");
        assert_eq!(a.len(), 16);
        assert_eq!(a, episode_id("https://feeds.example.org/a.xml", " guid-1 "));
        assert_ne!(a, episode_id("https://feeds.example.org/b.xml", "guid-1"));
        let mut e = episode("abc", 0);
        assert_eq!(e.file_name(), "abc.mp3");
        e.url = "https://x.example.org/y/Episode.M4A?token=1#t".into();
        assert_eq!(e.file_name(), "abc.m4a");
        e.url = "https://x.example.org/stream".into();
        e.mime = "audio/x-m4a".into();
        assert_eq!(e.file_name(), "abc.m4a");
        e.mime = String::new();
        assert_eq!(e.file_name(), "abc.mp3");
    }

    #[test]
    fn show_folders_are_safe_and_distinct() {
        assert_eq!(folder_name("The Example Garden", &[]), "The Example Garden");
        assert_eq!(folder_name("What? / Why: <Now>", &[]), "What Why Now");
        assert_eq!(folder_name("...", &[]), "Podcast");
        assert_eq!(folder_name("Daily", &["daily", "Daily 2"]), "Daily 3");
        assert_eq!(folder_name(&"x".repeat(100), &[]).len(), 60);
    }

    #[test]
    fn a_refresh_keeps_episodes_still_on_the_card() {
        let old = Show {
            folder: "Garden".into(),
            ..show("f", &[("b", 200), ("a", 100)])
        };
        let fresh = show("f", &[("c", 300), ("b", 200)]);
        let new = old.refreshed(fresh.clone(), |id| id == "a");
        let ids: Vec<&str> = new.episodes.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
        assert_eq!(new.folder, "Garden");
        let gone = old.refreshed(fresh, |_| false);
        assert_eq!(gone.episodes.len(), 2);
    }

    #[test]
    fn positions_resume_and_the_end_counts_as_played() {
        let mut st = PodcastState::default();
        assert_eq!(st.resume_ms("a"), 0);
        assert!(st.listened("a", 120_000, 600_000));
        assert!(!st.listened("a", 120_000, 600_000));
        assert_eq!(st.resume_ms("a"), 115_000);
        assert!(!st.listening("a").played);
        // Unknown length: never played by position alone.
        st.listened("b", 9_000_000, 0);
        assert!(!st.listening("b").played);
        st.listened("a", 575_000, 600_000);
        assert!(st.listening("a").played);
        assert_eq!(st.resume_ms("a"), 0);
        assert!(st.mark_played("b"));
        assert!(!st.mark_played("b"));
        assert_eq!(st.listening("b").position_ms, 0);
        let bytes = serde_json::to_vec(&st).unwrap();
        assert_eq!(serde_json::from_slice::<PodcastState>(&bytes).unwrap(), st);
    }

    #[test]
    fn new_episodes_download_by_themselves_once() {
        let mut st = PodcastState::default();
        let shows = vec![
            show("f", &[("a2", 200), ("a1", 100)]),
            show("g", &[("b1", 50)]),
        ];
        st.refreshed(&shows, |_| false, 1000);
        assert_eq!(st.wanted, ["a2", "b1"]);
        assert_eq!(st.checked, Some(1000));
        let none = |_: &str| false;
        let (s, e) = st.next_download(&shows, none).unwrap();
        assert_eq!((s.feed.as_str(), e.id.as_str()), ("f", "a2"));
        // Downloaded, then deleted: it does not come back by itself.
        st.forget("a2");
        st.refreshed(&shows, |_| false, 2000);
        assert_eq!(st.wanted, ["b1"]);
        // A new episode: the newest only.
        let shows = vec![
            show("f", &[("a4", 400), ("a3", 300), ("a2", 200)]),
            show("g", &[("b1", 50)]),
        ];
        st.mark_played("b1");
        st.refreshed(&shows, |id| id == "b1", 3000);
        assert_eq!(st.wanted, ["b1", "a4"]);
        assert_eq!(
            st.next_download(&shows, |id| id == "b1").unwrap().1.id,
            "a4"
        );
        // Episodes no feed lists any more are let go.
        st.refreshed(&[show("f", &[("a4", 400)])], |_| false, 4000);
        assert_eq!(st.wanted, ["a4"]);
        assert!(st.listening.is_empty());
        assert_eq!(st.seen.len(), 1);
        assert!(st.want("a3") && !st.want("a3"));
    }

    #[test]
    fn lengths_and_ages_read_plainly() {
        assert_eq!(length_text(0), "");
        assert_eq!(length_text(59), "1 min");
        assert_eq!(length_text(3900), "1 h 5 min");
        assert_eq!(length_text(7200), "2 h");
        let (day, now) = (86_400, 1_760_000_000);
        assert_eq!(age_text(now - 3600, now), "Today");
        assert_eq!(age_text(now - day - 5, now), "Yesterday");
        assert_eq!(age_text(now - 3 * day, now), "3 days ago");
        assert_eq!(age_text(now - 20 * day, now), "2 weeks ago");
        assert_eq!(age_text(now - 100 * day, now), "3 months ago");
        assert_eq!(age_text(now - 800 * day, now), "2 years ago");
        assert_eq!(age_text(0, now), "");
    }
}

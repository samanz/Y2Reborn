//! The podcast folder on the SD card and the worker that fills it: reading
//! the shows kept there, refreshing them from their feeds and downloading
//! episodes run on their own threads (a download can take minutes); the UI
//! thread sends jobs and drains replies each pass.
use reborn_core::podcasts::{
    self, feed_urls, folder_name, Episode, Show, FEEDS_FILE, FOLDER, INDEX_FILE, MARKER,
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver, Sender},
};

/// How feeds and audio are fetched (curl on the player, fixtures in tests).
/// `plain_ok` allows plain HTTP for this address and its redirects.
pub trait Fetch {
    fn feed(&self, url: &str, plain_ok: bool) -> Result<Vec<u8>, String>;
    /// Writes the audio at `url` to `dest`.
    fn download(&self, url: &str, dest: &Path, plain_ok: bool) -> Result<(), String>;
}

struct Curl;

impl Fetch for Curl {
    fn feed(&self, url: &str, plain_ok: bool) -> Result<Vec<u8>, String> {
        reborn_platform::podcast_fetch::feed(url, plain_ok)
    }
    fn download(&self, url: &str, dest: &Path, plain_ok: bool) -> Result<(), String> {
        reborn_platform::podcast_fetch::download(url, dest, plain_ok)
    }
}

/// Tries `url` over HTTPS first (`podcasts::https_first`); the last
/// attempt's error is the one reported.
fn https_first<T>(
    url: &str,
    plain_ok: bool,
    mut get: impl FnMut(&str, bool) -> Result<T, String>,
) -> Result<T, String> {
    let mut result = Err("Not a web address".into());
    for address in podcasts::https_first(url, plain_ok) {
        result = get(&address, podcasts::is_plain_http(&address));
        if result.is_ok() {
            break;
        }
    }
    result
}

/// What the card holds: its shows and the episodes whose audio is there.
#[derive(Debug, Default)]
pub struct Card {
    pub shows: Vec<Show>,
    pub on_card: HashSet<String>,
}

/// Downloads smaller than this are error pages, not audio.
const MIN_AUDIO_BYTES: u64 = 16 * 1024;

/// The feed addresses in `Podcasts/feeds.txt` and `Podcasts/*.opml`.
pub fn feeds(card: &Path) -> Vec<String> {
    let dir = card.join(FOLDER);
    let mut out = fs::read(dir.join(FEEDS_FILE))
        .map(|b| feed_urls(&String::from_utf8_lossy(&b)))
        .unwrap_or_default();
    let mut lists: Vec<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("opml"))
        })
        .collect();
    lists.sort();
    for list in lists {
        let text = fs::read(&list).unwrap_or_default();
        for url in reborn_core::feed::opml_urls(&String::from_utf8_lossy(&text)) {
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

/// The shows kept on the card (those of the feed list first, in its order)
/// and which episodes' audio is there.
pub fn read_card(card: &Path) -> Card {
    let dir = card.join(FOLDER);
    let mut shows: Vec<Show> = vec![];
    let mut on_card = HashSet::new();
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let Ok(bytes) = fs::read(entry.path().join(INDEX_FILE)) else {
            continue;
        };
        let Ok(mut show) = serde_json::from_slice::<Show>(&bytes) else {
            continue;
        };
        show.folder = entry.file_name().to_string_lossy().into_owned();
        let files: HashSet<String> = fs::read_dir(entry.path())
            .into_iter()
            .flatten()
            .flatten()
            .map(|f| f.file_name().to_string_lossy().into_owned())
            .collect();
        on_card.extend(
            show.episodes
                .iter()
                .filter(|e| files.contains(&e.file_name()))
                .map(|e| e.id.clone()),
        );
        shows.push(show);
    }
    let order = feeds(card);
    shows.sort_by_key(|s| {
        (
            order
                .iter()
                .position(|f| *f == s.feed)
                .unwrap_or(usize::MAX),
            s.title.to_lowercase(),
        )
    });
    Card { shows, on_card }
}

/// The podcast folder, with the marker that keeps its audio out of Songs.
fn folder(card: &Path) -> Result<PathBuf, String> {
    let dir = card.join(FOLDER);
    fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make the Podcasts folder: {e}"))?;
    let marker = dir.join(MARKER);
    if !marker.exists() {
        fs::write(
            &marker,
            b"Kept by Reborn: these episodes are listed under Podcasts, not Songs.\n",
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Reads every listed feed and writes each show's index; shows already on
/// the card keep their folder and the episodes still there. Returns the
/// card as it is now and the feeds that could not be read.
pub fn refresh(card: &Path, fetch: &dyn Fetch) -> Result<(Card, Vec<String>), String> {
    let urls = feeds(card);
    if urls.is_empty() {
        return Err("No feeds in Podcasts/feeds.txt".into());
    }
    let dir = folder(card)?;
    let before = read_card(card);
    let mut taken: Vec<String> = before.shows.iter().map(|s| s.folder.clone()).collect();
    let mut failed = vec![];
    let mut first_error = None;
    for url in &urls {
        // The show keeps the address as listed (its episode ids depend on it).
        let fresh = https_first(url, podcasts::is_plain_http(url), |u, plain| {
            fetch.feed(u, plain)
        })
        .and_then(|bytes| reborn_core::feed::parse_rss(url, &String::from_utf8_lossy(&bytes)));
        let fresh = match fresh {
            Ok(show) => show,
            Err(e) => {
                first_error.get_or_insert(e);
                failed.push(url.clone());
                continue;
            }
        };
        let show = match before.shows.iter().find(|s| s.feed == *url) {
            Some(old) => old.refreshed(fresh, |id| before.on_card.contains(id)),
            None => {
                let names: Vec<&str> = taken.iter().map(String::as_str).collect();
                let folder = folder_name(&fresh.title, &names);
                taken.push(folder.clone());
                Show { folder, ..fresh }
            }
        };
        let bytes = serde_json::to_vec(&show).map_err(|e| e.to_string())?;
        reborn_core::atomic_write(&dir.join(&show.folder).join(INDEX_FILE), &bytes)
            .map_err(|e| format!("Couldn't write to the SD card: {e}"))?;
    }
    if failed.len() == urls.len() {
        return Err(first_error.unwrap_or_default());
    }
    Ok((read_card(card), failed))
}

/// Downloads an episode into its show's folder through a `.part` file
/// (`started` learns its path, for progress), renamed when complete. The
/// audio comes over HTTPS, or as written over plain HTTP when the show's
/// feed is listed as `http://`.
pub fn download(
    card: &Path,
    show_folder: &str,
    feed: &str,
    episode: &Episode,
    fetch: &dyn Fetch,
    started: impl FnOnce(&Path),
) -> Result<(), String> {
    let dir = folder(card)?.join(show_folder);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = episode.file_name();
    let dest = dir.join(&name);
    if dest.is_file() {
        return Ok(());
    }
    let part = dir.join(format!("{name}.part"));
    let _ = fs::remove_file(&part);
    started(&part);
    let plain_ok = podcasts::is_plain_http(feed);
    let fetched = https_first(&episode.url, plain_ok, |u, plain| {
        let _ = fs::remove_file(&part);
        fetch.download(u, &part, plain)
    })
    .and_then(|()| {
        let size = fs::metadata(&part).map_or(0, |m| m.len());
        if size < MIN_AUDIO_BYTES {
            return Err("The download was empty".into());
        }
        fs::rename(&part, &dest).map_err(|e| e.to_string())
    });
    if fetched.is_err() {
        let _ = fs::remove_file(&part);
    }
    fetched
}

/// Deletes an episode's audio (and any partial download) from the card.
pub fn delete(card: &Path, show_folder: &str, episode: &Episode) {
    let dir = card.join(FOLDER).join(show_folder);
    let name = episode.file_name();
    let _ = fs::remove_file(dir.join(format!("{name}.part")));
    let _ = fs::remove_file(dir.join(name));
}

pub enum Reply {
    /// The card at this path was read.
    Read(PathBuf, Card),
    Refreshed(PathBuf, Result<(Card, Vec<String>), String>),
    /// A download began; its partial file grows at this path.
    Started {
        id: String,
        part: PathBuf,
    },
    Downloaded {
        id: String,
        result: Result<(), String>,
    },
}

enum FeedJob {
    Read(PathBuf),
    Refresh(PathBuf),
}

struct DownloadJob {
    card: PathBuf,
    folder: String,
    feed: String,
    episode: Episode,
}

pub struct Worker {
    feeds: Sender<FeedJob>,
    downloads: Sender<DownloadJob>,
    pub replies: Receiver<Reply>,
}

impl Worker {
    pub fn spawn() -> Result<Self, std::io::Error> {
        let (reply, replies) = channel();
        let (feeds, feed_jobs) = channel::<FeedJob>();
        let (downloads, download_jobs) = channel::<DownloadJob>();
        let tx = reply.clone();
        std::thread::Builder::new()
            .name("podcast-feeds".into())
            .spawn(move || {
                while let Ok(job) = feed_jobs.recv() {
                    let sent = match job {
                        FeedJob::Read(card) => {
                            let read = read_card(&card);
                            tx.send(Reply::Read(card, read))
                        }
                        FeedJob::Refresh(card) => {
                            let result = refresh(&card, &Curl);
                            tx.send(Reply::Refreshed(card, result))
                        }
                    };
                    if sent.is_err() {
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("podcast-download".into())
            .spawn(move || {
                while let Ok(job) = download_jobs.recv() {
                    let id = job.episode.id.clone();
                    let result = download(
                        &job.card,
                        &job.folder,
                        &job.feed,
                        &job.episode,
                        &Curl,
                        |part| {
                            let _ = reply.send(Reply::Started {
                                id: id.clone(),
                                part: part.to_owned(),
                            });
                        },
                    );
                    if reply.send(Reply::Downloaded { id, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            feeds,
            downloads,
            replies,
        })
    }

    pub fn read(&self, card: PathBuf) -> bool {
        self.feeds.send(FeedJob::Read(card)).is_ok()
    }

    pub fn refresh(&self, card: PathBuf) -> bool {
        self.feeds.send(FeedJob::Refresh(card)).is_ok()
    }

    pub fn download(&self, card: PathBuf, show: &Show, episode: Episode) -> bool {
        self.downloads
            .send(DownloadJob {
                card,
                folder: show.folder.clone(),
                feed: show.feed.clone(),
                episode,
            })
            .is_ok()
    }
}

/// Where listening and downloads are kept between runs.
pub fn state_path(root: &Path) -> PathBuf {
    root.join("state/podcasts.json")
}

pub fn load_state(root: &Path) -> podcasts::PodcastState {
    fs::read(state_path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    /// Feeds and audio from memory; records what was asked for.
    #[derive(Default)]
    struct Fixture {
        feeds: HashMap<String, String>,
        audio: HashMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Fixture {
        /// Like curl's --proto: plain HTTP only when allowed.
        fn ask(&self, url: &str, plain_ok: bool) -> Result<(), String> {
            self.asked.borrow_mut().push(url.into());
            if podcasts::is_plain_http(url) && !plain_ok {
                return Err("Protocol not allowed".into());
            }
            Ok(())
        }
    }

    impl Fetch for Fixture {
        fn feed(&self, url: &str, plain_ok: bool) -> Result<Vec<u8>, String> {
            self.ask(url, plain_ok)?;
            self.feeds
                .get(url)
                .map(|f| f.clone().into_bytes())
                .ok_or_else(|| "The podcast's server refused the request".into())
        }
        fn download(&self, url: &str, dest: &Path, plain_ok: bool) -> Result<(), String> {
            self.ask(url, plain_ok)?;
            let bytes = self.audio.get(url).ok_or("No internet connection")?;
            fs::write(dest, bytes).map_err(|e| e.to_string())
        }
    }

    fn rss(title: &str, items: &[(&str, &str)]) -> String {
        let items: String = items
            .iter()
            .map(|(guid, date)| {
                format!(
                    "<item><title>{guid}</title><guid>{guid}</guid><pubDate>{date}</pubDate>\
                     <enclosure url=\"https://media.example.org/{guid}.mp3\" length=\"20000\" \
                     type=\"audio/mpeg\"/></item>"
                )
            })
            .collect();
        format!("<rss><channel><title>{title}</title>{items}</channel></rss>")
    }

    fn card(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reborn-pod-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(FOLDER)).unwrap();
        dir
    }

    const GARDEN: &str = "https://feeds.example.org/garden.xml";
    const KITCHEN: &str = "https://feeds.example.org/kitchen.xml";

    #[test]
    fn feed_lists_come_from_feeds_txt_and_opml_files() {
        let card = card("lists");
        fs::write(
            card.join(FOLDER).join(FEEDS_FILE),
            format!("# shows\n{GARDEN}\n"),
        )
        .unwrap();
        fs::write(
            card.join(FOLDER).join("Exported.OPML"),
            format!("<opml><body><outline xmlUrl=\"{KITCHEN}\"/><outline xmlUrl=\"{GARDEN}\"/></body></opml>"),
        )
        .unwrap();
        assert_eq!(feeds(&card), [GARDEN, KITCHEN]);
        fs::remove_dir_all(card).unwrap();
    }

    #[test]
    fn a_refresh_writes_each_show_index_and_keeps_its_folder() {
        let card = card("refresh");
        assert!(refresh(&card, &Fixture::default()).is_err());
        fs::write(
            card.join(FOLDER).join(FEEDS_FILE),
            format!("{GARDEN}\n{KITCHEN}\nhttps://feeds.example.org/gone.xml\n"),
        )
        .unwrap();
        let mut fetch = Fixture::default();
        fetch.feeds.insert(
            GARDEN.into(),
            rss("Garden", &[("g1", "01 Oct 2025 00:00:00 GMT")]),
        );
        fetch.feeds.insert(
            KITCHEN.into(),
            rss("Kitchen", &[("k1", "02 Oct 2025 00:00:00 GMT")]),
        );
        let (now, failed) = refresh(&card, &fetch).unwrap();
        assert_eq!(failed, ["https://feeds.example.org/gone.xml"]);
        let folders: Vec<&str> = now.shows.iter().map(|s| s.folder.as_str()).collect();
        assert_eq!(folders, ["Garden", "Kitchen"]);
        assert!(card.join(FOLDER).join(MARKER).is_file());
        assert!(card.join(FOLDER).join("Garden").join(INDEX_FILE).is_file());
        // The show renames itself and drops g1 while its audio is on the
        // card: same folder, g1 kept.
        let g1 = now.shows[0].episodes[0].clone();
        fs::write(card.join(FOLDER).join("Garden").join(g1.file_name()), b"x").unwrap();
        fetch.feeds.insert(
            GARDEN.into(),
            rss("The Garden", &[("g2", "08 Oct 2025 00:00:00 GMT")]),
        );
        let (now, _) = refresh(&card, &fetch).unwrap();
        let garden = &now.shows[0];
        assert_eq!(
            (garden.folder.as_str(), garden.title.as_str()),
            ("Garden", "The Garden")
        );
        let titles: Vec<&str> = garden.episodes.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["g2", "g1"]);
        assert!(now.on_card.contains(&g1.id));
        assert_eq!(read_card(&card).shows.len(), 2);
        // Nothing readable at all is a failure, and the card is unchanged.
        let broken = Fixture::default();
        assert_eq!(
            refresh(&card, &broken).unwrap_err(),
            "The podcast's server refused the request"
        );
        assert_eq!(read_card(&card).shows[0].title, "The Garden");
        fs::remove_dir_all(card).unwrap();
    }

    #[test]
    fn downloads_go_through_a_part_file_and_empty_ones_are_refused() {
        let card = card("download");
        let episode = Episode {
            id: "0123456789abcdef".into(),
            url: "https://media.example.org/g1.mp3".into(),
            ..Default::default()
        };
        let mut fetch = Fixture::default();
        fetch
            .audio
            .insert(episode.url.clone(), vec![7; MIN_AUDIO_BYTES as usize]);
        let mut part = None;
        download(&card, "Garden", GARDEN, &episode, &fetch, |p| {
            part = Some(p.to_owned())
        })
        .unwrap();
        let dir = card.join(FOLDER).join("Garden");
        assert_eq!(part.unwrap(), dir.join("0123456789abcdef.mp3.part"));
        assert!(dir.join("0123456789abcdef.mp3").is_file());
        assert!(!dir.join("0123456789abcdef.mp3.part").exists());
        // Already there: nothing is fetched again.
        fetch.asked.borrow_mut().clear();
        download(&card, "Garden", GARDEN, &episode, &fetch, |_| {}).unwrap();
        assert!(fetch.asked.borrow().is_empty());
        delete(&card, "Garden", &episode);
        assert!(!dir.join("0123456789abcdef.mp3").exists());
        // An error page instead of audio, and a failed transfer, leave nothing.
        fetch.audio.insert(episode.url.clone(), b"<html>".to_vec());
        assert_eq!(
            download(&card, "Garden", GARDEN, &episode, &fetch, |_| {}).unwrap_err(),
            "The download was empty"
        );
        fetch.audio.clear();
        assert!(download(&card, "Garden", GARDEN, &episode, &fetch, |_| {}).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(card).unwrap();
    }

    #[test]
    fn https_comes_first_and_plain_http_only_for_feeds_listed_as_http() {
        let card = card("https");
        let plain = "http://feeds.example.org/garden.xml";
        fs::write(card.join(FOLDER).join(FEEDS_FILE), format!("{plain}\n")).unwrap();
        let mut fetch = Fixture::default();
        fetch.feeds.insert(
            GARDEN.into(),
            rss("Garden", &[("g1", "01 Oct 2025 00:00:00 GMT")]),
        );
        // Served over HTTPS too: plain HTTP is never asked for, and the show
        // keeps the address as listed.
        let (now, _) = refresh(&card, &fetch).unwrap();
        assert_eq!(*fetch.asked.borrow(), [GARDEN]);
        assert_eq!(now.shows[0].feed, plain);
        // Only over HTTP: asked for after HTTPS.
        fetch.feeds.clear();
        fetch.feeds.insert(
            plain.into(),
            rss("Garden", &[("g1", "01 Oct 2025 00:00:00 GMT")]),
        );
        fetch.asked.borrow_mut().clear();
        refresh(&card, &fetch).unwrap();
        assert_eq!(*fetch.asked.borrow(), [GARDEN, plain]);
        // An http:// enclosure: plain HTTP only when its feed is http:// too.
        let episode = Episode {
            id: "0123456789abcdef".into(),
            url: "http://media.example.org/g1.mp3".into(),
            ..Default::default()
        };
        fetch
            .audio
            .insert(episode.url.clone(), vec![7; MIN_AUDIO_BYTES as usize]);
        fetch.asked.borrow_mut().clear();
        assert!(download(&card, "Garden", GARDEN, &episode, &fetch, |_| {}).is_err());
        assert_eq!(*fetch.asked.borrow(), ["https://media.example.org/g1.mp3"]);
        download(&card, "Garden", plain, &episode, &fetch, |_| {}).unwrap();
        assert!(card
            .join(FOLDER)
            .join("Garden")
            .join("0123456789abcdef.mp3")
            .is_file());
        fs::remove_dir_all(card).unwrap();
    }

    #[test]
    fn missing_or_broken_state_starts_empty() {
        let root = card("state");
        assert_eq!(load_state(&root), Default::default());
        fs::create_dir_all(root.join("state")).unwrap();
        fs::write(state_path(&root), b"{not json").unwrap();
        assert_eq!(load_state(&root), Default::default());
        fs::remove_dir_all(root).unwrap();
    }
}

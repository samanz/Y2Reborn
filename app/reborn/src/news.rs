//! The headline worker: feeds are fetched on their own thread; the UI thread
//! sends requests and drains replies each pass. The feeds come from
//! `News/feeds.txt` on the SD card or in the internal music folder; the last
//! feed read, its headlines and the reader's text size are kept in
//! `news.json`.
use reborn_core::{
    news::{parse_feeds, Feed, Headline, FEEDS_FILE},
    Source,
};
use reborn_ui::news::NewsView;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::mpsc::{sync_channel, Receiver, SyncSender},
};

const FILE: &str = "news.json";

pub struct Worker {
    requests: SyncSender<String>,
    /// The feed's address and its headlines (or why they couldn't be had).
    pub replies: Receiver<(String, Result<Vec<Headline>, String>)>,
}

impl Worker {
    pub fn spawn() -> Result<Self, std::io::Error> {
        let (requests, rx) = sync_channel::<String>(2);
        let (tx, replies) = sync_channel(2);
        std::thread::Builder::new()
            .name("news".into())
            .spawn(move || {
                while let Ok(url) = rx.recv() {
                    let result = reborn_platform::news::fetch(&url);
                    if tx.send((url, result)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { requests, replies })
    }

    /// False when the worker is still busy with earlier requests.
    pub fn send(&self, url: String) -> bool {
        self.requests.try_send(url).is_ok()
    }
}

/// The feeds listed on the first source that has a feeds file (a file of
/// no usable lines counts as none), else none.
pub fn feeds(sources: &[Source]) -> Vec<Feed> {
    sources
        .iter()
        .filter(|s| s.online)
        .filter_map(|s| fs::read_to_string(s.root.join(FEEDS_FILE)).ok())
        .map(|text| parse_feeds(&text))
        .find(|f| !f.is_empty())
        .unwrap_or_default()
}

#[derive(Serialize, Deserialize)]
struct Saved {
    feed: String,
    items: Vec<Headline>,
    text_size: Option<u8>,
}

/// The saved headlines, if any (a file that does not parse is ignored).
pub fn load(root: &Path, view: &mut NewsView) {
    if let Some(saved) = fs::read(root.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
    {
        view.feed = saved.feed;
        view.items = saved.items;
        view.text_size = saved.text_size;
    }
}

pub fn save(root: &Path, view: &NewsView) -> Result<(), String> {
    let saved = Saved {
        feed: view.feed.clone(),
        items: view.items.clone(),
        text_size: view.text_size,
    };
    let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
    reborn_core::atomic_write(&root.join(FILE), &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::MediaSource;

    fn source(root: &Path, online: bool) -> Source {
        Source {
            id: "sd".into(),
            kind: MediaSource::SdCard("card".into()),
            root: root.to_owned(),
            online,
            mount: String::new(),
            mount_id: None,
        }
    }

    #[test]
    fn feeds_come_only_from_the_card() {
        let root = std::env::temp_dir().join(format!("reborn-feeds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("News")).unwrap();
        assert!(feeds(&[source(&root, true)]).is_empty());
        fs::write(
            root.join(FEEDS_FILE),
            "https://feeds.example.org/top.xml Top Stories\n",
        )
        .unwrap();
        let card = feeds(&[source(&root, true)]);
        assert_eq!(card.len(), 1);
        assert_eq!(card[0].name, "Top Stories");
        // A card that is not mounted is not read.
        assert!(feeds(&[source(&root, false)]).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn headlines_and_text_size_survive_a_restart() {
        let root = std::env::temp_dir().join(format!("reborn-news-saved-{}", std::process::id()));
        let mut view = NewsView {
            feed: "https://feeds.example.org/top.xml".into(),
            items: vec![Headline {
                title: "A headline".into(),
                summary: String::new(),
                published: None,
                link: String::new(),
            }],
            text_size: Some(2),
            ..Default::default()
        };
        save(&root, &view).unwrap();
        view = NewsView::default();
        load(&root, &mut view);
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.text_size, Some(2));
        fs::remove_dir_all(root).unwrap();
    }
}

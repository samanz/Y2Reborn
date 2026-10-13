//! Full news stories: fetched when one opens, and saved in the background
//! after headlines refresh so they read away from Wi-Fi. Two threads share
//! the saved stories under `<root>/news/`: one answers the story being
//! opened (from the saved copy when there is one), the other saves the top
//! stories of the feed just refreshed. The saved set is bounded: the least
//! recently read go first (`reborn_core::news::ArticleCache`).
use reborn_core::news::{cache_name, Article, ArticleCache, CACHE_BYTES, CACHE_STORIES};
use std::{
    fs,
    path::PathBuf,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// Stories saved after each refresh of a feed.
pub const PREFETCH: usize = 10;
const INDEX: &str = "index.json";

pub enum Reply {
    /// The story asked for (or why it couldn't be had).
    Article(String, Result<Article, String>),
    /// The saved stories now, and how many are still to be saved in the
    /// background (None: unchanged).
    Saved {
        urls: Vec<String>,
        pending: Option<usize>,
    },
}

pub struct Articles {
    /// A link, and whether Wi-Fi is up to fetch it if it is not saved.
    open: Sender<(String, bool)>,
    prefetch: Sender<Vec<String>>,
    pub replies: Receiver<Reply>,
}

impl Articles {
    pub fn spawn(dir: PathBuf) -> Result<Self, std::io::Error> {
        let store = Arc::new(Mutex::new(Store::load(dir)));
        let (open, open_rx) = channel::<(String, bool)>();
        let (prefetch, prefetch_rx) = channel::<Vec<String>>();
        let (tx, replies) = channel();
        let urls = lock(&store).urls();
        let _ = tx.send(Reply::Saved {
            urls,
            pending: Some(0),
        });
        {
            let store = store.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("news-open".into())
                .spawn(move || {
                    while let Ok(mut request) = open_rx.recv() {
                        // Only the latest story asked for matters.
                        while let Ok(newer) = open_rx.try_recv() {
                            request = newer;
                        }
                        let (url, online) = request;
                        let saved = lock(&store).get(&url);
                        let result = match saved {
                            Some(a) => Ok(a),
                            None if !online => Err("Not saved for reading offline".into()),
                            None => {
                                let fetched = reborn_platform::article::fetch(&url);
                                if let Ok(a) = &fetched {
                                    let mut s = lock(&store);
                                    s.put(a);
                                    let _ = tx.send(Reply::Saved {
                                        urls: s.urls(),
                                        pending: None,
                                    });
                                }
                                fetched
                            }
                        };
                        if tx.send(Reply::Article(url, result)).is_err() {
                            break;
                        }
                    }
                })?;
        }
        std::thread::Builder::new()
            .name("news-save".into())
            .spawn(move || {
                while let Ok(mut batch) = prefetch_rx.recv() {
                    let mut at = 0;
                    while at < batch.len() {
                        // A newer refresh replaces what is left to save.
                        if let Ok(newer) = prefetch_rx.try_recv() {
                            batch = newer;
                            at = 0;
                            continue;
                        }
                        let url = batch[at].clone();
                        at += 1;
                        if lock(&store).index.contains(&url) {
                            continue;
                        }
                        if let Ok(a) = reborn_platform::article::fetch(&url) {
                            lock(&store).put(&a);
                        }
                        let reply = Reply::Saved {
                            urls: lock(&store).urls(),
                            pending: Some(batch.len() - at),
                        };
                        if tx.send(reply).is_err() {
                            return;
                        }
                    }
                    let urls = lock(&store).urls();
                    let done = Reply::Saved {
                        urls,
                        pending: Some(0),
                    };
                    if tx.send(done).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self {
            open,
            prefetch,
            replies,
        })
    }

    /// Ask for a story (answered with `Reply::Article`); without Wi-Fi only
    /// a saved copy can answer.
    pub fn open(&self, url: String, online: bool) -> bool {
        self.open.send((url, online)).is_ok()
    }

    /// Save these stories in the background (those not saved already).
    pub fn prefetch(&self, urls: Vec<String>) -> bool {
        self.prefetch.send(urls).is_ok()
    }
}

fn lock(store: &Mutex<Store>) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The saved stories: one JSON file each, and an index of when each was
/// last used.
struct Store {
    dir: PathBuf,
    index: ArticleCache,
}

impl Store {
    fn load(dir: PathBuf) -> Self {
        let mut index: ArticleCache = fs::read(dir.join(INDEX))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        index
            .entries
            .retain(|e| dir.join(cache_name(&e.url)).is_file());
        Store { dir, index }
    }

    fn urls(&self) -> Vec<String> {
        self.index.entries.iter().map(|e| e.url.clone()).collect()
    }

    fn get(&mut self, url: &str) -> Option<Article> {
        if !self.index.contains(url) {
            return None;
        }
        let article = fs::read(self.dir.join(cache_name(url)))
            .ok()
            .and_then(|b| serde_json::from_slice::<Article>(&b).ok());
        match &article {
            Some(_) => self.index.touch(url, unix_now()),
            None => self.index.remove(url),
        }
        self.save_index();
        article
    }

    fn put(&mut self, article: &Article) {
        let Ok(bytes) = serde_json::to_vec(article) else {
            return;
        };
        let path = self.dir.join(cache_name(&article.url));
        if reborn_core::atomic_write(&path, &bytes).is_err() {
            return;
        }
        let evicted = self.index.insert(
            &article.url,
            bytes.len() as u64,
            unix_now(),
            CACHE_STORIES,
            CACHE_BYTES,
        );
        for url in evicted {
            let _ = fs::remove_file(self.dir.join(cache_name(&url)));
        }
        self.save_index();
    }

    fn save_index(&self) {
        if let Ok(bytes) = serde_json::to_vec(&self.index) {
            let _ = reborn_core::atomic_write(&self.dir.join(INDEX), &bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::news::{Block, BlockKind};

    fn article(url: &str, words: usize) -> Article {
        Article {
            url: url.into(),
            title: "t".into(),
            byline: String::new(),
            blocks: vec![Block {
                kind: BlockKind::Paragraph,
                text: "word ".repeat(words),
            }],
            paywalled: false,
        }
    }

    #[test]
    fn saved_stories_survive_a_restart_and_stay_bounded() {
        let dir = std::env::temp_dir().join(format!("reborn-news-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut s = Store::load(dir.clone());
        assert!(s.get("https://a").is_none());
        for i in 0..CACHE_STORIES + 5 {
            s.put(&article(&format!("https://e/{i}"), 10));
        }
        assert_eq!(s.index.entries.len(), CACHE_STORIES);
        // The oldest went, with their files.
        assert!(!dir.join(cache_name("https://e/0")).exists());
        assert!(dir.join(cache_name("https://e/5")).exists());
        let files = fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, CACHE_STORIES + 1, "stories and the index");
        let mut again = Store::load(dir.clone());
        assert_eq!(again.urls().len(), CACHE_STORIES);
        assert_eq!(again.get("https://e/7"), Some(article("https://e/7", 10)));
        // A story whose file went missing is dropped from the index.
        fs::remove_file(dir.join(cache_name("https://e/8"))).unwrap();
        assert!(again.get("https://e/8").is_none());
        assert!(!Store::load(dir.clone()).index.contains("https://e/8"));
        let _ = fs::remove_dir_all(&dir);
    }
}

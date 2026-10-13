//! News: headlines from RSS and Atom feeds, and full stories saved for
//! reading away from Wi-Fi. The feeds are listed in `News/feeds.txt` on the
//! SD card (or the internal music folder), one per line; there are no
//! built-in feeds.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Headline {
    pub title: String,
    /// Plain-text summary (HTML removed).
    pub summary: String,
    /// Publication time, Unix seconds, when the feed gives one.
    pub published: Option<i64>,
    /// The story's web page (empty when the feed gives none).
    pub link: String,
}

/// One feed: its address and the name shown for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub url: String,
    pub name: String,
}

/// Where the list of feeds lives, relative to the card or music folder.
pub const FEEDS_FILE: &str = "News/feeds.txt";

/// At most this many feeds are read from the file.
const MAX_FEEDS: usize = 30;

/// Feeds from the text of `feeds.txt`: one address per line, optionally
/// followed by a name ("https://example.com/rss.xml Example News"); blank
/// lines and lines starting with `#` are skipped. Feeds are fetched over
/// HTTPS, so `http://` addresses are read as `https://`.
pub fn parse_feeds(text: &str) -> Vec<Feed> {
    let mut feeds: Vec<Feed> = vec![];
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (url, name) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(u, n)| (u, n.trim()));
        let url = match url.strip_prefix("http://") {
            Some(rest) => format!("https://{rest}"),
            None if url.starts_with("https://") => url.to_owned(),
            None => continue,
        };
        if host(&url).is_empty() || feeds.iter().any(|f| f.url == url) {
            continue;
        }
        let name = if name.is_empty() {
            host(&url).trim_start_matches("www.").to_owned()
        } else {
            name.to_owned()
        };
        feeds.push(Feed { url, name });
        if feeds.len() == MAX_FEEDS {
            break;
        }
    }
    feeds
}

/// The host of an address ("feeds.example.com").
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or("")
}

/// A full story, as extracted from its page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Article {
    /// The story link from the feed (the cache key).
    pub url: String,
    pub title: String,
    /// "Jane Doe" or "Jane Doe and John Roe" (empty when unknown).
    pub byline: String,
    pub blocks: Vec<Block>,
    /// The site marks the story as for subscribers; `blocks` hold the part
    /// it shows everyone.
    pub paywalled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Paragraph,
    Heading,
    Quote,
    Item,
}

/// Unhurried reading pace, words per minute.
const WORDS_PER_MINUTE: usize = 220;

/// Whole minutes to read `words`, at least one.
pub fn reading_minutes(words: usize) -> usize {
    words.div_ceil(WORDS_PER_MINUTE).max(1)
}

impl Article {
    pub fn words(&self) -> usize {
        self.blocks
            .iter()
            .map(|b| b.text.split_whitespace().count())
            .sum()
    }
}

/// Saved stories: which are kept and when each was last read or saved, so
/// the oldest go when the store is full.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleCache {
    pub entries: Vec<CacheEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub url: String,
    /// Size of the saved file, bytes.
    pub bytes: u64,
    /// Last saved or opened, Unix seconds.
    pub used: i64,
}

/// At most this many stories are kept...
pub const CACHE_STORIES: usize = 60;
/// ...and at most this many bytes of them.
pub const CACHE_BYTES: u64 = 3 * 1024 * 1024;

impl ArticleCache {
    pub fn contains(&self, url: &str) -> bool {
        self.entries.iter().any(|e| e.url == url)
    }

    /// Mark a story as just read.
    pub fn touch(&mut self, url: &str, now: i64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.url == url) {
            e.used = now;
        }
    }

    /// Record a saved story; returns the stories to delete (least recently
    /// used first) so the store stays within `max_stories` and `max_bytes`.
    /// The story just saved is never among them.
    pub fn insert(
        &mut self,
        url: &str,
        bytes: u64,
        now: i64,
        max_stories: usize,
        max_bytes: u64,
    ) -> Vec<String> {
        self.entries.retain(|e| e.url != url);
        self.entries.push(CacheEntry {
            url: url.to_owned(),
            bytes,
            used: now,
        });
        // Newest last; ties keep insertion order.
        self.entries.sort_by_key(|e| e.used);
        let mut evicted = vec![];
        let total = |entries: &[CacheEntry]| entries.iter().map(|e| e.bytes).sum::<u64>();
        while self.entries.len() > 1
            && (self.entries.len() > max_stories || total(&self.entries) > max_bytes)
        {
            let at = self.entries.iter().position(|e| e.url != url).unwrap_or(0);
            evicted.push(self.entries.remove(at).url);
        }
        evicted
    }

    pub fn remove(&mut self, url: &str) {
        self.entries.retain(|e| e.url != url);
    }
}

/// A story's file name in the store: a hash of its link.
pub fn cache_name(url: &str) -> String {
    // FNV-1a, 64 bits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}.json")
}

/// "5 min ago", "3 h ago", "2 d ago" (None for future or unknown times).
pub fn age(published: Option<i64>, now: i64) -> Option<String> {
    let secs = now - published?;
    if secs < 0 {
        return None;
    }
    Some(match secs {
        0..=59 => "Just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(age(Some(1000), 1030).as_deref(), Some("Just now"));
        assert_eq!(age(Some(1000), 1000 + 600).as_deref(), Some("10 min ago"));
        assert_eq!(age(Some(0), 7300).as_deref(), Some("2 h ago"));
        assert_eq!(age(Some(0), 3 * 86_400).as_deref(), Some("3 d ago"));
        assert_eq!(age(Some(5000), 10), None);
        assert_eq!(age(None, 10), None);
    }

    #[test]
    fn reading_time_rounds_up_and_is_at_least_a_minute() {
        assert_eq!(reading_minutes(0), 1);
        assert_eq!(reading_minutes(220), 1);
        assert_eq!(reading_minutes(221), 2);
        assert_eq!(reading_minutes(900), 5);
        let article = Article {
            url: "u".into(),
            title: "t".into(),
            byline: String::new(),
            blocks: vec![Block {
                kind: BlockKind::Paragraph,
                text: "word ".repeat(30),
            }],
            paywalled: false,
        };
        assert_eq!(article.words(), 30);
    }

    #[test]
    fn feeds_come_from_a_plain_list_with_optional_names() {
        let text = "# My feeds\n\
                    https://feeds.example.org/top.xml  Top Stories\n\
                    \n\
                    http://www.example.net/rss\n\
                    ftp://example.com/feed\n\
                    https://feeds.example.org/top.xml Again\n\
                    not a feed\n";
        let feeds = parse_feeds(text);
        assert_eq!(
            feeds,
            [
                Feed {
                    url: "https://feeds.example.org/top.xml".into(),
                    name: "Top Stories".into(),
                },
                Feed {
                    url: "https://www.example.net/rss".into(),
                    name: "example.net".into(),
                },
            ]
        );
        assert!(parse_feeds("").is_empty());
        let many = "https://e.com/a\n".repeat(5)
            + &(0..40)
                .map(|i| format!("https://e{i}.com/\n"))
                .collect::<String>();
        assert_eq!(parse_feeds(&many).len(), MAX_FEEDS);
    }

    #[test]
    fn cache_evicts_least_recently_used_within_both_limits() {
        let mut c = ArticleCache::default();
        assert!(c.insert("a", 100, 1, 3, 1000).is_empty());
        assert!(c.insert("b", 100, 2, 3, 1000).is_empty());
        assert!(c.insert("c", 100, 3, 3, 1000).is_empty());
        // Reading "a" makes "b" the oldest.
        c.touch("a", 4);
        assert_eq!(c.insert("d", 100, 5, 3, 1000), ["b"]);
        assert!(c.contains("a") && !c.contains("b"));
        // Too many bytes: the oldest go until it fits.
        assert_eq!(c.insert("e", 850, 6, 3, 1000), ["c", "a"]);
        assert_eq!(
            c.entries.iter().map(|e| e.url.as_str()).collect::<Vec<_>>(),
            ["d", "e"]
        );
        // Saving a story again replaces its entry.
        assert!(c.insert("d", 150, 7, 3, 1000).is_empty());
        assert_eq!(c.entries.len(), 2);
        // A story larger than the whole store is still kept, alone.
        assert_eq!(c.insert("huge", 5000, 8, 3, 1000), ["e", "d"]);
        assert!(c.contains("huge"));
    }

    #[test]
    fn cache_names_are_stable_file_names() {
        let a = cache_name("https://news.example.com/articles/x?source=rss");
        assert_eq!(
            a,
            cache_name("https://news.example.com/articles/x?source=rss")
        );
        assert_ne!(a, cache_name("https://news.example.com/articles/y"));
        assert!(a.ends_with(".json") && a.len() == 21);
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'));
    }
}

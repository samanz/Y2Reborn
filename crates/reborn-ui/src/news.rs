//! News: headlines from the feeds listed on the card, fetched over Wi-Fi by
//! the app. Feeds and headlines
//! are ordinary settings-style lists; a story opens in the reader
//! (`reader.rs`) with its full text, fetched when it opens or saved earlier
//! for reading away from Wi-Fi.
use crate::{
    canvas::Canvas,
    reader::{self, ArticleState, Reader, Story},
    text::Face,
    theme::{color, WIDTH},
    Item, RowKind, Ui,
};
use reborn_core::{
    news::{age, host, Article, Feed, Headline, FEEDS_FILE},
    Action, AppModel, Effect, Screen,
};
use std::{
    collections::HashSet,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Headlines older than this are refreshed when their feed opens.
pub const STALE_AFTER: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Default)]
pub struct NewsView {
    /// The feeds to choose from (the app reads them from the card).
    pub feeds: Vec<Feed>,
    /// Address of the feed the headlines belong to.
    pub feed: String,
    pub items: Vec<Headline>,
    /// When this session last fetched `items` (None: loaded from disk).
    pub fetched: Option<Instant>,
    pub busy: bool,
    pub error: Option<String>,
    /// Story links whose full text is saved on the player (kept by the app).
    pub saved: HashSet<String>,
    /// Stories still being saved in the background after a refresh.
    pub saving: usize,
    /// The story open in the reader.
    pub reader: Reader,
    /// Reader text size (`reader::SIZES` index; None: Medium), remembered
    /// by the app.
    pub text_size: Option<u8>,
    /// The text size changed and should be saved.
    pub dirty: bool,
    /// Unix time to show ages at instead of the clock (previews and tests).
    pub clock: Option<i64>,
}

impl NewsView {
    pub fn stale(&self, now: Instant) -> bool {
        self.fetched
            .is_none_or(|t| now.saturating_duration_since(t) >= STALE_AFTER)
    }

    /// The reader's text size (`reader::SIZES` index).
    pub fn size(&self) -> u8 {
        self.text_size.unwrap_or(reader::DEFAULT_SIZE).min(2)
    }

    /// The reader is gliding to a new position (keep drawing).
    pub fn scrolling(&self) -> bool {
        self.reader.scroll.moving(Instant::now())
    }

    /// The name shown for the feed at `url`.
    pub fn feed_name<'a>(&'a self, url: &'a str) -> &'a str {
        self.feeds
            .iter()
            .find(|f| f.url == url)
            .map_or_else(|| host(url), |f| f.name.as_str())
    }

    /// Open story `index` in the reader. Returns the link to fetch (or load
    /// from the saved copy), if it has one.
    pub fn open(&mut self, index: usize) -> Option<String> {
        let link = self.items.get(index).map_or("", |h| h.link.as_str());
        self.reader = Reader::open(link);
        Some(link.to_owned()).filter(|l| !l.is_empty())
    }

    /// A story arrived (or failed) for `url`; ignored if another is open.
    pub fn arrived(&mut self, url: &str, result: Result<Article, String>) {
        if self.reader.url != url {
            return;
        }
        self.reader.state = match result {
            Ok(a) => ArticleState::Ready(Box::new(a)),
            Err(e) => ArticleState::Failed(e),
        };
    }

    fn now(&self) -> i64 {
        self.clock.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        })
    }

    /// The open story laid out at the current text size.
    fn layout(&self, index: usize) -> Option<reader::Layout> {
        let h = self.items.get(index)?;
        Some(reader::layout(&Story {
            headline: h,
            source: self.feed_name(&self.feed),
            reader: &self.reader,
            saved: !h.link.is_empty() && self.saved.contains(&h.link),
            size: self.size(),
            now: self.now(),
        }))
    }
}

/// News without a feeds file: how to add feeds.
pub(crate) const NO_FEEDS: (&str, &str) = (
    "No feeds yet",
    "Add News/feeds.txt to the SD card or the music folder: one feed address \
     per line, optionally followed by a name. Lines starting with # are skipped.",
);

/// News: one row per feed, and where the list comes from.
pub(crate) fn feed_rows(v: &NewsView) -> (Vec<Item>, String) {
    let rows = v
        .feeds
        .iter()
        .map(|f| {
            let detail = if f.url == v.feed && !v.items.is_empty() {
                let saved = v.items.iter().filter(|h| v.saved.contains(&h.link)).count();
                if saved > 0 {
                    format!("{} headlines · {saved} saved", v.items.len())
                } else {
                    format!("{} headlines", v.items.len())
                }
            } else {
                String::new()
            };
            Item::new(&f.name, format!("news:{}", f.url))
                .with_secondary(detail)
                .with_kind(RowKind::Chevron)
        })
        .collect();
    (rows, format!("Feeds from {FEEDS_FILE}"))
}

/// One feed: a refresh row with the fetch status, then the headlines.
pub(crate) fn headline_rows(v: &NewsView, url: &str) -> Vec<Item> {
    if v.feed != url || (v.items.is_empty() && v.busy) {
        return vec![Item::new("Loading headlines…", "news_refresh").with_secondary("Uses Wi-Fi")];
    }
    let status = if v.busy {
        "Updating…".to_owned()
    } else if v.saving > 0 {
        format!("Saving for offline: {} left", v.saving)
    } else if let Some(e) = &v.error {
        e.clone()
    } else if v.fetched.is_some() {
        "Up to date".to_owned()
    } else if !v.items.is_empty() {
        "Saved headlines".to_owned()
    } else {
        String::new()
    };
    let label = if v.items.is_empty() && v.error.is_some() {
        "Try Again"
    } else {
        "Refresh"
    };
    let now = v.now();
    std::iter::once(Item::new(label, "news_refresh").with_secondary(status))
        .chain(v.items.iter().enumerate().map(|(i, h)| {
            let when = age(h.published, now).unwrap_or_default();
            // Saved stories read without Wi-Fi.
            let detail = match (v.saved.contains(&h.link), when.is_empty()) {
                (true, true) => "Saved".to_owned(),
                (true, false) => format!("{when} · Saved"),
                (false, _) => when,
            };
            Item::new(&h.title, format!("story:{i}")).with_secondary(detail)
        }))
        .collect()
}

impl Ui {
    /// Rows that open News, a feed, a refresh or a story.
    pub(crate) fn news_key(&mut self, m: &mut AppModel, key: &str) -> Option<Effect> {
        if key == "news" {
            Self::go(m, Screen::News, "");
            return Some(Effect::None);
        }
        if key == "news_refresh" {
            let url = m.navigation.filter.clone();
            return Some(if self.news.busy || url.is_empty() {
                Effect::None
            } else {
                Effect::NewsFetch(url)
            });
        }
        if let Some(url) = key.strip_prefix("news:") {
            Self::go(m, Screen::News, url);
            let n = &self.news;
            return Some(if n.feed != url || (!n.busy && n.stale(Instant::now())) {
                Effect::NewsFetch(url.to_owned())
            } else {
                Effect::None
            });
        }
        let i = key.strip_prefix("story:")?;
        Self::go(m, Screen::NewsStory, i);
        let index = i.parse().unwrap_or(0);
        Some(
            self.news
                .open(index)
                .map_or(Effect::None, Effect::NewsArticle),
        )
    }

    /// The reader's wheel and buttons: the wheel scrolls, Up and Down (on a
    /// keyboard) jump between paragraphs, Select changes the text size (or
    /// tries again after a failed fetch), Next and Previous change story.
    /// `None` lets Back, Home and the rest through.
    pub(crate) fn news_action(&mut self, m: &mut AppModel, action: Action) -> Option<Effect> {
        if m.screen != Screen::NewsStory
            || m.screen_off
            || m.navigation.modal.is_some()
            || self.text_entry
            || self.pairing.is_some()
            || self.alert.is_some()
        {
            return None;
        }
        match action {
            Action::NextTrack => return Some(self.change_story(m, true)),
            Action::PreviousTrack => return Some(self.change_story(m, false)),
            _ => {}
        }
        let index: usize = m.navigation.filter.parse().unwrap_or(0);
        let now = Instant::now();
        let n = &mut self.news;
        let layout = n.layout(index)?;
        let max = layout.max_scroll();
        let line = reader::SIZES[usize::from(n.size())].2;
        let detents = |d: u8| i32::from(d.clamp(1, 8));
        match action {
            Action::WheelClockwise(d) => n.reader.scroll.wheel(detents(d), line, max, now),
            Action::WheelCounterClockwise(d) => n.reader.scroll.wheel(-detents(d), line, max, now),
            Action::NavigateDown | Action::NavigateUp => {
                let to = layout.next_block(n.reader.scroll.target, action == Action::NavigateDown);
                n.reader.scroll.go(to, max, now);
            }
            Action::Select => {
                if let ArticleState::Failed(_) = n.reader.state {
                    n.reader.state = ArticleState::Loading;
                    return Some(Effect::NewsArticle(n.reader.url.clone()));
                }
                // Keep the place in the story at the new size.
                let progress = layout.progress(n.reader.scroll.target);
                n.text_size = Some((n.size() + 1) % 3);
                n.dirty = true;
                let max = n.layout(index).map_or(0., |l| l.max_scroll());
                n.reader.scroll.jump(progress * max, max);
                let name = reader::SIZES[usize::from(n.size())].0;
                self.flash(format!("Text size: {name}"));
            }
            _ => return None,
        }
        Some(Effect::None)
    }

    fn change_story(&mut self, m: &mut AppModel, forward: bool) -> Effect {
        let at: usize = m.navigation.filter.parse().unwrap_or(0);
        let to = if forward {
            Some(at + 1).filter(|&to| to < self.news.items.len())
        } else {
            at.checked_sub(1)
        };
        let Some(to) = to else {
            self.flash(if forward {
                "That was the last story"
            } else {
                "This is the first story"
            });
            return Effect::None;
        };
        m.navigation.filter = to.to_string();
        // Keep the headline list's focus on the story being read.
        if let Some(frame) = m.navigation.history.last_mut() {
            frame.focus = to + 1;
        }
        self.news.open(to).map_or(Effect::None, Effect::NewsArticle)
    }
}

/// One story below the status bar (which the caller draws).
pub(crate) fn draw_story(c: &mut Canvas, v: &NewsView, index: usize, accent: u32) {
    let Some(layout) = v.layout(index) else {
        c.text_mid_center(
            WIDTH / 2.,
            170.,
            "This story is no longer here",
            Face::R15,
            color::MUTED,
        );
        return;
    };
    let scroll = v.reader.scroll.at(Instant::now());
    let hint = format!(
        "{} of {} · Next or Previous for more stories · Select for text size",
        index + 1,
        v.items.len()
    );
    reader::draw(c, &layout, scroll, &hint, accent);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_900_000;
    const FEED: &str = "https://feeds.example.org/top.xml";

    fn view() -> NewsView {
        NewsView {
            feeds: vec![
                Feed {
                    url: FEED.into(),
                    name: "Example News".into(),
                },
                Feed {
                    url: "https://other.example.net/rss".into(),
                    name: "Other".into(),
                },
            ],
            feed: FEED.into(),
            items: (0..3)
                .map(|i| Headline {
                    title: format!("Headline {i} that is long enough to need wrapping onto a second and even a third line of text on this display"),
                    summary: "A summary sentence that keeps going. ".repeat(30),
                    published: Some(NOW - 7200),
                    link: if i == 2 { String::new() } else { format!("https://example.com/{i}") },
                })
                .collect(),
            fetched: Some(Instant::now()),
            clock: Some(NOW),
            ..Default::default()
        }
    }

    #[test]
    fn headline_rows_lead_with_refresh_and_age() {
        let rows = headline_rows(&view(), FEED);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].key, "news_refresh");
        assert_eq!(rows[1].key, "story:0");
        assert_eq!(rows[1].secondary, "2 h ago");
        assert_eq!(
            headline_rows(&view(), "https://other.example.net/rss").len(),
            1,
            "another feed shows loading"
        );
    }

    #[test]
    fn saved_stories_are_marked_and_counted() {
        let mut v = view();
        v.saved.insert("https://example.com/1".into());
        v.saving = 1;
        let rows = headline_rows(&v, FEED);
        assert_eq!(rows[0].secondary, "Saving for offline: 1 left");
        assert_eq!(rows[1].secondary, "2 h ago");
        assert_eq!(rows[2].secondary, "2 h ago · Saved");
        let (feeds, status) = feed_rows(&v);
        assert_eq!(feeds[0].secondary, "3 headlines · 1 saved");
        assert_eq!(feeds[1].key, "news:https://other.example.net/rss");
        assert_eq!(status, "Feeds from News/feeds.txt");
    }

    #[test]
    fn opening_a_story_asks_for_its_link_and_takes_only_its_reply() {
        let mut v = view();
        assert_eq!(v.open(1).as_deref(), Some("https://example.com/1"));
        assert_eq!(v.reader.state, ArticleState::Loading);
        v.arrived("https://example.com/0", Err("late reply".into()));
        assert_eq!(v.reader.state, ArticleState::Loading);
        v.arrived(
            "https://example.com/1",
            Err("No internet connection".into()),
        );
        assert_eq!(
            v.reader.state,
            ArticleState::Failed("No internet connection".into())
        );
        // A story without a link shows its summary.
        assert_eq!(v.open(2), None);
        assert_eq!(v.reader.state, ArticleState::None);
    }

    #[test]
    fn extras_opens_the_feeds_and_a_feed_fetches_its_headlines() {
        let mut ui = Ui::default();
        let mut m = AppModel::default();
        let home = ui.rows(&m, &[]);
        m.navigation.focus = home.iter().position(|r| r.key == "extras").unwrap();
        ui.action(&mut m, &[], Action::Select);
        let extras = ui.rows(&m, &[]);
        m.navigation.focus = extras.iter().position(|r| r.key == "news").unwrap();
        assert_eq!(ui.action(&mut m, &[], Action::Select), Effect::None);
        assert_eq!((m.screen, m.navigation.filter.as_str()), (Screen::News, ""));
        // No built-in feeds: the page says how to add some.
        let page = crate::pages::page(&ui, &m, &[]);
        assert!(page.rows.is_empty());
        assert!(page.hero.unwrap().body.contains(FEEDS_FILE));
        ui.news.feeds = view().feeds;
        assert_eq!(ui.rows(&m, &[]).len(), 2);
        let url = FEED.to_owned();
        assert_eq!(
            ui.action(&mut m, &[], Action::Select),
            Effect::NewsFetch(url.clone())
        );
        assert_eq!(m.navigation.filter, url);
        // While it loads, the refresh row asks for nothing more.
        ui.news.busy = true;
        assert_eq!(ui.action(&mut m, &[], Action::Select), Effect::None);
    }

    #[test]
    fn reader_controls_open_scroll_resize_and_change_story() {
        let mut ui = Ui {
            news: view(),
            ..Default::default()
        };
        let mut m = AppModel {
            screen: Screen::News,
            ..Default::default()
        };
        m.navigation.filter = FEED.into();
        m.navigation.focus = 2;
        assert_eq!(
            ui.action(&mut m, &[], Action::Select),
            Effect::NewsArticle("https://example.com/1".into())
        );
        assert_eq!(
            (m.screen, m.navigation.filter.as_str()),
            (Screen::NewsStory, "1")
        );
        // The wheel scrolls (the summary is long enough), and stays put at the top.
        assert_eq!(
            ui.action(&mut m, &[], Action::WheelClockwise(2)),
            Effect::None
        );
        assert!(ui.news.reader.scroll.target > 0.);
        ui.action(&mut m, &[], Action::WheelCounterClockwise(8));
        assert_eq!(ui.news.reader.scroll.target, 0.);
        // Select steps the text size from Medium and asks for it to be saved.
        assert_eq!(ui.news.size(), reader::DEFAULT_SIZE);
        ui.action(&mut m, &[], Action::Select);
        assert_eq!((ui.news.text_size, ui.news.dirty), (Some(2), true));
        ui.action(&mut m, &[], Action::Select);
        assert_eq!(ui.news.size(), 0);
        // Next and Previous change story; the last one says so.
        assert_eq!(ui.action(&mut m, &[], Action::NextTrack), Effect::None);
        assert_eq!(m.navigation.filter, "2");
        assert_eq!(ui.news.reader.state, ArticleState::None);
        assert_eq!(ui.action(&mut m, &[], Action::NextTrack), Effect::None);
        assert_eq!(m.navigation.filter, "2");
        assert_eq!(
            ui.action(&mut m, &[], Action::PreviousTrack),
            Effect::NewsArticle("https://example.com/1".into())
        );
        // A failed story is fetched again with Select.
        ui.news
            .arrived("https://example.com/1", Err("Timed out".into()));
        assert_eq!(
            ui.action(&mut m, &[], Action::Select),
            Effect::NewsArticle("https://example.com/1".into())
        );
        assert_eq!(ui.news.reader.state, ArticleState::Loading);
        // Away from the reader, Next is the music's again.
        ui.action(&mut m, &[], Action::Back);
        assert_eq!(m.screen, Screen::News);
        assert_eq!(m.navigation.focus, 2, "the list follows the story read");
        assert_eq!(ui.action(&mut m, &[], Action::NextTrack), Effect::NextTrack);
    }

    #[test]
    fn stories_draw_inside_the_display() {
        let v = view();
        for index in [0, 2, 9] {
            let mut c = Canvas::new();
            draw_story(&mut c, &v, index, 0x40a0_ffff);
            for q in c.finish() {
                assert!(q.x >= -16. && q.y >= -16., "{q:?}");
                assert!(q.x + q.w <= 496. && q.y + q.h <= 376., "{q:?}");
            }
        }
    }
}

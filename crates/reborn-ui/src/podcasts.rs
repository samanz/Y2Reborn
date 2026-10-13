//! Home > Podcasts: the shows whose feeds are listed on the SD card, each
//! show's episodes, and one episode's page (what it is, Play or Download,
//! Mark played, Delete, and its description).
//!
//! The episode page is a document of blocks (`Block`); the wheel moves
//! through the focusable ones (the actions and the pieces of the
//! description), which are also the screen's rows. Drawing is in
//! `views_podcasts`.
use crate::{
    pages::{Hero, Page},
    Item, RowKind, Ui, WifiStatus,
};
use reborn_core::{
    podcast_text,
    podcasts::{age_text, length_text, Episode, PodcastState, Show},
    AppModel, ConfirmAction, Effect, Modal, Screen,
};
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct PodcastsView {
    /// The shows on the SD card, in the order of the feed list.
    pub shows: Vec<Show>,
    pub state: PodcastState,
    /// Episodes whose audio is on the card.
    pub on_card: HashSet<String>,
    /// The episode downloading now and the bytes it has so far.
    pub downloading: Option<(String, u64)>,
    pub refreshing: bool,
    /// The last refresh's or download's problem, in plain words.
    pub problem: Option<String>,
    /// An SD card is in.
    pub card: bool,
    /// The episode playing now (set by the app).
    pub playing: Option<String>,
    /// Unix seconds, for "3 days ago" (set by the app; fixed in previews).
    pub now: i64,
    /// `state` changed and should be saved.
    pub dirty: bool,
}

impl PodcastsView {
    pub fn show(&self, folder: &str) -> Option<&Show> {
        self.shows.iter().find(|s| s.folder == folder)
    }

    /// An episode and its show, by episode id.
    pub fn find(&self, id: &str) -> Option<(&Show, &Episode)> {
        self.shows
            .iter()
            .find_map(|s| s.episode(id).map(|e| (s, e)))
    }

    pub fn is_on_card(&self, id: &str) -> bool {
        self.on_card.contains(id)
    }

    /// How far the download of `id` is: Some(None) while it runs with an
    /// unknown size.
    pub fn download_progress(&self, id: &str) -> Option<Option<f32>> {
        let (dl, got) = self.downloading.as_ref().filter(|(dl, _)| dl == id)?;
        let size = self.find(dl).map_or(0, |(_, e)| e.length);
        Some((size > 0).then(|| (*got as f32 / size as f32).clamp(0., 0.99)))
    }

    /// An episode's second line: its download, or how much is left, or when
    /// it came out and how long it is.
    pub fn episode_detail(&self, e: &Episode) -> String {
        match self.download_progress(&e.id) {
            Some(Some(part)) => return format!("Downloading {}%", (part * 100.) as u32),
            Some(None) => return "Downloading…".into(),
            None => {}
        }
        if !self.is_on_card(&e.id) && self.state.wanted.contains(&e.id) {
            return "Waiting to download".into();
        }
        let l = self.state.listening(&e.id);
        if l.played {
            return "Played".into();
        }
        let total = e.duration_ms();
        if l.position_ms > 0 && total > l.position_ms {
            let left = (total - l.position_ms).div_ceil(1000);
            return format!("{} left", length_text(left));
        }
        joined(&[&age_text(e.published, self.now), &length_text(e.duration_s)])
    }

    /// How much of `id` has been heard, 0..=1.
    pub fn heard(&self, e: &Episode) -> f32 {
        let l = self.state.listening(&e.id);
        if l.played {
            return 1.;
        }
        let total = e.duration_ms();
        if total == 0 {
            return 0.;
        }
        (l.position_ms as f32 / total as f32).clamp(0., 1.)
    }

    fn checked_text(&self, online: bool) -> String {
        if self.refreshing {
            return "Checking…".into();
        }
        if let Some(problem) = &self.problem {
            return problem.clone();
        }
        if !online {
            return "Needs Wi-Fi".into();
        }
        match self.state.checked {
            Some(t) => format!("Checked {}", ago(self.now - t)),
            None => String::new(),
        }
    }
}

fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 90 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 2 * 86400 => format!("{} h ago", s / 3600),
        s => format!("{} days ago", s / 86400),
    }
}

/// "Today · 42 min", leaving out what is empty.
fn joined(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn online(ui: &Ui) -> bool {
    matches!(ui.wifi.status, WifiStatus::Connected(_))
}

/// The page of a podcast screen.
pub(crate) fn page(ui: &Ui, m: &AppModel) -> Page {
    let v = &ui.podcasts;
    match m.screen {
        Screen::PodcastShow => show_page(v, &m.navigation.filter),
        Screen::PodcastEpisode => Page {
            rows: page_rows(&episode_page(v, &m.navigation.filter)),
            ..Default::default()
        },
        _ => list_page(ui),
    }
}

/// Home > Podcasts: Check for New Episodes, then the shows.
fn list_page(ui: &Ui) -> Page {
    let v = &ui.podcasts;
    if !v.card {
        return Page {
            hero: Some(Hero {
                title: "No SD card".into(),
                body: "Podcasts are kept on the SD card. Insert it to see your shows.".into(),
            }),
            ..Default::default()
        };
    }
    let mut rows = vec![Item::new("Check for New Episodes", "pod_refresh")
        .with_secondary(v.checked_text(online(ui)))];
    for show in &v.shows {
        let here = show.episodes.iter().filter(|e| v.is_on_card(&e.id)).count();
        let detail = match here {
            0 => match show.episodes.len() {
                1 => "1 episode".to_owned(),
                n => format!("{n} episodes"),
            },
            n => format!("{n} on the player"),
        };
        rows.push(
            Item::new(&show.title, format!("pod_show:{}", show.folder))
                .with_secondary(detail)
                .with_kind(RowKind::Chevron),
        );
    }
    let hero = v.shows.is_empty().then(|| Hero {
        title: "No podcasts yet".into(),
        body: "Put feed addresses in Podcasts/feeds.txt on the SD card, one per line, then \
               check for new episodes on Wi-Fi."
            .into(),
    });
    Page {
        hero,
        rows,
        ..Default::default()
    }
}

/// A show's episodes, newest first.
fn show_page(v: &PodcastsView, folder: &str) -> Page {
    let Some(show) = v.show(folder) else {
        return Page {
            hero: Some(Hero {
                title: "This show is no longer here".into(),
                body: String::new(),
            }),
            ..Default::default()
        };
    };
    let rows: Vec<Item> = show
        .episodes
        .iter()
        .map(|e| {
            Item::new(&e.title, format!("pod_ep:{}", e.id)).with_secondary(v.episode_detail(e))
        })
        .collect();
    let hero = rows.is_empty().then(|| Hero {
        title: "No episodes".into(),
        body: "The feed lists no audio episodes.".into(),
    });
    Page {
        hero,
        rows,
        ..Default::default()
    }
}

/// One part of an episode's page, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Block {
    /// Title, when, length and progress.
    Header,
    /// A line of buttons, each a row.
    Actions(Vec<Item>),
    Heading(String),
    /// A piece of the description (a row); `first` starts a paragraph.
    Text {
        item: Item,
        first: bool,
    },
    /// A quiet line, e.g. when there is no description.
    Note(String),
}

impl Block {
    pub(crate) fn rows(&self) -> &[Item] {
        match self {
            Block::Actions(items) => items,
            Block::Text { item, .. } => std::slice::from_ref(item),
            _ => &[],
        }
    }
}

/// The episode page: Play or Resume, Mark played and Delete for an episode
/// on the card; its download while it waits or runs; else Download.
pub(crate) fn episode_page(v: &PodcastsView, id: &str) -> Vec<Block> {
    let Some((_, e)) = v.find(id) else {
        return vec![Block::Note("This episode is no longer here".into())];
    };
    let mut actions = vec![];
    if v.is_on_card(id) {
        let l = v.state.listening(id);
        actions.push(if v.playing.as_deref() == Some(id) {
            Item::new("Now Playing", "now_playing")
        } else if l.position_ms > 0 && !l.played {
            Item::new("Resume", format!("pod_play:{id}"))
        } else {
            Item::new("Play", format!("pod_play:{id}"))
        });
        if !l.played {
            actions.push(Item::new("Mark played", format!("pod_played:{id}")));
        }
        actions.push(Item::new("Delete", format!("pod_delete:{id}")));
    } else if v.state.wanted.iter().any(|w| w == id) {
        let label = match v.download_progress(id) {
            Some(Some(part)) => format!("Downloading {}%", (part * 100.) as u32),
            Some(None) => "Downloading…".into(),
            None => "Waiting to download".into(),
        };
        actions.push(Item::new(label, format!("pod_wait:{id}")));
        actions.push(Item::new("Cancel", format!("pod_delete:{id}")));
    } else {
        actions.push(Item::new("Download", format!("pod_download:{id}")));
    }
    let mut blocks = vec![
        Block::Header,
        Block::Actions(actions),
        Block::Heading("About this episode".into()),
    ];
    if e.description.is_empty() {
        blocks.push(Block::Note("No description".into()));
    } else {
        let mut n = 0;
        for paragraph in e.description.split('\n') {
            for (i, piece) in podcast_text::chunks(paragraph).into_iter().enumerate() {
                blocks.push(Block::Text {
                    item: Item::new(piece, format!("pod_text:{n}")),
                    first: i == 0,
                });
                n += 1;
            }
        }
    }
    blocks
}

/// The rows of a page: its focusable blocks in order.
pub(crate) fn page_rows(blocks: &[Block]) -> Vec<Item> {
    blocks
        .iter()
        .flat_map(|b| b.rows().iter().cloned())
        .collect()
}

impl Ui {
    /// Select on a podcast row (keys starting `pod_`); None for other keys.
    pub(crate) fn podcast_key(&mut self, m: &mut AppModel, key: &str) -> Option<Effect> {
        if !key.starts_with("pod_") {
            return None;
        }
        let (kind, arg) = key.split_once(':').unwrap_or((key, ""));
        let online = online(self);
        let v = &mut self.podcasts;
        Some(match kind {
            "pod_refresh" if !online => {
                self.flash("Connect to Wi-Fi to check for new episodes");
                Effect::None
            }
            "pod_refresh" => Effect::PodcastRefresh,
            "pod_show" => {
                Self::go(m, Screen::PodcastShow, arg);
                Effect::None
            }
            "pod_ep" => {
                Self::go(m, Screen::PodcastEpisode, arg);
                Effect::None
            }
            "pod_play" => {
                Self::go(m, Screen::NowPlaying, "");
                Effect::PlayEpisode(arg.into())
            }
            "pod_played" => {
                if v.state.mark_played(arg) {
                    v.dirty = true;
                    self.flash("Marked as played");
                }
                Effect::None
            }
            "pod_download" => {
                if v.state.want(arg) {
                    v.dirty = true;
                }
                if !online {
                    self.flash("Downloads when the player is on Wi-Fi");
                }
                Effect::PodcastDownload
            }
            // Audio on the card is asked about first; a download that has
            // not finished is cancelled at once.
            "pod_delete" if v.is_on_card(arg) => {
                m.navigation.context_key = Some(arg.into());
                Self::open_modal(m, Modal::Confirm(ConfirmAction::DeleteEpisode));
                Effect::None
            }
            "pod_delete" => Effect::DeleteEpisode(arg.into()),
            _ => Effect::None,
        })
    }

    /// Delete confirmed: back to the show, the episode's audio goes.
    pub(crate) fn delete_episode(&mut self, m: &mut AppModel, id: String) -> Effect {
        Self::back(m);
        self.flash("Episode deleted");
        Effect::DeleteEpisode(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Ui};

    fn garden() -> PodcastsView {
        let ep = |id: &str, published: i64, minutes: u64| Episode {
            id: id.into(),
            title: format!("Episode {id}"),
            published,
            duration_s: minutes * 60,
            url: format!("https://media.example.org/{id}.mp3"),
            length: 30_000_000,
            mime: "audio/mpeg".into(),
            description: "First paragraph.\nSecond paragraph.".into(),
        };
        let now = 1_760_000_000;
        let mut v = PodcastsView {
            shows: vec![Show {
                feed: "https://feeds.example.org/garden.xml".into(),
                title: "The Example Garden".into(),
                folder: "The Example Garden".into(),
                episodes: vec![
                    ep("c", now - 3600, 42),
                    ep("b", now - 3 * 86_400, 30),
                    ep("a", now - 20 * 86_400, 65),
                ],
                ..Default::default()
            }],
            card: true,
            now,
            ..Default::default()
        };
        v.on_card.insert("a".into());
        v.on_card.insert("b".into());
        v
    }

    fn ui() -> Ui {
        let mut ui = Ui {
            podcasts: garden(),
            ..Default::default()
        };
        ui.wifi.status = WifiStatus::Connected("Home".into());
        ui
    }

    fn keys(rows: &[Item]) -> Vec<&str> {
        rows.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn podcasts_lists_the_shows_under_a_check_for_new_episodes() {
        let mut ui = ui();
        let m = AppModel {
            screen: Screen::Podcasts,
            ..Default::default()
        };
        let rows = ui.rows(&m, &[]);
        assert_eq!(keys(&rows), ["pod_refresh", "pod_show:The Example Garden"]);
        assert_eq!(rows[1].secondary, "2 on the player");
        assert_eq!(rows[0].secondary, "");
        ui.podcasts.state.checked = Some(ui.podcasts.now - 7200);
        assert_eq!(ui.rows(&m, &[])[0].secondary, "Checked 2 h ago");
        ui.podcasts.refreshing = true;
        assert_eq!(ui.rows(&m, &[])[0].secondary, "Checking…");
        // Without shows: how to add some; without a card: only that.
        ui.podcasts.shows.clear();
        let page = crate::pages::page(&ui, &m, &[]);
        assert_eq!(page.hero.unwrap().title, "No podcasts yet");
        assert_eq!(keys(&page.rows), ["pod_refresh"]);
        ui.podcasts.card = false;
        assert!(ui.rows(&m, &[]).is_empty());
    }

    #[test]
    fn episodes_say_what_is_on_the_player_and_how_far_it_was_heard() {
        let mut v = garden();
        v.state.listened("a", 1_950_000, 3_900_000);
        v.state.mark_played("b");
        v.state.want("c");
        let rows = show_page(&v, "The Example Garden").rows;
        assert_eq!(keys(&rows), ["pod_ep:c", "pod_ep:b", "pod_ep:a"]);
        let details: Vec<&str> = rows.iter().map(|r| r.secondary.as_str()).collect();
        assert_eq!(details, ["Waiting to download", "Played", "33 min left"]);
        v.downloading = Some(("c".into(), 15_000_000));
        assert_eq!(
            show_page(&v, "The Example Garden").rows[0].secondary,
            "Downloading 50%"
        );
        v.state.wanted.clear();
        v.downloading = None;
        assert_eq!(
            show_page(&v, "The Example Garden").rows[0].secondary,
            "Today · 42 min"
        );
        assert!(show_page(&v, "Gone").hero.is_some());
    }

    #[test]
    fn the_episode_page_offers_what_fits_its_state() {
        let mut v = garden();
        let actions = |v: &PodcastsView, id: &str| -> Vec<String> {
            match &episode_page(v, id)[1] {
                Block::Actions(items) => items.iter().map(|i| i.label.clone()).collect(),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(actions(&v, "b"), ["Play", "Mark played", "Delete"]);
        v.state.listened("b", 60_000, 1_800_000);
        assert_eq!(actions(&v, "b"), ["Resume", "Mark played", "Delete"]);
        v.playing = Some("b".into());
        assert_eq!(actions(&v, "b")[0], "Now Playing");
        v.state.mark_played("a");
        assert_eq!(actions(&v, "a"), ["Play", "Delete"]);
        assert_eq!(actions(&v, "c"), ["Download"]);
        v.state.want("c");
        assert_eq!(actions(&v, "c"), ["Waiting to download", "Cancel"]);
        v.downloading = Some(("c".into(), 3_000_000));
        assert_eq!(actions(&v, "c")[0], "Downloading 10%");
        let blocks = episode_page(&v, "c");
        assert_eq!(blocks[2], Block::Heading("About this episode".into()));
        let rows = page_rows(&blocks);
        assert_eq!(
            keys(&rows),
            ["pod_wait:c", "pod_delete:c", "pod_text:0", "pod_text:1"]
        );
        assert_eq!(rows[3].label, "Second paragraph.");
        assert_eq!(
            episode_page(&v, "gone"),
            [Block::Note("This episode is no longer here".into())]
        );
    }

    #[test]
    fn select_walks_from_shows_to_an_episode_and_plays_it() {
        let mut ui = ui();
        let mut m = AppModel {
            screen: Screen::Podcasts,
            ..Default::default()
        };
        assert_eq!(
            ui.model_action(&mut m, Action::Select),
            Effect::PodcastRefresh
        );
        m.navigation.focus = 1;
        ui.model_action(&mut m, Action::Select);
        assert_eq!(
            (m.screen, m.navigation.filter.as_str()),
            (Screen::PodcastShow, "The Example Garden")
        );
        m.navigation.focus = 1;
        ui.model_action(&mut m, Action::Select);
        assert_eq!(
            (m.screen, m.navigation.filter.as_str()),
            (Screen::PodcastEpisode, "b")
        );
        // The wheel moves through the buttons and the description.
        ui.model_action(&mut m, Action::WheelClockwise(1));
        assert_eq!(ui.model_action(&mut m, Action::Select), Effect::None);
        assert!(ui.podcasts.state.listening("b").played && ui.podcasts.dirty);
        m.navigation.focus = 0;
        assert_eq!(
            ui.model_action(&mut m, Action::Select),
            Effect::PlayEpisode("b".into())
        );
        assert_eq!(m.screen, Screen::NowPlaying);
        // Back to the page: Delete asks first; Cancel keeps the episode.
        ui.model_action(&mut m, Action::Back);
        assert_eq!(m.screen, Screen::PodcastEpisode);
        m.navigation.focus = 1;
        assert_eq!(ui.model_action(&mut m, Action::Select), Effect::None);
        assert_eq!(
            m.navigation.modal,
            Some(Modal::Confirm(ConfirmAction::DeleteEpisode))
        );
        assert_eq!(ui.modal_copy(&m).0, "Delete this episode?");
        let labels: Vec<String> = ui
            .modal_rows_public(&m, &[])
            .into_iter()
            .map(|r| r.label)
            .collect();
        assert_eq!(labels, ["Cancel", "Delete"]);
        assert_eq!(ui.model_action(&mut m, Action::Select), Effect::None);
        assert_eq!(m.screen, Screen::PodcastEpisode);
        assert!(m.navigation.modal.is_none());
        // Confirmed, it goes and the page returns to the show.
        ui.model_action(&mut m, Action::Select);
        m.navigation.modal_focus = 1;
        assert_eq!(
            ui.model_action(&mut m, Action::Select),
            Effect::DeleteEpisode("b".into())
        );
        assert_eq!(m.screen, Screen::PodcastShow);
        assert_eq!(ui.notice, "Episode deleted");
    }

    #[test]
    fn every_podcast_screen_draws_one_focus_inside_the_display() {
        let mut ui = ui();
        ui.podcasts.state.listened("a", 600_000, 3_900_000);
        ui.podcasts.state.mark_played("b");
        ui.podcasts.state.want("c");
        ui.podcasts.downloading = Some(("c".into(), 6_000_000));
        ui.podcasts.shows[0].episodes[2].description = "Why bread rises. ".repeat(60);
        for (screen, filter) in [
            (Screen::Podcasts, ""),
            (Screen::PodcastShow, "The Example Garden"),
            (Screen::PodcastEpisode, "a"),
            (Screen::PodcastEpisode, "b"),
            (Screen::PodcastEpisode, "c"),
        ] {
            let mut m = AppModel {
                screen,
                ..Default::default()
            };
            m.navigation.filter = filter.into();
            let rows = ui.rows(&m, &[]);
            assert!(!rows.is_empty(), "{screen:?} {filter}");
            // Down the whole page and back up.
            for focus in (0..rows.len()).chain([0]) {
                m.navigation.focus = focus;
                let quads = ui.draw(&m, &[], false);
                assert_eq!(
                    crate::focus_target_count(&quads),
                    1,
                    "{screen:?} {filter} focus {focus}"
                );
                for q in &quads {
                    assert!(q.x >= -16. && q.y >= -16., "{q:?}");
                    assert!(q.x + q.w <= 496. && q.y + q.h <= 376., "{q:?}");
                }
            }
        }
    }

    #[test]
    fn downloads_are_chosen_from_the_page_and_wait_for_wi_fi() {
        let mut ui = ui();
        ui.wifi.status = WifiStatus::Disconnected;
        let mut m = AppModel {
            screen: Screen::PodcastEpisode,
            ..Default::default()
        };
        m.navigation.filter = "c".into();
        assert_eq!(
            ui.model_action(&mut m, Action::Select),
            Effect::PodcastDownload
        );
        assert_eq!(ui.podcasts.state.wanted, ["c"]);
        assert_eq!(ui.notice, "Downloads when the player is on Wi-Fi");
        // Cancel takes it off the list; the page stays.
        m.navigation.focus = 1;
        assert_eq!(
            ui.model_action(&mut m, Action::Select),
            Effect::DeleteEpisode("c".into())
        );
        assert_eq!(m.screen, Screen::PodcastEpisode);
        // Checking needs Wi-Fi.
        m.screen = Screen::Podcasts;
        m.navigation.focus = 0;
        assert_eq!(ui.model_action(&mut m, Action::Select), Effect::None);
        assert_eq!(ui.notice, "Connect to Wi-Fi to check for new episodes");
    }
}

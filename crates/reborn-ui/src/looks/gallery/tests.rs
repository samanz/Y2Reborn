//! Gallery's wheel model: one focus on every screen, tabs from the pill and
//! Today's shelves walked in reading order.
use super::today::{locate, with_shelves};
use super::*;
use crate::focus_target_count;
use reborn_core::{Action, PlaybackState};

/// Seven albums of three songs, the first playing, drawn at rest.
fn library() -> (Ui, AppModel) {
    let ui = Ui {
        motion_fixed: true,
        ..Default::default()
    };
    let mut m = AppModel {
        playback: PlaybackState::Playing,
        ..Default::default()
    };
    m.settings.look = "gallery".into();
    let mut tracks = vec![];
    for a in 0..7 {
        for n in 0..3 {
            tracks.push(Track {
                id: tracks.len() as i64 + 1,
                path: format!("/m/{a}/{n}.flac").into(),
                title: format!("Song {a}{n}"),
                artist: format!("Artist {a}"),
                album_artist: format!("Artist {a}"),
                album: format!("Album {a}"),
                track: n + 1,
                duration_ms: 200_000,
                mtime: a as i64,
                artwork: true,
                online: true,
                ..Default::default()
            });
        }
    }
    m.replace_queue(tracks[..3].to_vec(), 0).unwrap();
    m.library.tracks = tracks;
    (ui, m)
}

fn act(ui: &mut Ui, m: &mut AppModel, action: Action) -> Effect {
    ui.model_action(m, action)
}

fn focus_count(ui: &Ui, m: &AppModel) -> usize {
    focus_target_count(&ui.draw(m, &m.library.tracks, true))
}

#[test]
fn today_walks_every_card_in_reading_order() {
    let (mut ui, mut m) = library();
    assert_eq!(focus_count(&ui, &m), 1, "the hero");
    let shelves = with_shelves(&ui, &m, &m.library.tracks, <[_]>::to_vec);
    assert_eq!(shelves[0].title, "New in your library");
    assert_eq!(shelves[0].cards[0].title, "Album 6", "the newest first");
    assert_eq!(shelves.last().unwrap().title, "Your library");
    let first = shelves[0].cards.len();
    // Across the first shelf, then on to the first card of the next.
    for n in 0..=first {
        act(&mut ui, &mut m, Action::WheelClockwise(1));
        assert_eq!(focus_count(&ui, &m), 1);
        let expect = if n < first { (0, n) } else { (1, 0) };
        assert_eq!(locate(&shelves, ui.gallery.focus.get()), Some(expect));
    }
    // The wheel stops at the last card, and turns back to the hero.
    for _ in 0..80 {
        act(&mut ui, &mut m, Action::WheelClockwise(1));
    }
    let last = shelves.iter().map(|s| s.cards.len()).sum::<usize>();
    assert_eq!(ui.gallery.focus.get(), last);
    assert_eq!(focus_count(&ui, &m), 1);
    for _ in 0..80 {
        act(&mut ui, &mut m, Action::WheelCounterClockwise(1));
    }
    assert_eq!(ui.gallery.focus.get(), 0);
    assert_eq!(m.screen, Screen::Home);
}

#[test]
fn shelves_come_from_the_library_alone() {
    let (ui, m) = library();
    let shelves = with_shelves(&ui, &m, &m.library.tracks, <[_]>::to_vec);
    let titles: Vec<&str> = shelves.iter().map(|s| s.title).collect();
    assert_eq!(
        titles,
        [
            "New in your library",
            "Picked for today",
            "Your artists",
            "Your library"
        ]
    );
    assert_eq!(shelves[1].cards.len(), 6);
    assert!(shelves[2]
        .cards
        .iter()
        .all(|c| matches!(c.art, today::Art::Portrait(_))));
}

#[test]
fn a_card_opens_its_album_and_back_returns_to_today() {
    let (mut ui, mut m) = library();
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    act(&mut ui, &mut m, Action::Select);
    assert_eq!(m.screen, Screen::Album);
    assert!(m.navigation.filter.contains("Album 6"), "the newest");
    assert_eq!(focus_count(&ui, &m), 1);
    act(&mut ui, &mut m, Action::Back);
    assert_eq!(m.screen, Screen::Home);
    assert_eq!(ui.gallery.focus.get(), 1, "the focus is kept");
    assert!(!ui.gallery.pill.get());
}

#[test]
fn the_hero_shuffles_everything() {
    let (mut ui, mut m) = library();
    match act(&mut ui, &mut m, Action::Select) {
        Effect::PlayCollection {
            members, shuffle, ..
        } => {
            assert!(shuffle);
            assert_eq!(members.len(), 21);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(m.screen, Screen::NowPlaying);
}

#[test]
fn back_at_a_root_moves_to_the_pill_and_the_wheel_changes_tab() {
    let (mut ui, mut m) = library();
    act(&mut ui, &mut m, Action::Back);
    assert!(ui.gallery.pill.get());
    assert_eq!(focus_count(&ui, &m), 1, "only the pill");
    // The wheel moves the capsule; nothing opens until Select.
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    assert_eq!(m.screen, Screen::Home);
    act(&mut ui, &mut m, Action::Select);
    assert_eq!(m.screen, Screen::Albums);
    assert!(m.navigation.stack.is_empty() && m.navigation.history.is_empty());
    assert_eq!(root_tab(&m), Some(1));
    assert!(!ui.gallery.pill.get(), "the wall has the focus");
    assert_eq!(focus_count(&ui, &m), 1);
    // From Albums to Artists and Songs the same way.
    for (tab, screen) in [(2, Screen::Artists), (3, Screen::Tracks)] {
        act(&mut ui, &mut m, Action::Back);
        assert!(ui.gallery.pill.get(), "{screen:?}");
        assert_eq!(focus_count(&ui, &m), 1, "{screen:?} pill");
        act(&mut ui, &mut m, Action::WheelClockwise(1));
        act(&mut ui, &mut m, Action::Select);
        assert_eq!(m.screen, screen);
        assert_eq!(root_tab(&m), Some(tab));
        assert_eq!(focus_count(&ui, &m), 1, "{screen:?}");
    }
    // Back on the pill returns to the tab.
    act(&mut ui, &mut m, Action::Back);
    act(&mut ui, &mut m, Action::Back);
    assert!(!ui.gallery.pill.get());
    assert_eq!(m.screen, Screen::Tracks);
    // Home is Today, from the top.
    act(&mut ui, &mut m, Action::Home);
    assert_eq!(m.screen, Screen::Home);
    assert_eq!(ui.gallery.focus.get(), 0);
}

#[test]
fn the_wheel_on_the_pill_stays_within_the_tabs() {
    let (mut ui, mut m) = library();
    act(&mut ui, &mut m, Action::Back);
    for _ in 0..9 {
        act(&mut ui, &mut m, Action::WheelCounterClockwise(1));
    }
    assert_eq!(ui.gallery.pill_tab.get(), 0);
    for _ in 0..9 {
        act(&mut ui, &mut m, Action::WheelClockwise(1));
    }
    assert_eq!(ui.gallery.pill_tab.get(), TABS.len() - 1);
    // Transport keeps its meaning on the pill.
    assert_eq!(
        act(&mut ui, &mut m, Action::PlayPause),
        Effect::TogglePlayback
    );
    assert!(ui.gallery.pill.get());
}

#[test]
fn deeper_screens_keep_the_standard_back() {
    let (mut ui, mut m) = library();
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    act(&mut ui, &mut m, Action::Select);
    assert_eq!(root_tab(&m), None);
    act(&mut ui, &mut m, Action::Back);
    assert!(!ui.gallery.pill.get());
    assert_eq!(m.screen, Screen::Home);
}

#[test]
fn other_looks_take_no_input() {
    let (mut ui, mut m) = library();
    m.settings.look = String::new();
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    assert_eq!(ui.gallery.focus.get(), 0);
    assert_eq!(m.navigation.focus, 1, "the standard Home list moved");
}

#[test]
fn an_album_page_focuses_play_shuffle_and_songs() {
    let (mut ui, mut m) = library();
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    act(&mut ui, &mut m, Action::Select);
    assert_eq!(m.screen, Screen::Album);
    for _ in 0..6 {
        assert_eq!(focus_count(&ui, &m), 1, "focus {}", m.navigation.focus);
        act(&mut ui, &mut m, Action::WheelClockwise(1));
    }
}

#[test]
fn every_screen_draws_one_focus() {
    let (ui, mut m) = library();
    for screen in [
        Screen::Albums,
        Screen::Artists,
        Screen::Genres,
        Screen::Tracks,
        Screen::Folders,
        Screen::Queue,
        Screen::NowPlaying,
        Screen::Settings,
        Screen::SettingsAudio,
        Screen::SettingsDisplay,
        Screen::Look,
        Screen::Storage,
        Screen::Update,
    ] {
        m.screen = screen;
        m.navigation.focus = 1;
        let lists = matches!(
            screen,
            Screen::Albums | Screen::Artists | Screen::Tracks | Screen::Settings | Screen::Look
        );
        let count = focus_count(&ui, &m);
        if lists || screen == Screen::NowPlaying {
            assert_eq!(count, 1, "{screen:?}");
        } else {
            assert!(count <= 1, "{screen:?}: {count}");
        }
    }
}

#[test]
fn the_look_takes_the_album_colour() {
    let (mut ui, m) = library();
    let shown = || SHOWN.with(Cell::get);
    ui.accent = 0xFF965AFF;
    ui.draw(&m, &m.library.tracks, true);
    let warm = shown();
    ui.accent = 0x5AE6CDFF;
    ui.draw(&m, &m.library.tracks, true);
    let cool = shown();
    assert_ne!(warm.bg, cool.bg);
    assert!(!ui.animating(), "previews and tests are drawn at rest");
    // With a clock the change eases, and frames keep coming meanwhile.
    ui.motion_fixed = false;
    ui.draw(&m, &m.library.tracks, true);
    ui.accent = 0xFF965AFF;
    ui.draw(&m, &m.library.tracks, true);
    assert!(ui.animating());
    assert_ne!(shown(), warm);
}

#[test]
fn pages_slide_in_and_back_only_while_moving() {
    let (mut ui, mut m) = library();
    ui.motion_fixed = false;
    ui.draw(&m, &m.library.tracks, true);
    std::thread::sleep(std::time::Duration::from_millis(30));
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    act(&mut ui, &mut m, Action::Select);
    let quads = ui.draw(&m, &m.library.tracks, true);
    assert!(ui.animating(), "a push slides");
    assert_eq!(focus_target_count(&quads), 1);
    // The arriving page starts off to the right.
    assert!(quads.iter().any(|q| q.x > WIDTH * 0.8));
    std::thread::sleep(std::time::Duration::from_millis(450));
    let quads = ui.draw(&m, &m.library.tracks, true);
    assert_eq!(focus_target_count(&quads), 1);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    ui.draw(&m, &m.library.tracks, true);
    assert!(!ui.gallery.clock.moving(), "at rest, frames stop");
    act(&mut ui, &mut m, Action::Back);
    ui.draw(&m, &m.library.tracks, true);
    assert!(ui.gallery.clock.moving(), "a pop slides back");
}

#[test]
fn idle_today_fills_the_room_under_the_hero() {
    let (mut ui, mut m) = library();
    m.queue.clear();
    m.queue_entry_ids.clear();
    let shelves = with_shelves(&ui, &m, &m.library.tracks, <[_]>::to_vec);
    assert!(shelves[0].compact, "a strip of three under the hero");
    assert_eq!(shelves[0].cards.len(), 3);
    act(&mut ui, &mut m, Action::WheelClockwise(1));
    assert_eq!(locate(&shelves, ui.gallery.focus.get()), Some((0, 0)));
    assert_eq!(focus_count(&ui, &m), 1);
}

#[test]
fn standard_faces_read_in_gallery_type() {
    let look = Gallery;
    assert_eq!(look.face(Face::R15), Face::DM15);
    assert_eq!(look.face(Face::S20), Face::SY20);
    assert_eq!(look.face(Face::DM12), Face::DM12);
    assert_eq!(look.face(Face::S36), Face::S36);
}

//! Music search: an index of the library's artists, albums and songs, built
//! once per library, and a ranked, grouped query against it on every
//! keystroke.
//!
//! Names are compared folded (`sort::fold`: case, accents and punctuation
//! ignored), and a leading "The " or "A " is optional. A name matches a
//! query, best first, when it is the query; starts with it; has a word that
//! starts with it; has initials that start with it ("tps" is The Postal
//! Service); or contains it anywhere. Ties go A to Z.
//!
//! Typing only ever adds to the query, so each keystroke looks again only at
//! what could still match the shorter query (`Searcher`).
use crate::{
    sort::{fold, sort_key, SortAs},
    Track,
};
use std::collections::BTreeMap;

/// The groups of results, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Artists,
    Albums,
    Songs,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Artists, Kind::Albums, Kind::Songs];

    fn slot(self) -> usize {
        self as usize
    }

    /// Stable identifier used in row keys.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Artists => "artists",
            Kind::Albums => "albums",
            Kind::Songs => "songs",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// The group's heading.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Artists => "Artists",
            Kind::Albums => "Albums",
            Kind::Songs => "Songs",
        }
    }

    /// "1 song", "23 songs".
    pub fn count(self, n: usize) -> String {
        let (one, many) = match self {
            Kind::Artists => ("artist", "artists"),
            Kind::Albums => ("album", "albums"),
            Kind::Songs => ("song", "songs"),
        };
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    }
}

/// How well a name matches; lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Exact = 0,
    /// The name (or the name after "The ") starts with the query.
    Start = 1,
    /// A later word starts with the query.
    Word = 2,
    Initials = 3,
    Substring = 4,
}

/// A name folded once, with what matching needs precomputed.
#[derive(Debug, Clone, Default)]
struct Name {
    text: Box<str>,
    /// Byte offset past a leading "the " or "a " (0 when there is none).
    core: u16,
    /// Byte offsets where the second and later words start.
    starts: Box<[u16]>,
    /// The first letter of every word ("tps").
    initials: Box<str>,
}

impl Name {
    fn new(s: &str) -> Self {
        let text = fold(s);
        let core = ["the ", "a "]
            .iter()
            .find(|a| text.len() > a.len() && text.starts_with(*a))
            .map_or(0, |a| a.len());
        // Folded names are at most 240 bytes, so offsets fit in u16.
        let starts: Vec<u16> = text
            .match_indices(' ')
            .map(|(i, _)| (i + 1) as u16)
            .collect();
        let initials = text.split(' ').filter_map(|w| w.chars().next()).collect();
        Self {
            text: text.into(),
            core: core as u16,
            starts: starts.into(),
            initials,
        }
    }

    /// The initials without a leading article's ("ps" for The Postal Service).
    fn core_initials(&self) -> &str {
        if self.core > 0 {
            &self.initials[1..]
        } else {
            &self.initials
        }
    }

    fn tier(&self, q: &str, compact: &str) -> Option<Tier> {
        let text = &*self.text;
        let core = &text[usize::from(self.core)..];
        if text == q || core == q {
            return Some(Tier::Exact);
        }
        if core.starts_with(q) || text.starts_with(q) {
            return Some(Tier::Start);
        }
        if self
            .starts
            .iter()
            .any(|&s| text[usize::from(s)..].starts_with(q))
        {
            return Some(Tier::Word);
        }
        // A single letter is never initials; it would match nearly everything.
        if compact.len() >= 2
            && (self.initials.starts_with(compact) || self.core_initials().starts_with(compact))
        {
            return Some(Tier::Initials);
        }
        if q.len() >= 2 && text.contains(q) {
            return Some(Tier::Substring);
        }
        None
    }

    /// Whether some longer query that starts with `q` could still match.
    fn could_match(&self, q: &str, compact: &str) -> bool {
        self.text.contains(q)
            || self.initials.starts_with(compact)
            || self.core_initials().starts_with(compact)
    }
}

/// One artist, album or song in the index.
#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    /// Songs: the song's library index. Artists and albums: a song carrying
    /// the cover.
    pub track: usize,
    /// The name shown (empty for songs; their track has it).
    pub name: String,
    /// Albums: the album's artist. Empty otherwise.
    pub detail: String,
    /// The row key it opens, as the library lists name it (empty for songs).
    pub key: String,
    /// Songs in it (1 for a song).
    pub count: usize,
    /// The main name, and (songs, albums) the name with the artist after it,
    /// so "creep radiohead" finds the song.
    names: [Option<Name>; 2],
    /// A to Z order (`sort_key`), for ties.
    sort: Box<str>,
}

impl Entry {
    fn new(kind: Kind, track: usize, name: &str, with: Option<String>) -> Self {
        let order = if kind == Kind::Artists {
            SortAs::Artist
        } else {
            SortAs::Title
        };
        Self {
            kind,
            track,
            name: String::new(),
            detail: String::new(),
            key: String::new(),
            count: 1,
            names: [Some(Name::new(name)), with.map(|w| Name::new(&w))],
            sort: sort_key(name, order).into(),
        }
    }

    /// The best tier over the names as a score (the second name ranks a
    /// little after the first in the same tier).
    fn score(&self, q: &str, compact: &str) -> Option<u16> {
        self.names
            .iter()
            .enumerate()
            .filter_map(|(field, name)| {
                let tier = name.as_ref()?.tier(q, compact)?;
                Some(tier as u16 * 2 + field as u16)
            })
            .min()
    }

    fn could_match(&self, q: &str, compact: &str) -> bool {
        self.names
            .iter()
            .flatten()
            .any(|n| n.could_match(q, compact))
    }
}

/// Everything search looks through, built from the library in one go.
#[derive(Debug, Clone, Default)]
pub struct Index {
    groups: [Vec<Entry>; 3],
}

fn title_of(t: &Track) -> &str {
    if t.title.is_empty() {
        &t.filename
    } else {
        &t.title
    }
}

impl Index {
    /// Index the songs on inserted media, their artists and their albums.
    pub fn build(tracks: &[Track]) -> Self {
        // Per artist or album: the cover song, whether it has artwork, songs.
        let mut artists: BTreeMap<&str, (usize, bool, usize)> = BTreeMap::new();
        let mut albums: BTreeMap<(&str, &str), (usize, bool, usize)> = BTreeMap::new();
        let count = |e: &mut (usize, bool, usize), i: usize, art: bool| {
            e.2 += 1;
            if !e.1 && art {
                (e.0, e.1) = (i, true);
            }
        };
        let online = || tracks.iter().enumerate().filter(|(_, t)| t.online);
        for (i, t) in online() {
            if !t.artist.is_empty() {
                let e = artists.entry(&t.artist).or_insert((i, t.artwork, 0));
                count(e, i, t.artwork);
            }
            if !t.album.is_empty() {
                let e = albums
                    .entry((&t.album, &t.album_artist))
                    .or_insert((i, t.artwork, 0));
                count(e, i, t.artwork);
            }
        }
        let mut index = Index::default();
        for (name, (art, _, songs)) in artists {
            let mut e = Entry::new(Kind::Artists, art, name, None);
            e.name = name.to_owned();
            e.key = format!("artist:{name}");
            e.count = songs;
            index.groups[Kind::Artists.slot()].push(e);
        }
        for ((album, album_artist), (art, _, songs)) in albums {
            let t = &tracks[art];
            let artist = if album_artist.is_empty() {
                t.artist.as_str()
            } else {
                album_artist
            };
            let mut e = Entry::new(Kind::Albums, art, album, Some(format!("{album} {artist}")));
            e.name = album.to_owned();
            e.detail = artist.to_owned();
            e.key = format!("album:{album_artist}\u{1f}{album}");
            e.count = songs;
            index.groups[Kind::Albums.slot()].push(e);
        }
        for (i, t) in online() {
            let title = title_of(t);
            let with = (!t.artist.is_empty()).then(|| format!("{title} {}", t.artist));
            index.groups[Kind::Songs.slot()].push(Entry::new(Kind::Songs, i, title, with));
        }
        index
    }

    pub fn entry(&self, kind: Kind, at: usize) -> Option<&Entry> {
        self.groups[kind.slot()].get(at)
    }
}

/// One group of results: how many matched, and the best of them (entry
/// positions in the index's group), best first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub kind: Kind,
    pub total: usize,
    pub hits: Vec<usize>,
}

/// Runs queries against one index, remembering what the last query could
/// still match so the next, longer one looks only there.
#[derive(Debug, Clone, Default)]
pub struct Searcher {
    last: String,
    candidates: [Vec<u32>; 3],
}

impl Searcher {
    /// Forget the last query (the index changed).
    pub fn reset(&mut self) {
        self.last.clear();
        self.candidates.iter_mut().for_each(Vec::clear);
    }

    /// The groups matching `query` that have results, each holding at most
    /// `shown(kind)` hits.
    pub fn run(&mut self, index: &Index, query: &str, shown: impl Fn(Kind) -> usize) -> Vec<Group> {
        let q = fold(query);
        if q.is_empty() {
            self.reset();
            return vec![];
        }
        let compact: String = q.chars().filter(|c| *c != ' ').collect();
        let narrow = !self.last.is_empty() && q.starts_with(self.last.as_str());
        let mut groups = vec![];
        for kind in Kind::ALL {
            let entries = &index.groups[kind.slot()];
            let previous = std::mem::take(&mut self.candidates[kind.slot()]);
            let mut next = Vec::new();
            let mut hits: Vec<(u16, usize)> = Vec::new();
            let mut visit = |at: usize| {
                let e = &entries[at];
                if !e.could_match(&q, &compact) {
                    return;
                }
                next.push(at as u32);
                if let Some(score) = e.score(&q, &compact) {
                    hits.push((score, at));
                }
            };
            if narrow {
                previous.iter().for_each(|&at| visit(at as usize));
            } else {
                (0..entries.len()).for_each(&mut visit);
            }
            self.candidates[kind.slot()] = next;
            let total = hits.len();
            let shown = shown(kind).min(total);
            if shown == 0 {
                continue;
            }
            let order = |a: &(u16, usize), b: &(u16, usize)| {
                a.0.cmp(&b.0)
                    .then_with(|| entries[a.1].sort.cmp(&entries[b.1].sort))
                    .then(a.1.cmp(&b.1))
            };
            if shown < total {
                hits.select_nth_unstable_by(shown - 1, order);
                hits.truncate(shown);
            }
            hits.sort_unstable_by(order);
            groups.push(Group {
                kind,
                total,
                hits: hits.into_iter().map(|h| h.1).collect(),
            });
        }
        self.last = q;
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, album: &str) -> Track {
        Track {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: artist.into(),
            filename: format!("{title}.flac"),
            artwork: true,
            online: true,
            ..Default::default()
        }
    }

    fn library() -> Vec<Track> {
        vec![
            track("Such Great Heights", "The Postal Service", "Give Up"),
            track(
                "The District Sleeps Alone Tonight",
                "The Postal Service",
                "Give Up",
            ),
            track("Creep", "Radiohead", "Pablo Honey"),
            track("Paranoid Android", "Radiohead", "OK Computer"),
            track(
                "Lumière à l'horizon",
                "Émilie & the Northern Lights",
                "Paysage",
            ),
            track("Don't Stop Me Now", "Queen", "Jazz"),
            track("Heights", "Unknown Pleasures", "Heights"),
            track("Postcards", "James Blunt", "Back to Bedlam"),
            track("Losing My Religion", "R.E.M.", "Out of Time"),
        ]
    }

    /// The names found for `q`, group by group.
    fn run(index: &Index, q: &str) -> Vec<(Kind, Vec<String>)> {
        let tracks = library();
        Searcher::default()
            .run(index, q, |_| 50)
            .into_iter()
            .map(|g| {
                let names = g
                    .hits
                    .iter()
                    .map(|&at| {
                        let e = index.entry(g.kind, at).unwrap();
                        match g.kind {
                            Kind::Songs => tracks[e.track].title.clone(),
                            _ => e.name.clone(),
                        }
                    })
                    .collect();
                (g.kind, names)
            })
            .collect()
    }

    fn found(index: &Index, kind: Kind, q: &str) -> Vec<String> {
        run(index, q)
            .into_iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, s)| s)
            .unwrap_or_default()
    }

    #[test]
    fn names_match_from_any_word_start_and_without_the_article() {
        let index = Index::build(&library());
        let artists = |q| found(&index, Kind::Artists, q);
        let songs = |q| found(&index, Kind::Songs, q);
        assert_eq!(artists("postal"), ["The Postal Service"]);
        assert_eq!(artists("the postal"), ["The Postal Service"]);
        assert_eq!(artists("serv"), ["The Postal Service"]);
        assert_eq!(artists("EMILIE"), ["Émilie & the Northern Lights"]);
        assert_eq!(artists("rem"), ["R.E.M."]);
        assert_eq!(songs("lhoriz"), ["Lumière à l'horizon"]);
        assert_eq!(songs("dont stop"), ["Don't Stop Me Now"]);
        // The song's artist after its title: "creep radiohead".
        assert_eq!(songs("creep radio"), ["Creep"]);
        assert!(run(&index, "zzz").is_empty());
        assert!(run(&index, "   ").is_empty());
    }

    #[test]
    fn initials_find_names_with_or_without_the_article() {
        let index = Index::build(&library());
        let all = run(&index, "tps");
        assert_eq!(all[0], (Kind::Artists, vec!["The Postal Service".into()]));
        assert_eq!(found(&index, Kind::Artists, "ps"), ["The Postal Service"]);
        assert_eq!(found(&index, Kind::Songs, "sg"), ["Such Great Heights"]);
        assert_eq!(
            found(&index, Kind::Songs, "tdsat"),
            ["The District Sleeps Alone Tonight"]
        );
    }

    #[test]
    fn ranking_is_exact_then_start_then_word_then_initials_then_substring() {
        let index = Index::build(&library());
        let songs = |q| found(&index, Kind::Songs, q);
        // "heights": exact (Heights), then a later word (Such Great Heights).
        assert_eq!(songs("heights"), ["Heights", "Such Great Heights"]);
        // "post": a title's start (Postcards) before the songs that match
        // through their artist, a weaker field; those go A to Z.
        assert_eq!(
            songs("post"),
            [
                "Postcards",
                "The District Sleeps Alone Tonight",
                "Such Great Heights"
            ]
        );
        // Inside a word only: substring, A to Z.
        assert_eq!(songs("eight"), ["Heights", "Such Great Heights"]);
        assert_eq!(found(&index, Kind::Albums, "edlam"), ["Back to Bedlam"]);
    }

    #[test]
    fn groups_come_in_order_with_totals_and_open_their_pages() {
        let index = Index::build(&library());
        let groups =
            Searcher::default().run(&index, "h", |k| if k == Kind::Songs { 1 } else { 10 });
        let kinds: Vec<_> = groups.iter().map(|g| g.kind).collect();
        assert_eq!(kinds, [Kind::Albums, Kind::Songs]);
        let songs = &groups[1];
        assert_eq!(songs.hits.len(), 1, "cut to what is shown");
        assert_eq!(songs.total, 2, "but all are counted");
        assert_eq!(index.entry(Kind::Songs, songs.hits[0]).unwrap().track, 6);
        // Album and artist rows carry the keys the library pages use.
        let album = index.entry(Kind::Albums, groups[0].hits[0]).unwrap();
        assert_eq!(album.key, "album:Unknown Pleasures\u{1f}Heights");
        assert_eq!(
            (album.detail.as_str(), album.count),
            ("Unknown Pleasures", 1)
        );
        let radiohead = Searcher::default().run(&index, "radiohead", |_| 5);
        let artist = index.entry(Kind::Artists, radiohead[0].hits[0]).unwrap();
        assert_eq!((artist.key.as_str(), artist.count), ("artist:Radiohead", 2));
    }

    #[test]
    fn songs_on_removed_media_are_not_found() {
        let mut tracks = library();
        tracks[2].online = false;
        let index = Index::build(&tracks);
        assert!(found(&index, Kind::Songs, "creep").is_empty());
        assert!(found(&index, Kind::Albums, "pablo").is_empty());
        assert_eq!(found(&index, Kind::Artists, "radiohead"), ["Radiohead"]);
    }

    #[test]
    fn a_longer_query_narrows_the_last_one_with_the_same_answer() {
        let mut big = vec![];
        for n in 0..300 {
            big.extend(library().into_iter().map(|mut t| {
                t.title = format!("{} {n}", t.title);
                t
            }));
        }
        let index = Index::build(&big);
        let mut typing = Searcher::default();
        let mut typed = String::new();
        for ch in "such gr".chars() {
            typed.push(ch);
            let narrowed = typing.run(&index, &typed, |_| 20);
            let fresh = Searcher::default().run(&index, &typed, |_| 20);
            assert_eq!(narrowed, fresh, "after {typed:?}");
        }
        // Deleting a character is a fresh search.
        let shorter = typing.run(&index, "such", |_| 20);
        assert_eq!(shorter, Searcher::default().run(&index, "such", |_| 20));
        // Initials survive narrowing too.
        typing.reset();
        typing.run(&index, "t", |_| 20);
        let tps = typing.run(&index, "tps", |_| 20);
        assert_eq!(tps, Searcher::default().run(&index, "tps", |_| 20));
        assert!(!tps.is_empty());
    }
}

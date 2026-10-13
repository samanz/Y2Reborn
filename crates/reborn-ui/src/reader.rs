//! The news reader: a story laid out for the 480x360 screen. The title,
//! source, byline, age and reading time lead; paragraphs follow at a
//! comfortable measure. The wheel scrolls (faster the faster it turns, eased
//! between positions); Next and Previous change story; Select changes the
//! text size. Until the full story arrives, or when it can't be had, the
//! feed's summary stands in with a note saying so.
use crate::{
    canvas::Canvas,
    text::{self, Face},
    theme::{color, WIDTH},
};
use reborn_core::news::{age, reading_minutes, Article, BlockKind, Headline};
use std::time::{Duration, Instant};

/// Text left and right margins: about 65 characters a line at Medium.
const MARGIN: f32 = 26.;
const MEASURE: f32 = WIDTH - 2. * MARGIN;
/// The window the story scrolls through (below the status bar, above the
/// hint and progress bar).
pub const VIEW_TOP: f32 = 46.;
pub const VIEW_BOTTOM: f32 = 326.;
const VIEW_HEIGHT: f32 = VIEW_BOTTOM - VIEW_TOP;
/// How long the eased move to a new scroll position takes.
const GLIDE: Duration = Duration::from_millis(140);
/// Wheel detents closer together than this count as one fast turn.
const BURST: Duration = Duration::from_millis(110);

/// Reader text sizes: name, face and line height.
pub const SIZES: [(&str, Face, f32); 3] = [
    ("Small", Face::R13, 20.),
    ("Medium", Face::R15, 23.),
    ("Large", Face::R17, 26.),
];
pub const DEFAULT_SIZE: u8 = 1;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ArticleState {
    /// No full story to fetch (the feed gave no link).
    #[default]
    None,
    Loading,
    Ready(Box<Article>),
    Failed(String),
}

/// The story open in the reader and where it is scrolled to.
#[derive(Debug, Clone, Default)]
pub struct Reader {
    /// The story's link (empty: none).
    pub url: String,
    pub state: ArticleState,
    pub scroll: Scroll,
}

impl Reader {
    pub fn open(url: &str) -> Self {
        Reader {
            url: url.to_owned(),
            state: if url.is_empty() {
                ArticleState::None
            } else {
                ArticleState::Loading
            },
            scroll: Scroll::default(),
        }
    }
}

/// An eased scroll position, in pixels from the top of the story.
#[derive(Debug, Clone, Default)]
pub struct Scroll {
    from: f32,
    pub target: f32,
    started: Option<Instant>,
    last_turn: Option<Instant>,
    /// Detents in the current fast turn.
    streak: u32,
}

impl Scroll {
    /// The position to draw at `now`.
    pub fn at(&self, now: Instant) -> f32 {
        let Some(started) = self.started else {
            return self.target;
        };
        let t = now.saturating_duration_since(started).as_secs_f32() / GLIDE.as_secs_f32();
        if t >= 1. {
            return self.target;
        }
        // Ease out (cubic).
        let e = 1. - (1. - t).powi(3);
        self.from + (self.target - self.from) * e
    }

    pub fn moving(&self, now: Instant) -> bool {
        self.started
            .is_some_and(|s| now.saturating_duration_since(s) < GLIDE)
    }

    /// Glide to `to` (clamped to the story).
    pub fn go(&mut self, to: f32, max: f32, now: Instant) {
        let to = to.clamp(0., max.max(0.));
        self.from = self.at(now);
        self.target = to;
        self.started = Some(now);
    }

    /// Jump without easing (a new story, a new text size).
    pub fn jump(&mut self, to: f32, max: f32) {
        self.target = to.clamp(0., max.max(0.));
        self.from = self.target;
        self.started = None;
    }

    /// Wheel `detents` (negative: up). A quick run of detents speeds up,
    /// to four times the line-and-a-half step.
    pub fn wheel(&mut self, detents: i32, line: f32, max: f32, now: Instant) {
        let quick = self
            .last_turn
            .is_some_and(|t| now.saturating_duration_since(t) < BURST);
        self.streak = if quick {
            self.streak.saturating_add(detents.unsigned_abs())
        } else {
            0
        };
        self.last_turn = Some(now);
        let speed = 1. + (self.streak.min(12) as f32) / 4.;
        let step = line * 1.5 * speed * detents as f32;
        self.go(self.target + step, max, now);
    }
}

/// What colour and indent a laid-out line takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Title,
    Meta,
    Note,
    Body,
    Heading,
    Quote,
    /// A list item's first line (drawn with a bullet).
    Bullet,
    Item,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Top of the line, pixels from the top of the story.
    pub y: f32,
    pub height: f32,
    pub text: String,
    pub face: Face,
    pub tone: Tone,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub lines: Vec<Line>,
    /// The whole story's height.
    pub height: f32,
    /// Where each block of the body starts (Down and Up jump between them).
    pub starts: Vec<f32>,
}

impl Layout {
    /// The furthest the story scrolls.
    pub fn max_scroll(&self) -> f32 {
        (self.height - VIEW_HEIGHT).max(0.)
    }

    /// 0 at the top, 1 at the end (1 when it all fits).
    pub fn progress(&self, scroll: f32) -> f32 {
        let max = self.max_scroll();
        if max <= 0. {
            1.
        } else {
            (scroll / max).clamp(0., 1.)
        }
    }

    /// The next block start below `scroll` (or above, going back).
    pub fn next_block(&self, scroll: f32, forward: bool) -> f32 {
        let found = if forward {
            self.starts.iter().copied().find(|&s| s > scroll + 1.)
        } else {
            self.starts.iter().rev().copied().find(|&s| s < scroll - 1.)
        };
        found
            .unwrap_or(if forward { self.max_scroll() } else { 0. })
            .min(self.max_scroll())
    }

    /// The lines wholly inside the view at `scroll`.
    pub fn visible(&self, scroll: f32) -> impl Iterator<Item = &Line> {
        self.lines.iter().filter(move |l| {
            let top = VIEW_TOP + l.y - scroll;
            top >= VIEW_TOP - 0.5 && top + l.height <= VIEW_BOTTOM + 0.5
        })
    }
}

/// What a story's header says.
pub struct Story<'a> {
    pub headline: &'a Headline,
    pub source: &'a str,
    pub reader: &'a Reader,
    pub saved: bool,
    pub size: u8,
    pub now: i64,
}

/// The note shown above the summary when the full story isn't shown.
fn note(state: &ArticleState) -> Option<String> {
    match state {
        ArticleState::None => Some("This feed gives only a summary".into()),
        ArticleState::Loading => Some("Loading the full story…".into()),
        ArticleState::Failed(e) => Some(format!("{e}. Here is the summary.")),
        ArticleState::Ready(a) if a.paywalled => {
            Some("Subscriber story: the site may show only part of it".into())
        }
        ArticleState::Ready(_) => None,
    }
}

/// Lines placed one under another.
struct Builder {
    out: Layout,
    y: f32,
}

impl Builder {
    fn line(&mut self, text: String, face: Face, height: f32, tone: Tone) {
        self.out.lines.push(Line {
            y: self.y,
            height,
            text,
            face,
            tone,
        });
        self.y += height;
    }

    fn wrap(&mut self, text: &str, face: Face, height: f32, tone: Tone, width: f32) {
        for l in text::wrap(face, text, width, 400) {
            self.line(l, face, height, tone);
        }
    }

    /// The next block starts here.
    fn start(&mut self) {
        self.out.starts.push(self.y);
    }
}

pub(crate) fn layout(s: &Story) -> Layout {
    let (_, face, line) = SIZES[usize::from(s.size.min(2))];
    let mut b = Builder {
        out: Layout::default(),
        y: 6.,
    };
    b.wrap(&s.headline.title, Face::S20, 26., Tone::Title, MEASURE);
    b.y += 4.;
    let article = match &s.reader.state {
        ArticleState::Ready(a) => Some(a.as_ref()),
        _ => None,
    };
    let byline = article.map_or("", |a| a.byline.as_str());
    let meta = [
        s.source.to_owned(),
        if byline.is_empty() {
            String::new()
        } else {
            format!("By {byline}")
        },
        age(s.headline.published, s.now).unwrap_or_default(),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    b.wrap(&meta, Face::R13, 19., Tone::Meta, MEASURE);
    let words = article.map_or_else(
        || s.headline.summary.split_whitespace().count(),
        Article::words,
    );
    let minutes = reading_minutes(words);
    let mut second = format!("Reading time {minutes} min");
    if s.saved {
        second.push_str(" · Saved for offline");
    }
    b.line(second, Face::R13, 19., Tone::Meta);
    b.y += line * 0.7;
    if let Some(n) = note(&s.reader.state) {
        b.wrap(&n, Face::R13, 19., Tone::Note, MEASURE);
        b.y += line * 0.4;
    }
    let gap = (line * 0.55).round();
    let Some(article) = article else {
        b.start();
        b.wrap(&s.headline.summary, face, line, Tone::Body, MEASURE);
        b.out.height = b.y + gap + 8.;
        return b.out;
    };
    let heading = if s.size == 0 { Face::S15 } else { Face::S17 };
    for block in &article.blocks {
        let text = block.text.as_str();
        match block.kind {
            BlockKind::Heading => {
                b.y += gap * 0.5;
                b.start();
                b.wrap(text, heading, line + 2., Tone::Heading, MEASURE);
                b.y += gap * 0.4;
            }
            BlockKind::Paragraph => {
                b.start();
                b.wrap(text, face, line, Tone::Body, MEASURE);
                b.y += gap;
            }
            BlockKind::Quote => {
                b.start();
                b.wrap(text, face, line, Tone::Quote, MEASURE - QUOTE_INDENT);
                b.y += gap;
            }
            BlockKind::Item => {
                b.start();
                let first = b.out.lines.len();
                b.wrap(text, face, line, Tone::Item, MEASURE - ITEM_INDENT);
                if let Some(l) = b.out.lines.get_mut(first) {
                    l.tone = Tone::Bullet;
                }
                b.y += gap * 0.4;
            }
        }
    }
    b.y += gap;
    b.line("End of story".into(), Face::R13, 22., Tone::End);
    b.out.height = b.y + 8.;
    b.out
}
const QUOTE_INDENT: f32 = 16.;
const ITEM_INDENT: f32 = 16.;

/// Draw the story under the status bar (which the caller draws): the
/// visible lines, the hint and the reading-progress bar.
pub(crate) fn draw(c: &mut Canvas, l: &Layout, scroll: f32, hint: &str, accent: u32) {
    for line in l.visible(scroll) {
        let top = VIEW_TOP + line.y - scroll;
        let cy = top + line.height / 2.;
        let (x, color) = match line.tone {
            Tone::Title | Tone::Heading | Tone::Body => (MARGIN, color::TEXT),
            Tone::Meta => (MARGIN, color::MUTED),
            Tone::Note => (MARGIN, color::WARNING),
            Tone::Quote => {
                // One bar down the whole quote: each line draws its part.
                c.rect(MARGIN + 2., top, 3., line.height, accent);
                (MARGIN + QUOTE_INDENT, color::MUTED)
            }
            Tone::Bullet | Tone::Item => {
                if line.tone == Tone::Bullet {
                    c.disc(MARGIN + 5., cy, 2.5, color::MUTED);
                }
                (MARGIN + ITEM_INDENT, color::TEXT)
            }
            Tone::End => {
                c.text_mid_center(WIDTH / 2., cy, &line.text, line.face, color::DIM);
                continue;
            }
        };
        c.text_mid(x, cy, MEASURE, &line.text, line.face, color);
    }
    c.text_mid(MARGIN, 338., MEASURE, hint, Face::R11, color::DIM);
    // Reading progress along the bottom edge.
    let width = WIDTH - 2. * MARGIN;
    c.rrect(MARGIN, 350., width, 3., 1.5, color::TRACK);
    let done = (width * l.progress(scroll)).max(3.);
    c.rrect(MARGIN, 350., done, 3., 1.5, accent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use reborn_core::news::Block;

    fn headline() -> Headline {
        Headline {
            title: "A headline long enough to wrap onto a second line on the player's screen"
                .into(),
            summary: "The summary from the feed, a sentence or two.".into(),
            published: Some(1000),
            link: "https://example.com/story".into(),
        }
    }

    fn article(paragraphs: usize) -> Article {
        let mut blocks = vec![];
        for i in 0..paragraphs {
            if i % 5 == 4 {
                blocks.push(Block {
                    kind: BlockKind::Heading,
                    text: format!("Section {i}"),
                });
            }
            let kind = match i % 7 {
                3 => BlockKind::Quote,
                5 => BlockKind::Item,
                _ => BlockKind::Paragraph,
            };
            blocks.push(Block {
                kind,
                text: "Words that fill a paragraph of a story, one after another. ".repeat(4),
            });
        }
        Article {
            url: "https://example.com/story".into(),
            title: "t".into(),
            byline: "Jane Doe".into(),
            blocks,
            paywalled: false,
        }
    }

    fn story<'a>(h: &'a Headline, r: &'a Reader, size: u8) -> Story<'a> {
        Story {
            headline: h,
            source: "Example News",
            reader: r,
            saved: true,
            size,
            now: 1000 + 7200,
        }
    }

    fn ready(paragraphs: usize) -> Reader {
        Reader {
            state: ArticleState::Ready(Box::new(article(paragraphs))),
            ..Reader::open("https://example.com/story")
        }
    }

    #[test]
    fn header_reads_source_byline_age_and_reading_time() {
        let h = headline();
        let r = ready(20);
        let l = layout(&story(&h, &r, 1));
        let texts: Vec<&str> = l.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(l.lines[0].tone, Tone::Title);
        assert!(
            texts.contains(&"Example News · By Jane Doe · 2 h ago"),
            "{texts:?}"
        );
        let words = article(20).words();
        let minutes = words.div_ceil(220);
        assert!(texts.contains(&format!("Reading time {minutes} min · Saved for offline").as_str()));
        assert_eq!(texts.last(), Some(&"End of story"));
        assert!(!l.lines.iter().any(|l| l.tone == Tone::Note));
    }

    #[test]
    fn lines_fit_the_measure_and_never_overlap() {
        let h = headline();
        let r = ready(30);
        for size in 0..3 {
            let l = layout(&story(&h, &r, size));
            for pair in l.lines.windows(2) {
                assert!(pair[1].y >= pair[0].y + pair[0].height - 0.01);
            }
            for line in &l.lines {
                assert!(
                    text::width(line.face, &line.text) <= MEASURE + 0.5,
                    "{line:?}"
                );
            }
            assert!(l.height > l.lines.last().unwrap().y);
        }
    }

    #[test]
    fn bigger_text_makes_a_longer_story() {
        let h = headline();
        let r = ready(30);
        let heights: Vec<f32> = (0..3).map(|s| layout(&story(&h, &r, s)).height).collect();
        assert!(
            heights[0] < heights[1] && heights[1] < heights[2],
            "{heights:?}"
        );
    }

    #[test]
    fn summary_stands_in_with_a_note() {
        let h = headline();
        for (state, note) in [
            (ArticleState::Loading, "Loading the full story…"),
            (
                ArticleState::Failed("No internet connection".into()),
                "No internet connection. Here is the summary.",
            ),
        ] {
            let r = Reader {
                state,
                ..Reader::open(&h.link)
            };
            let l = layout(&story(&h, &r, 1));
            let n = l.lines.iter().find(|l| l.tone == Tone::Note).unwrap();
            assert_eq!(n.text, note);
            assert!(l.lines.iter().any(|l| l.text.starts_with("The summary")));
            assert!(l
                .lines
                .iter()
                .any(|l| l.text == "Reading time 1 min · Saved for offline"));
        }
    }

    #[test]
    fn scroll_bounds_progress_and_block_jumps() {
        let h = headline();
        let r = ready(30);
        let l = layout(&story(&h, &r, 1));
        let max = l.max_scroll();
        assert!(max > 1000.);
        assert_eq!(l.progress(0.), 0.);
        assert_eq!(l.progress(max), 1.);
        assert!((l.progress(max / 2.) - 0.5).abs() < 1e-4);
        // Down from the top lands on the first block below; Up returns.
        let first = l.next_block(0., true);
        assert_eq!(first, l.starts[0]);
        let second = l.next_block(first, true);
        assert_eq!(second, l.starts[1]);
        assert_eq!(l.next_block(second, false), first);
        assert_eq!(l.next_block(first, false), 0.);
        assert_eq!(l.next_block(max, true), max);
        // Visible lines lie inside the window.
        for scroll in [0., 123., max] {
            let lines: Vec<_> = l.visible(scroll).collect();
            assert!(lines.len() >= 8, "{scroll}");
            for line in lines {
                let top = VIEW_TOP + line.y - scroll;
                assert!(top >= VIEW_TOP - 0.5 && top + line.height <= VIEW_BOTTOM + 0.5);
            }
        }
        // A short story doesn't scroll and reads as complete.
        let short = Reader {
            state: ArticleState::Ready(Box::new(article(1))),
            ..Reader::open(&h.link)
        };
        let l = layout(&story(&h, &short, 1));
        assert_eq!(l.max_scroll(), 0.);
        assert_eq!(l.progress(0.), 1.);
    }

    #[test]
    fn wheel_accelerates_and_glides() {
        let t0 = Instant::now();
        let mut s = Scroll::default();
        s.wheel(1, 20., 10_000., t0);
        assert_eq!(s.target, 30.);
        assert_eq!(s.at(t0), 0.);
        assert!(s.moving(t0 + Duration::from_millis(50)));
        let mid = s.at(t0 + Duration::from_millis(70));
        assert!(mid > 15. && mid < 30., "{mid}");
        assert_eq!(s.at(t0 + GLIDE), 30.);
        // A slow turn moves by the same step...
        let t1 = t0 + Duration::from_secs(1);
        s.wheel(1, 20., 10_000., t1);
        assert_eq!(s.target, 60.);
        // ...a quick run speeds up, to at most four times the step.
        let mut last = 30.;
        let mut at = t1;
        for _ in 0..20 {
            at += Duration::from_millis(30);
            let before = s.target;
            s.wheel(1, 20., 10_000., at);
            let step = s.target - before;
            assert!(step >= last - 0.01 && step <= 120.01, "{step}");
            last = step;
        }
        assert_eq!(last, 120.);
        // Turning back slows down again after a pause, and stops at the top.
        s.wheel(-100, 20., 10_000., at + Duration::from_secs(1));
        assert_eq!(s.target, 0.);
        s.wheel(5, 20., 100., at + Duration::from_secs(3));
        assert_eq!(s.target, 100.);
        s.jump(500., 100.);
        assert_eq!(s.at(at), 100.);
    }

    #[test]
    fn story_punctuation_has_glyphs_at_every_size() {
        for (_, face, _) in SIZES {
            for ch in ['’', '‘', '“', '”', '–', '—', '…', '·', '£', '€', 'é'] {
                assert!(
                    matches!(text::lookup(face, ch), Some(text::Source::Exact(_))),
                    "{ch:?} in {face:?}"
                );
            }
        }
    }

    #[test]
    fn draws_inside_the_display() {
        let h = headline();
        let r = ready(12);
        let l = layout(&story(&h, &r, 2));
        for scroll in [0., l.max_scroll() / 2., l.max_scroll()] {
            let mut c = Canvas::new();
            draw(&mut c, &l, scroll, "1 of 3", 0xff00_00ff);
            for q in c.finish() {
                assert!(q.x >= -16. && q.y >= VIEW_TOP - 16., "{q:?}");
                assert!(q.x + q.w <= 496. && q.y + q.h <= 376., "{q:?}");
            }
        }
    }
}

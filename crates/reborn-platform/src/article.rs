//! Full stories: fetch a story's page and keep its readable text.
//!
//! Two widely read sites have a route: a text-only page (NPR) or the
//! element that holds the story on their pages (BBC). Anything else goes
//! through a readability-style search for the densest cluster of
//! paragraphs. Scripts, navigation, figures, share bars, newsletter boxes
//! and "related" promos are dropped; headings, paragraphs, quotes and list
//! items are kept in order.
mod html;

use crate::{https, news::decode};
use html::Doc;
use reborn_core::news::{host, Article, Block, BlockKind};

/// Story pages are bigger than feeds; this bounds the memory one costs.
const MAX_PAGE: usize = 3 * 1024 * 1024;
/// Less readable text than this is not a story (a video page, a paywall).
const MIN_TEXT: usize = 300;

/// Where a site's story text lives on its pages.
#[derive(Clone, Copy)]
enum Sel {
    Tag(&'static str),
    /// A class token, exactly.
    Class(&'static str),
}

/// Host suffix to the story's container on that site's pages.
const ROUTES: [(&str, Sel); 3] = [
    ("text.npr.org", Sel::Class("paragraphs-container")),
    ("bbc.co.uk", Sel::Tag("article")),
    ("bbc.com", Sel::Tag("article")),
];

/// Fetch and extract the story at `url`.
pub fn fetch(url: &str) -> Result<Article, String> {
    let page = page_url(url);
    let r = https::get(&page, MAX_PAGE)?;
    if r.status != 200 {
        return Err(format!("The site answered {}", r.status));
    }
    let html = String::from_utf8_lossy(&r.body);
    extract(&html, &page).map(|mut a| {
        a.url = url.to_owned();
        a
    })
}

/// The page to fetch for a story link: NPR stories come from NPR's
/// text-only site (small, no scripts); everything else as linked.
pub fn page_url(url: &str) -> String {
    // Pages are fetched over HTTPS only; most sites serve both.
    let url = match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    };
    let url = url.as_str();
    let host = host(url);
    if host == "npr.org" || host == "www.npr.org" {
        // .../2026/10/12/nx-s1-5998948/slug: the id follows the date.
        let segments: Vec<&str> = path(url).split('/').filter(|s| !s.is_empty()).collect();
        for w in segments.windows(4) {
            let date = w[0].len() == 4
                && w[1].len() == 2
                && w[2].len() == 2
                && w[..3].iter().all(|s| s.bytes().all(|b| b.is_ascii_digit()));
            if date && !w[3].is_empty() {
                return format!("https://text.npr.org/{}", w[3]);
            }
        }
    }
    url.to_owned()
}

fn path(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.find('/').map_or("", |i| &rest[i..]);
    path.split(['?', '#']).next().unwrap_or("")
}

/// The readable story in a page. Errors are worded for the reader.
pub fn extract(page: &str, url: &str) -> Result<Article, String> {
    let doc = html::parse(page);
    let meta = Meta::read(&doc);
    let junk = junk_marks(&doc);
    let host = host(url);
    let route = ROUTES
        .iter()
        .find(|(h, _)| host == *h || host.ends_with(&format!(".{h}")))
        .map(|(_, sel)| *sel);
    let mut blocks = vec![];
    if let Some(sel) = route {
        let roots = select(&doc, sel, &junk);
        blocks = blocks_of(&doc, &roots, &junk);
    }
    if text_len(&blocks) < MIN_TEXT {
        if let Some(root) = densest(&doc, &junk) {
            blocks = blocks_of(&doc, &[root], &junk);
        }
    }
    if text_len(&blocks) < MIN_TEXT && !meta.paywalled {
        // Some sites only give the text as structured data.
        if let Some(body) = &meta.body {
            blocks = body
                .split('\n')
                .map(clean)
                .filter(|t| !t.is_empty())
                .map(|text| Block {
                    kind: BlockKind::Paragraph,
                    text,
                })
                .collect();
        }
    }
    let mut byline = meta.byline.clone();
    tidy(&mut blocks, &mut byline, &meta.title);
    if text_len(&blocks) < MIN_TEXT {
        return Err(if meta.paywalled {
            "This story is for subscribers".into()
        } else {
            "Couldn't find the story on this page".into()
        });
    }
    Ok(Article {
        url: url.to_owned(),
        title: meta.title,
        byline,
        blocks,
        paywalled: meta.paywalled,
    })
}

fn text_len(blocks: &[Block]) -> usize {
    blocks.iter().map(|b| b.text.len()).sum()
}

/// What the page says about itself (meta tags and JSON-LD).
#[derive(Default)]
struct Meta {
    title: String,
    byline: String,
    paywalled: bool,
    /// JSON-LD `articleBody`, when the page has one.
    body: Option<String>,
}

impl Meta {
    fn read(doc: &Doc) -> Self {
        let mut m = Meta::default();
        for id in doc.elements("meta") {
            let n = &doc.nodes[id];
            let key = n.attr("property").or(n.attr("name")).unwrap_or("");
            let value = clean(n.attr("content").unwrap_or(""));
            match key {
                "og:title" if m.title.is_empty() => m.title = value,
                "author" if m.byline.is_empty() && !value.starts_with("http") => m.byline = value,
                _ => {}
            }
        }
        if m.title.is_empty() {
            if let Some(t) = doc.elements("title").next() {
                m.title = clean(&doc.raw_text(t));
            }
        }
        for id in doc.elements("script") {
            if doc.nodes[id].attr("type") != Some("application/ld+json") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&doc.raw_text(id)) else {
                continue;
            };
            m.linked_data(&v, 0);
        }
        if m.byline.is_empty() {
            // A "By Jane Doe" line (NPR's text pages).
            m.byline = doc
                .elements("p")
                .map(|p| clean(&doc.raw_text(p)))
                .find(|t| t.starts_with("By ") && t.len() <= 80)
                .map(|t| t[3..].trim().to_owned())
                .unwrap_or_default();
        }
        m
    }

    /// Authors, paywall and body from JSON-LD (objects, arrays, @graph).
    fn linked_data(&mut self, v: &serde_json::Value, depth: u8) {
        use serde_json::Value;
        if depth > 6 {
            return;
        }
        match v {
            Value::Array(items) => items.iter().for_each(|i| self.linked_data(i, depth + 1)),
            Value::Object(o) => {
                let types: Vec<&str> = match o.get("@type") {
                    Some(Value::String(s)) => vec![s.as_str()],
                    Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
                    _ => vec![],
                };
                let story = types.iter().any(|t| {
                    matches!(
                        *t,
                        "NewsArticle"
                            | "Article"
                            | "ReportageNewsArticle"
                            | "AnalysisNewsArticle"
                            | "BlogPosting"
                            | "LiveBlogPosting"
                    )
                });
                if story {
                    let free = match o.get("isAccessibleForFree") {
                        Some(Value::Bool(b)) => *b,
                        Some(Value::String(s)) => !s.eq_ignore_ascii_case("false"),
                        _ => true,
                    };
                    self.paywalled |= !free;
                    if self.byline.is_empty() {
                        let names: Vec<String> = match o.get("author") {
                            Some(Value::Array(a)) => a.iter().filter_map(person).collect(),
                            Some(one) => person(one).into_iter().collect(),
                            None => vec![],
                        };
                        self.byline = join_names(&names);
                    }
                    if let Some(Value::String(body)) = o.get("articleBody") {
                        self.body = Some(body.clone());
                    }
                }
                if let Some(graph) = o.get("@graph") {
                    self.linked_data(graph, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn person(v: &serde_json::Value) -> Option<String> {
    let name = match v {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Object(o)
            if o.get("@type").and_then(|t| t.as_str()) != Some("Organization") =>
        {
            o.get("name")?.as_str()?
        }
        _ => return None,
    };
    Some(clean(name)).filter(|n| !n.is_empty() && !n.starts_with("http"))
}

/// "A", "A and B", "A, B and C".
fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Entities decoded, whitespace collapsed, and characters the player's
/// fonts lack replaced.
fn clean(text: &str) -> String {
    let decoded = decode(text);
    let mut out = String::with_capacity(decoded.len());
    for word in decoded.split_whitespace() {
        let word: String = word
            .chars()
            .filter(|c| !matches!(c, '\u{AD}' | '\u{200B}'..='\u{200D}' | '\u{FEFF}'))
            .map(|c| match c {
                '\u{2212}' => '-',
                c => c,
            })
            .collect();
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
    }
    out
}

/// Elements that never hold story text: scripts, navigation, figures,
/// forms, hidden parts and blocks named like promos, share bars or ads.
fn junk_marks(doc: &Doc) -> Vec<bool> {
    let mut junk = vec![false; doc.nodes.len()];
    for (id, n) in doc.nodes.iter().enumerate() {
        if n.is_text() {
            continue;
        }
        let parent_junk = id > 0 && junk[n.parent];
        junk[id] = parent_junk || is_junk(n);
    }
    junk
}

fn is_junk(n: &html::Node) -> bool {
    const TAGS: [&str; 18] = [
        "script", "style", "noscript", "template", "svg", "nav", "aside", "footer", "form",
        "button", "iframe", "figure", "select", "dialog", "head", "video", "audio", "picture",
    ];
    if TAGS.contains(&n.tag.as_str())
        || n.attr("aria-hidden") == Some("true")
        || n.attr("hidden").is_some()
        || matches!(
            n.attr("role"),
            Some("navigation" | "complementary" | "banner")
        )
    {
        return true;
    }
    // Words in class and id names that mark promos, ads and widgets.
    const PARTS: [&str; 15] = [
        "promo",
        "related",
        "newsletter",
        "advert",
        "sponsor",
        "taboola",
        "social",
        "sharing",
        "subscribe",
        "signup",
        "sign-up",
        "most-popular",
        "mostread",
        "breadcrumb",
        "popover",
    ];
    const WORDS: [&str; 12] = [
        "ad", "ads", "nav", "menu", "comments", "comment", "sidebar", "cookie", "modal", "toolbar",
        "share", "recirc",
    ];
    for value in [n.attr("class"), n.attr("id")].into_iter().flatten() {
        let lower = value.to_ascii_lowercase();
        if PARTS.iter().any(|p| lower.contains(p)) {
            return true;
        }
        if lower
            .split(|c: char| c.is_ascii_whitespace() || c == '-' || c == '_')
            .any(|w| WORDS.contains(&w))
        {
            return true;
        }
    }
    false
}

/// Outermost non-junk elements matching `sel`, in document order.
fn select(doc: &Doc, sel: Sel, junk: &[bool]) -> Vec<usize> {
    let matches = |id: usize| {
        let n = &doc.nodes[id];
        match sel {
            Sel::Tag(t) => n.tag == t,
            Sel::Class(c) => n.has_class(c),
        }
    };
    let mut out: Vec<usize> = vec![];
    for (id, &dropped) in junk.iter().enumerate().skip(1) {
        if dropped || doc.nodes[id].is_text() || !matches(id) {
            continue;
        }
        if out
            .last()
            .is_some_and(|&last| doc.ancestors(id).any(|a| a == last))
        {
            continue;
        }
        out.push(id);
    }
    out
}

/// The element holding most of the page's paragraph text: each good
/// paragraph scores its parent (and half for its grandparent); from the
/// best, climb while an ancestor up to three levels above holds clearly
/// more paragraph text (stories split across sibling containers).
fn densest(doc: &Doc, junk: &[bool]) -> Option<usize> {
    let n = doc.nodes.len();
    let mut score = vec![0f32; n];
    let mut text = vec![0usize; n];
    for id in doc.elements("p") {
        if junk[id] {
            continue;
        }
        let (len, link) = measure(doc, id, junk, false);
        if len < 25 || link * 2 > len {
            continue;
        }
        let parent = doc.nodes[id].parent;
        score[parent] += len as f32;
        let grand = doc.nodes[parent].parent;
        score[grand] += len as f32 / 2.;
        for a in doc.ancestors(id) {
            text[a] += len;
        }
    }
    let mut best = (1..n)
        .filter(|&i| !doc.nodes[i].is_text())
        .max_by(|&a, &b| score[a].total_cmp(&score[b]))?;
    if score[best] <= 0. {
        return None;
    }
    loop {
        let here = text[best];
        let up = doc
            .ancestors(best)
            .take(3)
            .take_while(|&a| !matches!(doc.nodes[a].tag.as_str(), "body" | "html"))
            .find(|&a| text[a] * 4 >= here * 5);
        let stop = matches!(doc.nodes[best].tag.as_str(), "article" | "main")
            || doc.nodes[best].attr("itemprop") == Some("articleBody");
        match up {
            Some(a) if !stop => best = a,
            _ => break,
        }
    }
    Some(best)
}

/// Text length under `id` and how much of it is link text.
fn measure(doc: &Doc, id: usize, junk: &[bool], in_link: bool) -> (usize, usize) {
    let node = &doc.nodes[id];
    if node.is_text() {
        let len = node.text.split_whitespace().map(|w| w.len() + 1).sum();
        return (len, if in_link { len } else { 0 });
    }
    if junk[id] {
        return (0, 0);
    }
    let link = in_link || node.tag == "a";
    node.children.iter().fold((0, 0), |(t, l), &c| {
        let (ct, cl) = measure(doc, c, junk, link);
        (t + ct, l + cl)
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Raw {
    /// Text loose in a container, not in a paragraph.
    Loose,
    Kind(BlockKind),
}

struct Sink {
    buf: String,
    link: usize,
    out: Vec<(Raw, String, f32)>,
}

impl Sink {
    fn flush(&mut self, kind: Raw) {
        let text = clean(&self.buf);
        if !text.is_empty() {
            let raw_len = self
                .buf
                .split_whitespace()
                .map(|w| w.len() + 1)
                .sum::<usize>();
            let links = self.link as f32 / raw_len.max(1) as f32;
            self.out.push((kind, text, links));
        }
        self.buf.clear();
        self.link = 0;
    }
}

fn blocks_of(doc: &Doc, roots: &[usize], junk: &[bool]) -> Vec<Block> {
    let mut sink = Sink {
        buf: String::new(),
        link: 0,
        out: vec![],
    };
    for &root in roots {
        walk(doc, root, junk, &mut sink, Raw::Loose, false);
        sink.flush(Raw::Loose);
    }
    sink.out
        .into_iter()
        .filter_map(|(raw, text, links)| {
            let len = text.chars().count();
            let kind = match raw {
                Raw::Loose if len >= 100 && links < 0.3 => BlockKind::Paragraph,
                Raw::Loose => return None,
                Raw::Kind(BlockKind::Item) if len < 15 || links >= 0.5 => return None,
                Raw::Kind(BlockKind::Heading) if len > 120 || links >= 0.5 => return None,
                Raw::Kind(_) if links >= 0.6 => return None,
                Raw::Kind(k) => k,
            };
            Some(Block { kind, text })
        })
        .collect()
}

fn walk(doc: &Doc, id: usize, junk: &[bool], sink: &mut Sink, ctx: Raw, in_link: bool) {
    let node = &doc.nodes[id];
    if node.is_text() {
        sink.buf.push_str(node.text);
        if in_link {
            sink.link += node
                .text
                .split_whitespace()
                .map(|w| w.len() + 1)
                .sum::<usize>();
        }
        return;
    }
    if junk[id] {
        return;
    }
    let quote = ctx == Raw::Kind(BlockKind::Quote);
    let kind = match node.tag.as_str() {
        "p" | "pre" if quote => Some(Raw::Kind(BlockKind::Quote)),
        "p" | "pre" => Some(Raw::Kind(BlockKind::Paragraph)),
        "h2" | "h3" | "h4" | "h5" | "h6" => Some(Raw::Kind(BlockKind::Heading)),
        // The title is shown from the feed.
        "h1" => return,
        "li" | "dd" => Some(Raw::Kind(BlockKind::Item)),
        "blockquote" => Some(Raw::Kind(BlockKind::Quote)),
        "br" => {
            sink.buf.push(' ');
            return;
        }
        "a" | "span" | "em" | "strong" | "b" | "i" | "u" | "s" | "time" | "abbr" | "sup"
        | "sub" | "small" | "mark" | "q" | "cite" | "code" | "font" | "label" | "del" | "ins"
        | "bdi" => None,
        // Containers: text loose in them is its own block.
        _ => Some(ctx),
    };
    let Some(kind) = kind else {
        let link = in_link || node.tag == "a";
        for &c in &node.children {
            walk(doc, c, junk, sink, ctx, link);
        }
        return;
    };
    // Text before the block belongs to the enclosing one.
    sink.flush(ctx);
    for &c in &node.children {
        walk(doc, c, junk, sink, kind, in_link);
    }
    sink.flush(kind);
}

/// Bylines out of the text, cruft before the story and after it removed,
/// repeated lines dropped.
fn tidy(blocks: &mut Vec<Block>, byline: &mut String, title: &str) {
    const BOILERPLATE: [&str; 22] = [
        "advertisement",
        "skip advertisement",
        "share",
        "share this article",
        "share this story",
        "read more",
        "related",
        "related topics",
        "related stories",
        "more on this story",
        "most read",
        "most viewed",
        "sign up",
        "subscribe",
        "image source",
        "image caption",
        "getty images",
        "listen to this article",
        "copy link",
        "comments",
        "follow us",
        "top stories",
    ];
    blocks.retain(|b| {
        let lower = b.text.trim_end_matches(['.', ':']).to_lowercase();
        !BOILERPLATE.contains(&lower.as_str()) && b.text != title
    });
    // "By Name" near the top is the byline.
    if let Some(at) = blocks.iter().take(6).position(|b| {
        b.kind != BlockKind::Heading && b.text.starts_with("By ") && b.text.len() <= 80
    }) {
        let b = blocks.remove(at);
        if byline.is_empty() {
            *byline = b.text[3..].trim().to_owned();
        }
    }
    // Short lines before the first real paragraph are labels and dates.
    if let Some(first) = blocks
        .iter()
        .position(|b| b.kind == BlockKind::Paragraph && b.text.len() >= 80)
    {
        let mut i = 0;
        blocks.retain(|b| {
            i += 1;
            i > first || b.text.len() >= 60
        });
    }
    blocks.dedup_by(|a, b| a.text == b.text);
    // A heading with nothing under it (its section was a promo).
    let mut i = 0;
    while i < blocks.len() {
        let next_is_heading = blocks
            .get(i + 1)
            .is_none_or(|b| b.kind == BlockKind::Heading);
        if blocks[i].kind == BlockKind::Heading && next_is_heading {
            blocks.remove(i);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests;

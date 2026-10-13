//! Podcast feeds as they come: RSS 2.0 with the iTunes namespace, and OPML
//! subscription lists. Feeds in the wild are often not well-formed XML (raw
//! HTML in descriptions, unclosed tags, unknown entities), so this reads
//! them with a small forgiving tokenizer instead of a validating parser.
use crate::podcasts::{episode_id, Episode, Show, MAX_EPISODES};

/// One element; `text` holds all text inside it, descendants included, in
/// document order.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.trim())
    }

    /// The trimmed text of the first child called `name` that has any.
    fn text_of(&self, name: &str) -> &str {
        self.children
            .iter()
            .filter(|c| c.name == name)
            .map(|c| c.text.trim())
            .find(|t| !t.is_empty())
            .unwrap_or("")
    }
}

enum Event {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        empty: bool,
    },
    End(String),
    Text(String),
}

/// Calls `f` for each tag and run of text. Names are lowercase; text and
/// attribute values have the XML entities decoded (CDATA is passed as is).
/// Comments, processing instructions and declarations are skipped; an
/// unterminated construct ends the document.
fn events(xml: &str, mut f: impl FnMut(Event)) {
    let mut rest = xml;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            f(Event::Text(decode(rest)));
            return;
        };
        if open > 0 {
            f(Event::Text(decode(&rest[..open])));
        }
        rest = &rest[open..];
        if let Some(body) = rest.strip_prefix("<![CDATA[") {
            let Some(end) = body.find("]]>") else {
                f(Event::Text(body.to_owned()));
                return;
            };
            f(Event::Text(body[..end].to_owned()));
            rest = &body[end + 3..];
            continue;
        }
        let skip_to = if rest.starts_with("<!--") {
            Some("-->")
        } else if rest.starts_with("<?") {
            Some("?>")
        } else if rest.starts_with("<!") {
            Some(">")
        } else {
            None
        };
        if let Some(end) = skip_to {
            match rest.find(end) {
                Some(at) => rest = &rest[at + end.len()..],
                None => return,
            }
            continue;
        }
        let Some(close) = rest.find('>') else {
            return;
        };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            f(Event::End(name.trim().to_ascii_lowercase()));
            continue;
        }
        let empty = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
        let name = tag[..name_end].to_ascii_lowercase();
        if name.is_empty() {
            // A "<" that starts no tag is text.
            f(Event::Text(decode(&format!("<{tag}>"))));
            continue;
        }
        f(Event::Start {
            name,
            attrs: attributes(&tag[name_end..]),
            empty,
        });
    }
}

/// `key="value"` pairs (single, double or no quotes); keys lowercase.
fn attributes(mut s: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    loop {
        s = s.trim_start();
        let Some(eq) = s.find('=') else {
            return out;
        };
        let key = s[..eq].trim().to_ascii_lowercase();
        let after = s[eq + 1..].trim_start();
        let (value, rest) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => match after[1..].find(q) {
                Some(end) => (&after[1..end + 1], &after[end + 2..]),
                None => (&after[1..], ""),
            },
            _ => {
                let end = after
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        if !key.is_empty() && !key.contains(char::is_whitespace) {
            out.push((key, decode(value)));
        }
        s = rest;
    }
}

/// The predefined XML entities and character references; anything else is
/// left as written (the description cleaner knows the HTML ones).
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let entity = after.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &after[..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code)
                }
            }?;
            Some((c, end))
        });
        match entity {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Elements that separate text (so descriptions written as raw HTML keep
/// their paragraphs).
const BLOCKS: [&str; 4] = ["p", "br", "div", "li"];
/// Deeper nesting than this is not podcast metadata; it is skipped.
const MAX_DEPTH: usize = 32;

/// Closes the innermost open element; it stays in its parent if `keep`
/// says so.
fn close(stack: &mut Vec<Node>, keep: &mut dyn FnMut(&Node) -> bool) {
    let Some(node) = stack.pop() else {
        return;
    };
    let Some(parent) = stack.last_mut().filter(|_| keep(&node)) else {
        return;
    };
    parent.text.push_str(&node.text);
    if BLOCKS.contains(&node.name.as_str()) {
        parent.text.push('\n');
    }
    parent.children.push(node);
}

/// Builds the element tree. An end tag closes everything opened after its
/// start tag; one without a start is ignored, and the end of the document
/// closes what is still open. `keep` sees each closed element and decides
/// whether it stays in its parent (feeds can hold thousands of items).
fn tree(xml: &str, mut keep: impl FnMut(&Node) -> bool) -> Node {
    let mut stack = vec![Node::default()];
    events(xml, |event| match event {
        Event::Start { name, attrs, empty } => {
            if stack.len() > MAX_DEPTH {
                return;
            }
            if BLOCKS.contains(&name.as_str()) {
                if let Some(top) = stack.last_mut() {
                    top.text.push('\n');
                }
            }
            stack.push(Node {
                name,
                attrs,
                ..Default::default()
            });
            if empty {
                close(&mut stack, &mut keep);
            }
        }
        Event::End(name) => {
            if let Some(at) = stack
                .iter()
                .rposition(|n| n.name == name)
                .filter(|at| *at > 0)
            {
                while stack.len() > at {
                    close(&mut stack, &mut keep);
                }
            }
        }
        Event::Text(text) => {
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&text);
            }
        }
    });
    while stack.len() > 1 {
        close(&mut stack, &mut keep);
    }
    stack.pop().unwrap_or_default()
}

/// An item as read, before the newest are kept and their text cleaned.
struct RawEpisode {
    episode: Episode,
    description: String,
}

/// Reads an RSS feed fetched from `feed`. Items without an audio enclosure
/// are left out; the newest `MAX_EPISODES` are kept, newest first.
pub fn parse_rss(feed: &str, xml: &str) -> Result<Show, String> {
    let mut raw: Vec<RawEpisode> = vec![];
    let root = tree(xml, |node| {
        if node.name != "item" {
            return true;
        }
        if let Some(r) = item(feed, node) {
            raw.push(r);
        }
        false
    });
    let channel = root
        .child("rss")
        .and_then(|rss| rss.child("channel"))
        .or_else(|| root.child("channel"))
        .ok_or("This address is not a podcast feed")?;
    raw.sort_by_key(|r| std::cmp::Reverse(r.episode.published));
    let mut seen = std::collections::HashSet::new();
    raw.retain(|r| seen.insert(r.episode.id.clone()));
    raw.truncate(MAX_EPISODES);
    let episodes = raw
        .into_iter()
        .map(|r| Episode {
            description: crate::podcast_text::clean(&r.description),
            ..r.episode
        })
        .collect();
    let image = channel
        .child("itunes:image")
        .and_then(|i| i.attr("href"))
        .filter(|h| !h.is_empty())
        .or_else(|| {
            channel
                .child("image")
                .map(|i| i.text_of("url"))
                .filter(|u| !u.is_empty())
        })
        .unwrap_or("");
    let author = ["itunes:author", "author", "managingeditor"]
        .iter()
        .map(|n| channel.text_of(n))
        .find(|a| !a.is_empty())
        .unwrap_or("");
    let about = ["description", "itunes:summary"]
        .iter()
        .map(|n| channel.text_of(n))
        .find(|a| !a.is_empty())
        .unwrap_or("");
    let title = one_line(channel.text_of("title"));
    Ok(Show {
        feed: feed.to_owned(),
        title: if title.is_empty() {
            feed.to_owned()
        } else {
            title
        },
        author: one_line(author),
        image: image.to_owned(),
        description: crate::podcast_text::clean(about),
        episodes,
        folder: String::new(),
    })
}

fn item(feed: &str, node: &Node) -> Option<RawEpisode> {
    let enclosure = node.child("enclosure")?;
    let url = enclosure.attr("url").filter(|u| !u.is_empty())?;
    let mime = enclosure.attr("type").unwrap_or("").to_ascii_lowercase();
    if !(mime.is_empty() || mime.starts_with("audio/") || mime.starts_with("video/")) {
        return None;
    }
    let guid = Some(node.text_of("guid"))
        .filter(|g| !g.is_empty())
        .unwrap_or(url);
    let title = one_line(node.text_of("title"));
    let description = ["content:encoded", "description", "itunes:summary"]
        .iter()
        .map(|n| node.text_of(n))
        .find(|d| !d.is_empty())
        .unwrap_or("")
        .to_owned();
    Some(RawEpisode {
        episode: Episode {
            id: episode_id(feed, guid),
            title: if title.is_empty() {
                "Untitled episode".into()
            } else {
                title
            },
            published: rfc822(node.text_of("pubdate")).unwrap_or(0),
            duration_s: duration(node.text_of("itunes:duration")),
            url: url.to_owned(),
            length: enclosure
                .attr("length")
                .and_then(|l| l.parse().ok())
                .unwrap_or(0),
            mime,
            description: String::new(),
        },
        description,
    })
}

/// Titles as one line of plain text.
fn one_line(text: &str) -> String {
    crate::podcast_text::clean(text)
        .split('\n')
        .collect::<Vec<_>>()
        .join(" ")
}

/// The feed addresses of an OPML subscription list (`xmlUrl` attributes).
pub fn opml_urls(xml: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    events(xml, |event| {
        if let Event::Start { name, attrs, .. } = event {
            let url = attrs.iter().find(|(k, _)| k == "xmlurl").map(|(_, v)| v);
            if let Some(url) = url.filter(|_| name == "outline") {
                let url = url.trim();
                if crate::podcasts::is_web_address(url) && !out.iter().any(|u| u == url) {
                    out.push(url.to_owned());
                }
            }
        }
    });
    out
}

/// `itunes:duration`: seconds, "MM:SS" or "HH:MM:SS"; 0 when unreadable.
fn duration(text: &str) -> u64 {
    let mut total: u64 = 0;
    for part in text.trim().split(':') {
        let Ok(value) = part.trim().parse::<f64>() else {
            return 0;
        };
        if !(0. ..1e7).contains(&value) {
            return 0;
        }
        total = total * 60 + value as u64;
    }
    total
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// An RSS date ("Tue, 07 Oct 2025 10:00:00 +0200", zone names, no weekday or
/// seconds) or, as some feeds write, ISO 8601; unix seconds.
pub fn rfc822(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() >= 10 && text.as_bytes()[4] == b'-' {
        return iso8601(text);
    }
    let text = text.split_once(',').map_or(text, |(_, rest)| rest);
    let mut parts = text.split_whitespace();
    let day: i64 = parts.next()?.parse().ok()?;
    let month = parts.next()?.get(..3)?.to_ascii_lowercase();
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| *m == month)? as i64
        + 1;
    let mut year: i64 = parts.next()?.parse().ok()?;
    if year < 100 {
        year += if year < 70 { 2000 } else { 1900 };
    }
    let mut clock = parts.next().unwrap_or("0:0").split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next().unwrap_or("0").parse().ok()?;
    let second: i64 = clock.next().unwrap_or("0").parse().unwrap_or(0);
    let offset = zone(parts.next().unwrap_or("GMT"));
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// A zone's offset from UTC in seconds ("+0200", "EST"); unknown is UTC.
fn zone(z: &str) -> i64 {
    if let Some(sign) = z.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits: String = z[1..].chars().filter(char::is_ascii_digit).collect();
        let n: i64 = digits.get(..4).and_then(|d| d.parse().ok()).unwrap_or(0);
        let seconds = (n / 100) * 3600 + (n % 100) * 60;
        return if sign == '-' { -seconds } else { seconds };
    }
    let hours = match z.to_ascii_uppercase().as_str() {
        "EDT" => -4,
        "EST" | "CDT" => -5,
        "CST" | "MDT" => -6,
        "MST" | "PDT" => -7,
        "PST" => -8,
        _ => 0,
    };
    hours * 3600
}

/// "2025-10-07T10:00:00Z", "2025-10-07T10:00:00+02:00" or a bare date.
fn iso8601(text: &str) -> Option<i64> {
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let day: i64 = text.get(8..10)?.parse().ok()?;
    let mut seconds = 0;
    let mut offset = 0;
    if let Some(time) = text.get(11..) {
        let hour: i64 = time.get(0..2)?.parse().ok()?;
        let minute: i64 = time.get(3..5)?.parse().ok()?;
        let second: i64 = time.get(6..8).and_then(|s| s.parse().ok()).unwrap_or(0);
        seconds = hour * 3600 + minute * 60 + second;
        if let Some(at) = time.rfind(['+', '-']).filter(|at| *at >= 5) {
            offset = zone(&time[at..]);
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + seconds - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = "https://feeds.example.org/garden.xml";

    /// A synthetic show shaped like common hosts' feeds: iTunes tags,
    /// CDATA HTML, an item without audio, one without a guid.
    fn garden() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"
     xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel>
  <title>The Example Garden</title>
  <itunes:author>Example Media</itunes:author>
  <itunes:image href="https://example.org/garden.jpg"/>
  <description><![CDATA[<p>Weekly notes from a <b>small</b> garden.</p>]]></description>
  <!-- a comment with <item> inside -->
  <item>
    <title>Episode 2: Tomatoes &amp; Basil</title>
    <guid isPermaLink="false">garden-2</guid>
    <pubDate>Tue, 07 Oct 2025 10:00:00 +0200</pubDate>
    <itunes:duration>1:02:03</itunes:duration>
    <enclosure url="https://media.example.org/garden-2.mp3?src=feed" length="41428232" type="audio/mpeg"/>
    <description>Short text.</description>
    <content:encoded><![CDATA[<p>Long text about tomatoes.</p><p>And basil.</p>]]></content:encoded>
  </item>
  <item>
    <title>A blog post without audio</title>
    <guid>post</guid>
    <pubDate>Wed, 08 Oct 2025 08:00:00 GMT</pubDate>
  </item>
  <item>
    <title>Episode 1: Soil</title>
    <pubDate>30 Sep 2025 06:30 EST</pubDate>
    <itunes:duration>754</itunes:duration>
    <enclosure url="https://media.example.org/garden-1.m4a" type="audio/x-m4a"/>
    <description>Plain &lt;i&gt;escaped&lt;/i&gt; HTML.</description>
  </item>
</channel>
</rss>"#
            .into()
    }

    #[test]
    fn rss_feeds_give_the_show_and_its_audio_episodes_newest_first() {
        let show = parse_rss(FEED, &garden()).unwrap();
        assert_eq!(show.title, "The Example Garden");
        assert_eq!(show.author, "Example Media");
        assert_eq!(show.image, "https://example.org/garden.jpg");
        assert_eq!(show.description, "Weekly notes from a small garden.");
        let titles: Vec<&str> = show.episodes.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Episode 2: Tomatoes & Basil", "Episode 1: Soil"]);
        let two = &show.episodes[0];
        assert_eq!(two.id, episode_id(FEED, "garden-2"));
        assert_eq!(two.published, 1_759_824_000);
        assert_eq!(two.duration_s, 3723);
        assert_eq!(two.length, 41_428_232);
        assert_eq!(two.mime, "audio/mpeg");
        assert_eq!(two.description, "Long text about tomatoes.\nAnd basil.");
        let one = &show.episodes[1];
        // Without a guid the audio address identifies the episode.
        assert_eq!(
            one.id,
            episode_id(FEED, "https://media.example.org/garden-1.m4a")
        );
        assert_eq!(one.published, 1_759_231_800);
        assert_eq!((one.duration_s, one.length), (754, 0));
        assert_eq!(one.description, "Plain escaped HTML.");
    }

    #[test]
    fn malformed_feeds_are_read_as_far_as_they_make_sense() {
        // Raw HTML in a description, an unclosed tag, an unknown entity,
        // a stray end tag and a document cut off in the middle.
        let xml = "<rss><channel><title>Broken &nbsp; Show</title></div>\
            <item><title>One</title><enclosure url='https://example.org/1.mp3' />\
            <description><p>First<br>second</p></description><guid>1</guid></item>\
            <item><title>Two</title><enclosure url=https://example.org/2.mp3>\
            <pubDate>not a date</pubDate></item><item><title>Thr";
        let show = parse_rss(FEED, xml).unwrap();
        assert_eq!(show.title, "Broken Show");
        let titles: Vec<&str> = show.episodes.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["One", "Two"]);
        assert_eq!(show.episodes[0].description, "First\nsecond");
        assert_eq!(show.episodes[1].url, "https://example.org/2.mp3");
        assert_eq!(show.episodes[1].published, 0);
        assert!(parse_rss(FEED, "<html><body>Not found</body></html>").is_err());
        assert!(parse_rss(FEED, "").is_err());
    }

    #[test]
    fn long_feeds_keep_only_the_newest_episodes() {
        let mut xml = String::from("<rss><channel><title>Daily</title>");
        for day in 1..=MAX_EPISODES + 20 {
            let (month, date) = (["Jan", "Feb", "Mar"][(day - 1) / 28], (day - 1) % 28 + 1);
            xml.push_str(&format!(
                "<item><title>Day {day}</title><guid>{day}</guid>\
                 <pubDate>{date:02} {month} 2025 00:00:00 GMT</pubDate>\
                 <enclosure url=\"https://example.org/{day}.mp3\" type=\"audio/mpeg\"/></item>"
            ));
        }
        // Repeated guids count once.
        xml.push_str("<item><guid>3</guid><enclosure url=\"https://example.org/x.mp3\"/></item>");
        let show = parse_rss(FEED, &xml).unwrap();
        assert_eq!(show.episodes.len(), MAX_EPISODES);
        assert_eq!(show.episodes[0].title, format!("Day {}", MAX_EPISODES + 20));
    }

    #[test]
    fn opml_lists_give_their_feed_addresses_once() {
        let xml = r#"<?xml version="1.0"?><opml version="2.0"><body>
            <outline text="Shows">
              <outline type="rss" text="Garden" xmlUrl="https://feeds.example.org/garden.xml"/>
              <outline type="rss" text="Again" xmlurl='https://feeds.example.org/garden.xml'/>
              <outline text="Kitchen" xmlUrl="http://example.org/kitchen.rss?a=1&amp;b=2"/>
              <outline text="Broken" xmlUrl="ftp://example.org/x"/>
            </outline></body></opml>"#;
        assert_eq!(
            opml_urls(xml),
            [
                "https://feeds.example.org/garden.xml",
                "http://example.org/kitchen.rss?a=1&b=2"
            ]
        );
    }

    #[test]
    fn dates_and_durations_in_their_common_spellings() {
        let base = rfc822("Tue, 07 Oct 2025 08:00:00 GMT").unwrap();
        assert_eq!(base, 1_759_824_000);
        assert_eq!(rfc822("07 Oct 2025 10:00:00 +0200"), Some(base));
        assert_eq!(rfc822("Tue, 7 October 2025 04:00 EDT"), Some(base));
        assert_eq!(rfc822("Tue, 07 Oct 25 01:00:00 PDT"), Some(base));
        assert_eq!(rfc822("2025-10-07T08:00:00Z"), Some(base));
        assert_eq!(rfc822("2025-10-07T10:00:00+02:00"), Some(base));
        assert_eq!(rfc822("2025-10-07"), Some(base - 8 * 3600));
        assert_eq!(rfc822("yesterday"), None);
        assert_eq!(duration("45"), 45);
        assert_eq!(duration("12:05"), 725);
        assert_eq!(duration(" 01:00:00 "), 3600);
        assert_eq!(duration("3723.6"), 3723);
        assert_eq!(duration("about an hour"), 0);
        assert_eq!(duration(""), 0);
    }
}

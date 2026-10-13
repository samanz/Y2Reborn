//! News headlines from RSS 2.0 and Atom feeds. A small, forgiving reader:
//! items' title, description, date and link, with CDATA, entities and HTML
//! handled.
use crate::https;
use reborn_core::news::Headline;

const MAX_ITEMS: usize = 40;
/// Feeds are text; anything larger is not a feed.
const MAX_FEED: usize = 512 * 1024;

pub fn fetch(url: &str) -> Result<Vec<Headline>, String> {
    let r = https::get(url, MAX_FEED)?;
    if r.status != 200 {
        return Err(format!("The feed answered {}", r.status));
    }
    let xml = String::from_utf8_lossy(&r.body);
    let items = parse(&xml);
    if items.is_empty() {
        return Err("No headlines in this feed".into());
    }
    Ok(items)
}

fn parse(xml: &str) -> Vec<Headline> {
    let mut out = vec![];
    let atom = !xml.contains("<item") && xml.contains("<entry");
    let (open, close) = if atom {
        ("<entry", "</entry>")
    } else {
        ("<item", "</item>")
    };
    let mut rest = xml;
    while let Some(start) = rest.find(open) {
        let after = &rest[start..];
        let Some(end) = after.find(close) else {
            break;
        };
        let item = &after[..end];
        rest = &after[end + close.len()..];
        let title = element(item, "title")
            .map(|t| plain(&t))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let headline = if atom {
            Headline {
                title,
                summary: element(item, "summary")
                    .or_else(|| element(item, "content"))
                    .map(|d| plain(&d))
                    .unwrap_or_default(),
                published: element(item, "published")
                    .or_else(|| element(item, "updated"))
                    .and_then(|d| iso8601(d.trim())),
                link: atom_link(item).unwrap_or_default(),
            }
        } else {
            Headline {
                title,
                summary: element(item, "description")
                    .map(|d| plain(&d))
                    .unwrap_or_default(),
                published: element(item, "pubDate").and_then(|d| rfc2822(d.trim())),
                link: element(item, "link")
                    .map(|l| l.trim().to_owned())
                    .filter(|l| l.starts_with("https://") || l.starts_with("http://"))
                    .unwrap_or_default(),
            }
        };
        out.push(headline);
        if out.len() == MAX_ITEMS {
            break;
        }
    }
    out
}

/// An Atom entry's web page: its `rel="alternate"` (or unnamed) link.
fn atom_link(entry: &str) -> Option<String> {
    let mut rest = entry;
    while let Some(at) = rest.find("<link") {
        let tag = &rest[at..at + rest[at..].find('>')?];
        rest = &rest[at + tag.len()..];
        let attr = |name: &str| {
            let key = format!(" {name}=\"");
            let start = tag.find(&key)? + key.len();
            Some(&tag[start..start + tag[start..].find('"')?])
        };
        if attr("rel").is_none_or(|r| r == "alternate") {
            return attr("href").map(decode);
        }
    }
    None
}

/// The text of the first `<name>` element (CDATA unwrapped, entities kept).
fn element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let mut from = 0;
    let start = loop {
        let at = from + xml[from..].find(&open)?;
        // `<title>` but not `<titlefoo>`.
        let next = xml[at + open.len()..].chars().next()?;
        if next == '>' || next.is_whitespace() {
            break at;
        }
        from = at + open.len();
    };
    let body_start = start + xml[start..].find('>')? + 1;
    if xml[..body_start].ends_with("/>") {
        return Some(String::new());
    }
    let close = format!("</{name}>");
    let body = &xml[body_start..body_start + xml[body_start..].find(&close)?];
    let body = body.trim();
    Some(match body.strip_prefix("<![CDATA[") {
        Some(inner) => inner.strip_suffix("]]>").unwrap_or(inner).to_owned(),
        None => decode(body),
    })
}

/// HTML (or escaped HTML) to plain text: tags removed, entities decoded,
/// whitespace collapsed.
fn plain(text: &str) -> String {
    let decoded = decode(text);
    let mut out = String::with_capacity(decoded.len());
    let mut in_tag = false;
    for ch in decoded.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Entities to characters (named ones common in news pages, and numeric).
pub(crate) fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        // Bytes, not a str slice: the tenth byte may fall inside a character.
        let Some(semi) = tail.bytes().take(10).position(|b| b == b';') else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..semi];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "rsquo" | "lsquo" => Some('\''),
            "rdquo" | "ldquo" => Some('"'),
            "pound" => Some('£'),
            "euro" => Some('€'),
            "copy" => Some('©'),
            "deg" => Some('°'),
            "eacute" => Some('é'),
            "egrave" => Some('è'),
            "aacute" => Some('á'),
            "iacute" => Some('í'),
            "oacute" => Some('ó'),
            "uacute" => Some('ú'),
            "ntilde" => Some('ñ'),
            "ccedil" => Some('ç'),
            "uuml" => Some('ü'),
            "ouml" => Some('ö'),
            "auml" => Some('ä'),
            "bull" | "middot" => Some('·'),
            "thinsp" | "ensp" | "emsp" => Some(' '),
            "shy" | "zwj" | "zwnj" => Some('\u{AD}'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// "Sat, 11 Oct 2026 08:15:00 GMT" (or "+0100") to Unix seconds.
fn rfc2822(date: &str) -> Option<i64> {
    let date = date.split_once(", ").map_or(date, |(_, d)| d);
    let mut parts = date.split_whitespace();
    let day: i64 = parts.next()?.parse().ok()?;
    let name = parts.next()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == name)? as i64
        + 1;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':').map(|p| p.parse::<i64>().ok());
    let (h, m, s) = (
        hms.next()??,
        hms.next()??,
        hms.next().flatten().unwrap_or(0),
    );
    let offset = match parts.next().unwrap_or("GMT") {
        "GMT" | "UT" | "UTC" | "Z" => 0,
        "EST" => -5 * 3600,
        "EDT" => -4 * 3600,
        "CST" => -6 * 3600,
        "CDT" => -5 * 3600,
        "PST" => -8 * 3600,
        "PDT" => -7 * 3600,
        z if z.len() == 5 && (z.starts_with('+') || z.starts_with('-')) => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let hh: i64 = z[1..3].parse().ok()?;
            let mm: i64 = z[3..5].parse().ok()?;
            sign * (hh * 3600 + mm * 60)
        }
        _ => 0,
    };
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s - offset)
}

/// "2026-10-12T21:20:57-04:00", "...57.042Z" or "...57Z" to Unix seconds.
fn iso8601(date: &str) -> Option<i64> {
    let (day, time) = date.split_once('T')?;
    let mut ymd = day.split('-').map(|p| p.parse::<i64>().ok());
    let (year, month, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let zone_at = time.find(['Z', '+', '-']).unwrap_or(time.len());
    let (clock, zone) = time.split_at(zone_at);
    let mut hms = clock.split(':');
    let h: i64 = hms.next()?.parse().ok()?;
    let m: i64 = hms.next()?.parse().ok()?;
    let s: i64 = hms
        .next()
        .and_then(|s| s.split('.').next()?.parse().ok())
        .unwrap_or(0);
    let offset = match zone {
        "" | "Z" => 0,
        z => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (hh, mm) = z[1..].split_once(':').unwrap_or((&z[1..], "0"));
            sign * (hh.parse::<i64>().ok()? * 3600 + mm.parse::<i64>().ok()? * 60)
        }
    };
    Some(days_from_civil(year, month, d) * 86_400 + h * 3600 + m * 60 + s - offset)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0"?><rss><channel><title>Feed</title>
<item><title><![CDATA[First & foremost]]></title>
<description><![CDATA[<p>Some <b>bold</b> news.</p>]]></description>
<link>https://example.com/news/first?a=1&amp;b=2</link>
<pubDate>Sun, 11 Oct 2026 08:15:00 GMT</pubDate></item>
<item><title>Caf&#233; &amp; &quot;bar&quot;</title><description>&lt;a href="x"&gt;Link&lt;/a&gt; text</description>
<pubDate>Sun, 11 Oct 2026 09:15:00 +0100</pubDate></item>
<item><title></title></item>
</channel></rss>"#;

    #[test]
    fn reads_items_with_cdata_entities_and_html() {
        let items = parse(FEED);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First & foremost");
        assert_eq!(items[0].summary, "Some bold news.");
        assert_eq!(items[1].title, "Café & \"bar\"");
        assert_eq!(items[1].summary, "Link text");
        assert_eq!(items[0].link, "https://example.com/news/first?a=1&b=2");
        assert_eq!(items[1].link, "");
        // Both are 08:15 UTC.
        assert_eq!(items[0].published, items[1].published);
    }

    #[test]
    fn dates() {
        assert_eq!(rfc2822("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            rfc2822("Sun, 11 Oct 2026 08:15:00 GMT"),
            Some(1_791_706_500)
        );
        assert_eq!(
            rfc2822("11 Oct 2026 08:15 -0400"),
            Some(1_791_706_500 + 4 * 3600)
        );
        assert_eq!(rfc2822("yesterday"), None);
    }

    #[test]
    fn an_ampersand_before_wide_characters_does_not_panic() {
        assert_eq!(decode("R&D \u{2009}é€"), "R&D \u{2009}é€");
        assert_eq!(decode("&amp;é"), "&é");
    }

    #[test]
    fn reads_atom_entries() {
        let feed = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom">
<title>The Site</title><link rel="self" href="https://example.com/feed"/>
<entry><title type="html">Robots &amp; rockets</title>
<link rel="alternate" type="text/html" href="https://example.com/a/1?x=1&amp;y=2"/>
<published>2026-10-11T04:15:00-04:00</published>
<summary type="html">&lt;p&gt;Short &lt;b&gt;summary&lt;/b&gt;&lt;/p&gt;</summary></entry>
<entry><title>Second</title><link href="https://example.com/a/2"/>
<updated>2026-10-11T08:15:00.042Z</updated><content>Body</content></entry>
</feed>"#;
        let items = parse(feed);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Robots & rockets");
        assert_eq!(items[0].link, "https://example.com/a/1?x=1&y=2");
        assert_eq!(items[0].summary, "Short summary");
        assert_eq!(items[0].published, Some(1_791_706_500));
        assert_eq!(items[1].link, "https://example.com/a/2");
        assert_eq!(items[1].published, Some(1_791_706_500));
        assert_eq!(items[1].summary, "Body");
    }

    #[test]
    fn channel_title_is_not_an_item() {
        assert!(parse("<rss><channel><title>x</title></channel></rss>").is_empty());
    }
}

//! Podcast descriptions as clean text: feeds send HTML (paragraphs, line
//! breaks, lists, links, entities) or plain text with stray blank lines.
//! Reborn shows them as paragraphs of plain text, in characters its fonts
//! have, without the sponsor boilerplate many hosts append.

/// Descriptions are kept to about this many characters.
pub const MAX_CHARS: usize = 6000;
/// A wheel step through a description moves one piece of about this many
/// characters (three or four lines on the screen).
pub const CHUNK_CHARS: usize = 220;

/// Paragraphs that are only hosting boilerplate.
const BOILERPLATE: [&str; 6] = [
    "adchoices",
    "podcastchoices.com",
    "omnystudio.com/listener",
    "privacy policy",
    "megaphone.fm/adchoices",
    "learn more about your ad choices",
];

/// `raw` (HTML or plain text) as plain paragraphs separated by `\n`.
pub fn clean(raw: &str) -> String {
    let text = strip_tags(raw);
    let text = decode_entities(&text);
    let mut paragraphs: Vec<String> = vec![];
    let mut total = 0;
    for line in text.split('\n') {
        let line: String = printable(line)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() || is_boilerplate(&line) {
            continue;
        }
        if total + line.len() > MAX_CHARS {
            let room = MAX_CHARS.saturating_sub(total);
            if room > 40 {
                let cut = cut_at_word(&line, room);
                paragraphs.push(format!("{cut}…"));
            }
            break;
        }
        total += line.len() + 1;
        paragraphs.push(line);
    }
    paragraphs.join("\n")
}

fn is_boilerplate(line: &str) -> bool {
    let lower = line.to_lowercase();
    BOILERPLATE.iter().any(|b| lower.contains(b)) && line.len() < 400
}

/// Tags become line breaks where they end a block (paragraphs, breaks, list
/// items, headings); list items get a leading dash; script and style
/// contents go.
fn strip_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let starts_tag = after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let close = after.find('>').filter(|_| starts_tag);
        let Some(close) = close.filter(|close| !after[..*close].contains('<')) else {
            // A "<" that starts no tag ("3 < 4") is text.
            out.push('<');
            rest = after;
            continue;
        };
        let tag = after[..close].trim();
        rest = &after[close + 1..];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let closing = tag.starts_with('/');
        if !closing && (name == "script" || name == "style") {
            let end = format!("</{name}");
            let lower = rest.to_ascii_lowercase();
            rest = match lower.find(&end) {
                Some(at) => rest[at..].split_once('>').map_or("", |(_, r)| r),
                None => "",
            };
            continue;
        }
        match name.as_str() {
            "br" | "p" | "div" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "blockquote" | "tr" | "hr" => out.push('\n'),
            "li" if !closing => out.push_str("\n- "),
            "li" => out.push('\n'),
            _ => {}
        }
    }
    out.push_str(rest);
    out
}

/// Named and numeric character references.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let end = after
            .char_indices()
            .take(10)
            .find(|(_, c)| *c == ';')
            .map(|(i, _)| i);
        let decoded = end.and_then(|end| entity(&after[..end]).map(|c| (c, end)));
        match decoded {
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

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "rsquo" | "lsquo" => '\'',
        "rdquo" | "ldquo" => '"',
        "mdash" | "ndash" | "minus" => '-',
        "hellip" => '…',
        "middot" | "bull" => '·',
        "copy" => '©',
        "eacute" => 'é',
        "egrave" => 'è',
        "aacute" => 'á',
        "oacute" => 'ó',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        _ => return None,
    })
}

/// Characters the player's fonts draw: typographic quotes and dashes as
/// their plain forms, invisible and emoji characters left out.
fn printable(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => out.push('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => out.push('"'),
            '\u{2010}'..='\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{2022}' | '\u{25CF}' | '\u{25AA}' => out.push('·'),
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\t' => out.push(' '),
            '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FE0F}' | '\u{FEFF}' => {}
            '\u{2600}'..='\u{27BF}' | '\u{1F000}'..='\u{1FAFF}' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The longest start of `text` of at most `max` bytes that ends at a word.
fn cut_at_word(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    match head.rfind(' ') {
        Some(space) if space > max / 2 => head[..space].trim_end(),
        _ => head,
    }
}

/// A paragraph in pieces of at most about `CHUNK_CHARS`, split after a
/// sentence when one ends late enough, else at a word.
pub fn chunks(paragraph: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = paragraph.trim();
    while rest.chars().count() > CHUNK_CHARS {
        let limit = rest
            .char_indices()
            .nth(CHUNK_CHARS)
            .map_or(rest.len(), |(i, _)| i);
        let head = &rest[..limit];
        let sentence = head
            .match_indices(". ")
            .chain(head.match_indices("? "))
            .chain(head.match_indices("! "))
            .map(|(i, _)| i + 1)
            .filter(|i| *i > limit / 2)
            .max();
        let word = head.rfind(' ').filter(|s| *s > limit / 2).unwrap_or(limit);
        let split = sentence.unwrap_or(word);
        out.push(rest[..split].trim().to_owned());
        rest = rest[split..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_paragraphs_of_plain_text() {
        let raw = "<p>Hello &amp; welcome to <b>the show</b>.</p><p>Second&nbsp;para<br/>line \
                   two</p><ul><li>One</li><li>Two &#8212; &#x2019;three&rsquo;</li></ul>\
                   <script>var x = 1;</script><style>p{}</style><p>After</p>";
        assert_eq!(
            clean(raw),
            "Hello & welcome to the show.\nSecond para\nline two\n- One\n- Two - 'three'\nAfter"
        );
    }

    #[test]
    fn plain_text_keeps_its_paragraphs_and_drops_blank_lines() {
        let raw = "First paragraph.\n \nSecond   one\twith  spaces.\n\n\n\u{201C}Quoted\u{201D} \
                   \u{2014} done \u{1F3A7}\u{200B}";
        assert_eq!(
            clean(raw),
            "First paragraph.\nSecond one with spaces.\n\"Quoted\" - done"
        );
    }

    #[test]
    fn boilerplate_goes_and_odd_markup_is_tolerated() {
        let raw = "Real text 3 < 4 and AT&T.\nLearn more about your ad choices. Visit \
                   megaphone.fm/adchoices\n<a href=\"x\">link</a> &bogus; &#99999999;";
        assert_eq!(
            clean(raw),
            "Real text 3 < 4 and AT&T.\nlink &bogus; &#99999999;"
        );
        assert_eq!(clean(""), "");
        assert_eq!(clean("<p></p><br>"), "");
        assert_eq!(clean("unclosed <b"), "unclosed <b");
    }

    #[test]
    fn long_descriptions_are_cut_at_a_word() {
        let raw = "word ".repeat(2000);
        let text = clean(&raw);
        assert!(text.len() <= MAX_CHARS + 4, "{}", text.len());
        assert!(text.ends_with("word…"));
    }

    #[test]
    fn paragraphs_split_into_wheel_steps_at_sentences() {
        let para = "This is a sentence that goes on for a while. ".repeat(12);
        let pieces = chunks(&para);
        assert!(pieces.len() >= 3, "{pieces:?}");
        assert!(pieces.iter().all(|p| p.chars().count() <= CHUNK_CHARS));
        assert!(pieces[..pieces.len() - 1].iter().all(|p| p.ends_with('.')));
        let words: usize = pieces.iter().map(|p| p.split_whitespace().count()).sum();
        assert_eq!(words, para.split_whitespace().count());
        // No sentence ends late enough: a word boundary.
        let run_on = "longword ".repeat(60);
        assert!(chunks(&run_on)
            .iter()
            .all(|p| !p.ends_with(' ') && !p.is_empty()));
        assert_eq!(chunks("Short."), ["Short."]);
        assert!(chunks("  ").is_empty());
        // Accented text never splits inside a character.
        assert!(!chunks(&"é".repeat(500)).is_empty());
    }
}

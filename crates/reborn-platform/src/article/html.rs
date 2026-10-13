//! A small, forgiving HTML tree builder for story pages: enough of the
//! HTML parsing rules to find paragraphs and their containers (void and
//! raw-text elements, comments, implied `</p>` and `</li>`, stray end tags).
//! Text and attribute values borrow from the page; only the attributes the
//! extractor looks at are kept.

/// One element or text run. Text nodes have an empty `tag`.
#[derive(Debug)]
pub struct Node<'a> {
    pub tag: String,
    pub attrs: Vec<(&'a str, &'a str)>,
    pub parent: usize,
    pub children: Vec<usize>,
    /// The raw text of a text node (entities still encoded).
    pub text: &'a str,
}

impl Node<'_> {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }

    pub fn is_text(&self) -> bool {
        self.tag.is_empty()
    }

    /// Whether the class list holds `token` exactly.
    pub fn has_class(&self, token: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_ascii_whitespace().any(|t| t == token))
    }
}

pub struct Doc<'a> {
    /// Node 0 is the document itself.
    pub nodes: Vec<Node<'a>>,
}

/// Attributes worth keeping (the rest are skipped while parsing).
const KEEP: [&str; 10] = [
    "id",
    "class",
    "itemprop",
    "role",
    "aria-hidden",
    "hidden",
    "type",
    "property",
    "name",
    "content",
];

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const RAW_TEXT: [&str; 6] = [
    "script", "style", "textarea", "title", "noscript", "template",
];

/// Elements whose start closes an open `<p>`.
const CLOSES_P: [&str; 24] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "div",
    "dl",
    "fieldset",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
];

/// Deeper nesting than this is flattened (keeps the walkers' recursion
/// bounded on hostile pages).
const MAX_DEPTH: usize = 200;

pub fn parse(html: &str) -> Doc<'_> {
    let mut doc = Doc {
        nodes: vec![Node {
            tag: "#document".into(),
            attrs: vec![],
            parent: 0,
            children: vec![],
            text: "",
        }],
    };
    let mut stack: Vec<usize> = vec![0];
    let bytes = html.as_bytes();
    let mut pos = 0;
    while pos < html.len() {
        let Some(lt) = html[pos..].find('<').map(|i| pos + i) else {
            doc.text(&stack, &html[pos..]);
            break;
        };
        if lt > pos {
            doc.text(&stack, &html[pos..lt]);
        }
        let rest = &html[lt..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            pos = comment.find("-->").map_or(html.len(), |i| lt + 4 + i + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            pos = rest.find('>').map_or(html.len(), |i| lt + i + 1);
            continue;
        }
        if let Some(after) = rest.strip_prefix("</") {
            let name_len = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-')
                .count();
            let name = after[..name_len].to_ascii_lowercase();
            pos = rest.find('>').map_or(html.len(), |i| lt + i + 1);
            if let Some(at) = stack.iter().rposition(|&n| doc.nodes[n].tag == name) {
                if at > 0 {
                    stack.truncate(at);
                }
            }
            continue;
        }
        let name_len = rest[1..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b':')
            .count();
        if name_len == 0 || !bytes[lt + 1].is_ascii_alphabetic() {
            // A bare "<" in text.
            doc.text(&stack, "<");
            pos = lt + 1;
            continue;
        }
        let tag = rest[1..1 + name_len].to_ascii_lowercase();
        let (attrs, end, self_closing) = attributes(html, lt + 1 + name_len);
        pos = end;
        // Implied end tags.
        if CLOSES_P.contains(&tag.as_str()) {
            if let Some(at) = stack.iter().rposition(|&n| doc.nodes[n].tag == "p") {
                // Only when the <p> is the current block (not across a div).
                if stack[at + 1..]
                    .iter()
                    .all(|&n| !CLOSES_P.contains(&doc.nodes[n].tag.as_str()))
                {
                    stack.truncate(at);
                }
            }
        }
        if tag == "li" {
            let list = stack
                .iter()
                .rposition(|&n| matches!(doc.nodes[n].tag.as_str(), "ul" | "ol"));
            if let Some(at) = stack.iter().rposition(|&n| doc.nodes[n].tag == "li") {
                if list.is_none_or(|l| at > l) {
                    stack.truncate(at);
                }
            }
        }
        let parent = *stack.last().unwrap_or(&0);
        let id = doc.nodes.len();
        let raw = RAW_TEXT.contains(&tag.as_str());
        let void = VOID.contains(&tag.as_str()) || self_closing && !raw;
        doc.nodes.push(Node {
            tag,
            attrs,
            parent,
            children: vec![],
            text: "",
        });
        doc.nodes[parent].children.push(id);
        if raw && !self_closing {
            // Everything up to the matching end tag is text.
            let close = format!("</{}", doc.nodes[id].tag);
            let end = find_ascii_ci(&html[pos..], &close).map_or(html.len(), |i| pos + i);
            if end > pos {
                let tid = doc.nodes.len();
                doc.nodes.push(Node {
                    tag: String::new(),
                    attrs: vec![],
                    parent: id,
                    children: vec![],
                    text: &html[pos..end],
                });
                doc.nodes[id].children.push(tid);
            }
            pos = html[end..].find('>').map_or(html.len(), |i| end + i + 1);
        } else if !void && stack.len() < MAX_DEPTH {
            stack.push(id);
        }
    }
    doc
}

impl<'a> Doc<'a> {
    fn text(&mut self, stack: &[usize], text: &'a str) {
        let parent = *stack.last().unwrap_or(&0);
        let id = self.nodes.len();
        self.nodes.push(Node {
            tag: String::new(),
            attrs: vec![],
            parent,
            children: vec![],
            text,
        });
        self.nodes[parent].children.push(id);
    }

    /// Ancestors of `id`, nearest first (excluding `id` and the document).
    pub fn ancestors(&self, id: usize) -> impl Iterator<Item = usize> + '_ {
        let mut at = id;
        std::iter::from_fn(move || {
            let p = self.nodes[at].parent;
            if p == at || p == 0 {
                return None;
            }
            at = p;
            Some(p)
        })
    }

    /// Elements named `tag`, in document order.
    pub fn elements<'s>(&'s self, tag: &'s str) -> impl Iterator<Item = usize> + 's {
        (0..self.nodes.len()).filter(move |&i| self.nodes[i].tag == tag)
    }

    /// The concatenated raw text under `id`.
    pub fn raw_text(&self, id: usize) -> String {
        let mut out = String::new();
        self.push_raw(id, &mut out);
        out
    }

    fn push_raw(&self, id: usize, out: &mut String) {
        let n = &self.nodes[id];
        if n.is_text() {
            out.push_str(n.text);
        }
        for &c in &n.children {
            self.push_raw(c, out);
        }
    }
}

/// Parse attributes from `from` (just after the tag name) to the closing
/// `>`. Returns the kept attributes, the position after `>`, and whether
/// the tag closed itself (`/>`).
fn attributes(html: &str, from: usize) -> (Vec<(&str, &str)>, usize, bool) {
    let b = html.as_bytes();
    let mut i = from;
    let mut attrs = vec![];
    let mut self_closing = false;
    while i < b.len() {
        match b[i] {
            b'>' => return (attrs, i + 1, self_closing),
            b'/' => {
                self_closing = true;
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                self_closing = false;
                let start = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>') {
                    // "/" ends a name only as part of "/>".
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'>') {
                        break;
                    }
                    i += 1;
                }
                let name = &html[start..i];
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                let mut value = "";
                if i < b.len() && b[i] == b'=' {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                        let q = b[i];
                        let vs = i + 1;
                        let ve = b[vs..]
                            .iter()
                            .position(|&c| c == q)
                            .map_or(b.len(), |p| vs + p);
                        value = &html[vs..ve];
                        i = (ve + 1).min(b.len());
                    } else {
                        let vs = i;
                        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                            i += 1;
                        }
                        value = &html[vs..i];
                    }
                }
                if let Some(keep) = KEEP.iter().find(|k| k.eq_ignore_ascii_case(name)) {
                    attrs.push((*keep, value));
                }
            }
        }
    }
    (attrs, b.len(), self_closing)
}

fn find_ascii_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(doc: &Doc, id: usize) -> Vec<String> {
        doc.nodes[id]
            .children
            .iter()
            .map(|&c| {
                let n = &doc.nodes[c];
                if n.is_text() {
                    format!("'{}'", n.text)
                } else {
                    n.tag.clone()
                }
            })
            .collect()
    }

    #[test]
    fn builds_a_tree_with_void_raw_and_implied_ends() {
        let doc = parse(
            "<!DOCTYPE html><html><head><script>if (a < b) { x('</p>') }</script>\
             <meta name=author content='A B'></head><body class=\"x  y\">\
             <p>One<br>two<p>Three <!-- note --><b>bold</p><ul><li>a<li>b</ul>\
             <div>stray</span> end</div><img src=x /></body></html>",
        );
        let body = doc.elements("body").next().unwrap();
        assert!(doc.nodes[body].has_class("y"));
        assert_eq!(tags(&doc, body), ["p", "p", "ul", "div", "img"]);
        let ps: Vec<_> = doc.elements("p").collect();
        assert_eq!(doc.raw_text(ps[0]), "Onetwo");
        assert_eq!(doc.raw_text(ps[1]), "Three bold");
        let ul = doc.elements("ul").next().unwrap();
        assert_eq!(tags(&doc, ul), ["li", "li"]);
        let script = doc.elements("script").next().unwrap();
        assert_eq!(doc.raw_text(script), "if (a < b) { x('</p>') }");
        let meta = doc.elements("meta").next().unwrap();
        assert_eq!(doc.nodes[meta].attr("content"), Some("A B"));
        assert_eq!(
            doc.raw_text(doc.elements("div").next().unwrap()),
            "stray end"
        );
    }

    #[test]
    fn a_p_inside_a_div_survives_the_divs_children() {
        let doc = parse("<p>Lead <div>inner</div> tail");
        // The div closes the paragraph, so it becomes the p's sibling.
        assert_eq!(tags(&doc, 0), ["p", "div", "' tail'"]);
    }

    #[test]
    fn hostile_input_stays_bounded() {
        let deep = "<div>".repeat(10_000) + "x";
        let doc = parse(&deep);
        assert!(doc.nodes.len() > 10_000);
        assert!(doc.ancestors(doc.nodes.len() - 1).count() < MAX_DEPTH + 1);
        // Unterminated constructs end the parse rather than panicking.
        for s in [
            "<",
            "<a",
            "<a href='x",
            "<!--",
            "</",
            "<script>x",
            "a < b",
            "<p/>",
        ] {
            parse(s);
        }
    }
}

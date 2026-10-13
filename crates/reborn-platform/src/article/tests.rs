use super::*;

// Small synthetic pages (invented text) in the shapes story pages take.
/// The story in an <article> (a routed site).
const ARTICLE_A: &str = include_str!("fixtures/article-a.html");
/// A text-only page with a byline paragraph (a routed site).
const ARTICLE_B: &str = include_str!("fixtures/article-b.html");
/// A blog post with comments and a sidebar (no route).
const ARTICLE_C: &str = include_str!("fixtures/article-c.html");
/// A story split across sibling containers (no route).
const ARTICLE_D: &str = include_str!("fixtures/article-d.html");
/// A subscriber-only teaser.
const ARTICLE_E: &str = include_str!("fixtures/article-e.html");

fn kinds(a: &Article) -> Vec<(BlockKind, &str)> {
    a.blocks.iter().map(|b| (b.kind, b.text.as_str())).collect()
}

fn all_text(a: &Article) -> String {
    a.blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// No navigation, promos, ads, captions, footers or comments in the text.
fn assert_clean(a: &Article) {
    let text = all_text(a).to_lowercase();
    for junk in [
        "advertisement",
        "copyright",
        "image caption",
        "related",
        "promotes",
        "great post",
        "subscribe",
        "more on this story",
        "not text",
    ] {
        assert!(!text.contains(junk), "{junk:?} in {text}");
    }
}

#[test]
fn a_routed_article_keeps_paragraphs_subheadings_and_lists() {
    let a = extract(
        ARTICLE_A,
        "https://www.bbc.co.uk/news/articles/x?at_medium=RSS",
    )
    .unwrap();
    assert_eq!(a.title, "Town library reopens after a long repair");
    assert_eq!(a.byline, "Riley Writer");
    assert!(!a.paywalled);
    let k = kinds(&a);
    assert_eq!(k.len(), 7, "{k:#?}");
    assert!(k[0].1.starts_with("The town library will reopen"));
    assert_eq!(k[3], (BlockKind::Heading, "Opening hours"));
    assert_eq!(
        k[5],
        (BlockKind::Item, "Children's corner open from Monday")
    );
    assert_clean(&a);
}

#[test]
fn a_text_only_page_with_its_byline() {
    let a = extract(ARTICLE_B, "https://text.npr.org/nx-s1-1").unwrap();
    assert_eq!(a.byline, "Jordan Sample");
    assert_eq!(a.blocks.len(), 4);
    assert!(a.blocks.iter().all(|b| b.kind == BlockKind::Paragraph));
    assert!(a.blocks[2].text.starts_with("\"It felt like"));
    assert!(!all_text(&a).contains("EDT"));
    assert_clean(&a);
}

#[test]
fn npr_links_go_to_the_text_only_site() {
    assert_eq!(
        page_url("https://www.npr.org/2026/10/13/nx-s1-5998948/a-story-slug"),
        "https://text.npr.org/nx-s1-5998948"
    );
    assert_eq!(
        page_url("https://www.npr.org/sections/science/2026/10/13/g-s1-1234/a-story"),
        "https://text.npr.org/g-s1-1234"
    );
    assert_eq!(page_url("http://example.com/a"), "https://example.com/a");
    // Not a story link, and other sites, are fetched as given.
    assert_eq!(
        page_url("https://www.npr.org/sections/news/"),
        "https://www.npr.org/sections/news/"
    );
    assert_eq!(
        page_url("https://example.com/2026/10/13/x/y"),
        "https://example.com/2026/10/13/x/y"
    );
}

#[test]
fn unknown_sites_use_the_densest_paragraphs() {
    let a = extract(ARTICLE_C, "https://blog.example.net/2026/kite").unwrap();
    assert_eq!(a.byline, "Casey Maker");
    let k = kinds(&a);
    assert_eq!(k[0].0, BlockKind::Paragraph);
    assert!(k.contains(&(BlockKind::Heading, "Materials")));
    assert!(k.contains(&(BlockKind::Item, "Two bamboo garden canes")));
    assert!(k.contains(&(BlockKind::Quote, "Wind always finds the weak seam.")));
    assert!(k.iter().any(|(_, t)| t.starts_with("ratio:")));
    assert_eq!(
        k.last().unwrap().1,
        "If you try this, fly it on a beach. Trees win every time."
    );
    // The title, date, menu, comments and sidebar are left out.
    let text = all_text(&a);
    assert!(!k.iter().any(|(_, t)| *t == a.title));
    assert!(!text.contains("October 13") && !text.contains("shower curtain"));
    assert!(!text.contains("mailing list") && !text.contains("bird box"));
    assert_clean(&a);
}

#[test]
fn a_story_split_across_containers_is_read_whole() {
    let a = extract(ARTICLE_D, "https://weekly.example.com/garden/pumpkin").unwrap();
    assert_eq!(a.byline, "Alex Grower");
    assert!(!a.paywalled);
    assert_eq!(a.blocks.len(), 3, "{:#?}", kinds(&a));
    assert!(a.blocks[2].text.contains("wheelbarrow"));
    // The standfirst sits above the story.
    assert!(!all_text(&a).contains("head teacher"));
    assert_clean(&a);
}

#[test]
fn a_subscriber_story_keeps_only_the_text_everyone_sees() {
    let free: String = (1..=4)
        .map(|i| {
            format!(
                "<p>Paragraph {i} is one that everyone can read, long enough to count as a \
                 real paragraph of a story.</p>"
            )
        })
        .collect();
    let page = format!(
        "<html><head><script type=\"application/ld+json\">{}</script></head>\
         <body><article>{free}</article></body></html>",
        serde_json::json!({"@type": "NewsArticle", "isAccessibleForFree": false,
            "author": {"@type": "Person", "name": "Sky Columnist"},
            "articleBody": "Hidden text not shown to readers."})
    );
    let a = extract(&page, "https://paper.example.com/column").unwrap();
    assert!(a.paywalled);
    assert_eq!(a.byline, "Sky Columnist");
    // The hidden body is never used.
    assert_eq!(a.blocks.len(), 4);
    assert!(!all_text(&a).contains("Hidden"));
}

#[test]
fn paywalls_and_empty_pages_are_reported() {
    assert_eq!(
        extract(ARTICLE_E, "https://paper.example.com/x").unwrap_err(),
        "This story is for subscribers"
    );
    assert_eq!(
        extract("<html><body><p>Video</p></body></html>", "https://e.com/v").unwrap_err(),
        "Couldn't find the story on this page"
    );
    assert!(extract("", "").is_err());
}

#[test]
fn structured_data_body_when_the_page_has_no_paragraphs() {
    let body = "First paragraph of a story told only in structured data, long enough to count.\n\
                Second paragraph, which goes on a little longer so the whole is a real story.\n\
                Third paragraph closes it with a final sentence or two about what comes next.\n\
                Fourth paragraph: some sites really do this, and readers deserve the text.";
    let page = format!(
        "<html><head><script type=\"application/ld+json\">{}</script></head><body><div id=app></div></body></html>",
        serde_json::json!({"@type": "NewsArticle", "articleBody": body, "author": "Ann Author"})
    );
    let a = extract(&page, "https://e.com/x").unwrap();
    assert_eq!(a.blocks.len(), 4);
    assert_eq!(a.byline, "Ann Author");
}

#[test]
fn text_is_cleaned_for_the_players_fonts() {
    assert_eq!(
        clean("  a&nbsp;b \u{2009}c\u{AD}d \u{2212}5 &amp;&#8212; "),
        "a b cd -5 &—"
    );
    assert_eq!(
        join_names(&["A".into(), "B".into(), "C".into()]),
        "A, B and C"
    );
}

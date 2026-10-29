use crate::{safe_link, Block, HtmlDocument, Inline, ParseOptions};

#[test]
fn hn_omitted_paragraph_end_tags_and_entities() {
    let doc = HtmlDocument::parse(
        "First &amp; foremost<p>Second &#x1f980; <i>emphasis</i><p>Third&nbsp;part &lt;T&gt;",
    );
    assert_eq!(doc.blocks.len(), 3);
    assert_eq!(
        doc.plain_text(),
        "First & foremost\nSecond 🦀 emphasis\nThird\u{a0}part <T>"
    );
    assert!(!doc.truncated);
}

#[test]
fn malformed_formatting_is_repaired_by_html5_parser() {
    let doc = HtmlDocument::parse("<p>one <b>two <i>three</b> four</i><p>five");
    assert_eq!(doc.plain_text(), "one two three four\nfive");
    assert_eq!(doc.blocks.len(), 2);
}

#[test]
fn active_subtrees_are_removed_and_images_are_inert() {
    let doc = HtmlDocument::parse(concat!(
        "<p>safe<script>alert('bad')</script><style>body{display:none}</style>",
        "<iframe src='https://evil.example'>frame</iframe><object>object</object>",
        "<form><input value='steal'>form</form><svg><text>svg</text></svg>",
        "<math><mi>math</mi></math><img src='https://tracker.example/' alt='A diagram'>end"
    ));
    // HTML5 closes the open paragraph before the form, even though the form is discarded.
    assert_eq!(doc.plain_text(), "safe\n[Image: A diagram]end");
    assert!(doc.links().is_empty());
}

#[test]
fn link_allowlist_runs_after_entity_decoding() {
    let doc = HtmlDocument::parse(concat!(
        "<a href='jav&#x61;script:alert(1)'>JS</a> ",
        "<a href='data:text/html,evil'>data</a> ",
        "<a href='file:///etc/passwd'>file</a> ",
        "<a href='https://safe.example/?a=1&amp;b=2' onclick='bad()'>safe</a>"
    ));
    assert_eq!(doc.plain_text(), "JS data file safe");
    assert_eq!(doc.links().len(), 1);
    assert_eq!(doc.links()[0].url, "https://safe.example/?a=1&b=2");
}

#[test]
fn urls_reject_credentials_controls_and_non_web_protocols() {
    for value in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,hi",
        "vbscript:hi",
        "mailto:a@b.test",
        "https://good.example@evil.example/",
        "https://name:secret@example.com",
        "java\nscript:alert(1)",
        "https:\\evil.example",
        "https://",
        "\t",
        "https://example.com/\n",
        "",
    ] {
        assert_eq!(safe_link(value), None, "accepted {value:?}");
    }
    assert_eq!(
        safe_link("item?id=8863"),
        Some("https://news.ycombinator.com/item?id=8863".into())
    );
    assert_eq!(
        safe_link("/user?id=pg"),
        Some("https://news.ycombinator.com/user?id=pg".into())
    );
    assert_eq!(
        safe_link("https://EXAMPLE.com/a"),
        Some("https://example.com/a".into())
    );
}

#[test]
fn preformatted_code_keeps_whitespace_and_decodes_entities_once() {
    let doc = HtmlDocument::parse(
        "<pre><code>fn main() {\n    println!(&quot;&lt;rust&gt;&quot;);\n}\n</code></pre>",
    );
    assert_eq!(
        doc.blocks,
        vec![Block::Code(
            "fn main() {\n    println!(\"<rust>\");\n}\n".into()
        )]
    );
    assert_eq!(HtmlDocument::parse("&amp;lt;").plain_text(), "&lt;");
}

#[test]
fn lists_quotes_headings_and_line_breaks_have_semantic_models() {
    let doc = HtmlDocument::parse("<h2>Title</h2><blockquote>A<br>B</blockquote><ol start='3'><li>First</li><li><b>Second</b><ul><li>Nested</li></ul></li></ol><hr>");
    assert!(matches!(doc.blocks[0], Block::Heading { level: 2, .. }));
    assert!(matches!(doc.blocks[1], Block::Quote(_)));
    assert!(matches!(
        doc.blocks[2],
        Block::List {
            ordered: true,
            start: 3,
            ..
        }
    ));
    assert!(matches!(doc.blocks[3], Block::Rule));
    assert!(doc.plain_text().contains("3. First\n4. Second\n• Nested"));
    assert!(doc.plain_text().contains("A\nB"));
}

#[test]
fn links_are_deduplicated_in_appearance_order() {
    let doc = HtmlDocument::parse("<a href='https://a.example'>A</a><blockquote><a href='https://b.example'>B</a><a href='https://a.example'>again</a></blockquote>");
    assert_eq!(
        doc.links()
            .iter()
            .map(|l| l.label.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
}

#[test]
fn utf8_input_limit_never_splits_a_character() {
    let options = ParseOptions {
        max_input_bytes: 5,
        ..ParseOptions::default()
    };
    let doc = HtmlDocument::parse_with_options("a🦀z", &options);
    assert_eq!(doc.plain_text(), "a🦀");
    assert!(doc.truncated);
    let options = ParseOptions {
        max_input_bytes: 3,
        ..options
    };
    assert_eq!(
        HtmlDocument::parse_with_options("a🦀z", &options).plain_text(),
        "a"
    );
}

#[test]
fn node_and_depth_budgets_surface_truncation() {
    let limited = ParseOptions {
        max_nodes: 3,
        ..ParseOptions::default()
    };
    let doc = HtmlDocument::parse_with_options("<p>a</p><p>b</p><p>c</p>", &limited);
    assert!(doc.truncated);
    assert!(!doc.plain_text().contains('c'));
    let limited = ParseOptions {
        max_depth: 4,
        ..ParseOptions::default()
    };
    let doc = HtmlDocument::parse_with_options(
        "<div><div><div><div><div><p>deep</p></div></div></div></div></div>",
        &limited,
    );
    assert!(doc.truncated);
}

#[test]
fn deep_input_does_not_overflow_model_or_dom_drop() {
    let html = format!("{}text{}", "<div>".repeat(4096), "</div>".repeat(4096));
    let doc = HtmlDocument::parse(&html);
    assert!(doc.truncated);
}

#[test]
fn whitespace_collapses_but_nonbreaking_spaces_survive() {
    assert_eq!(
        HtmlDocument::parse("  many\n  spaces\t remain&nbsp;nonbreaking ").plain_text(),
        "many spaces remain\u{a0}nonbreaking"
    );
    assert_eq!(
        HtmlDocument::parse("<p> a <b> bold </b> <i> emphasis </i> z </p>").plain_text(),
        "a bold emphasis z"
    );
}

#[test]
fn inline_plain_text_preserves_links_and_formatting_content() {
    assert_eq!(
        Inline::plain_text(&[
            Inline::Text("a ".into()),
            Inline::Strong(vec![Inline::Text("bold".into())]),
            Inline::LineBreak,
            Inline::Link {
                url: "https://example.com/".into(),
                content: vec![Inline::Code("b".into())]
            },
        ]),
        "a bold\nb"
    );
}

#[cfg(feature = "render-tests")]
mod render {
    use super::*;
    use ferroui_base::interactivity::RoutedEventArgs;
    use ferroui_base::Size;
    use ferroui_controls::testing::UnitTestApplication;
    use ferroui_controls::{Button, HyperlinkButton, SelectableTextBlock};
    use sample_testing::TestGlobalClock;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn native_text_wraps_and_links_invoke_host_once() {
        let clock = Rc::new(TestGlobalClock::default());
        let _app = UnitTestApplication::start(sample_testing::services(clock));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let observed = calls.clone();
        let document = HtmlDocument::parse("<p>A paragraph with <b>bold</b> words and enough words to wrap gracefully at a phone width.</p><p><a href='https://example.com/'>Open article</a></p><pre>let x = 1;</pre>");
        let control = crate::render(
            &document,
            &crate::RenderOptions::default(),
            Rc::new(move |url| observed.borrow_mut().push(url.to_owned())),
        );
        control.measure(Size::new(280.0, f64::INFINITY));
        assert!(control.desired_size().height > 50.0);
        assert!(control.desired_size().width <= 280.0);
        assert!(control
            .get_logical_descendants()
            .any(|node| node.is::<SelectableTextBlock>()));
        let links: Vec<_> = control
            .get_logical_descendants()
            .filter_map(|node| node.cast::<HyperlinkButton>())
            .collect();
        assert_eq!(links.len(), 1);
        assert!(links[0].navigate_uri().is_none());
        links[0].raise_event(&RoutedEventArgs::with_event(Button::click_event()));
        assert_eq!(*calls.borrow(), vec!["https://example.com/"]);
    }

    #[test]
    fn manually_constructed_model_cannot_bypass_url_check() {
        let clock = Rc::new(TestGlobalClock::default());
        let _app = UnitTestApplication::start(sample_testing::services(clock));
        let document = HtmlDocument {
            blocks: vec![Block::Paragraph(vec![Inline::Link {
                url: "javascript:bad()".into(),
                content: vec![Inline::Text("inert".into())],
            }])],
            truncated: false,
        };
        let control = crate::render(
            &document,
            &crate::RenderOptions::default(),
            Rc::new(|_| panic!("unsafe callback")),
        );
        assert!(!control
            .get_logical_descendants()
            .any(|node| node.is::<HyperlinkButton>()));
    }
}

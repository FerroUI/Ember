# Ember rich content

`ember-content` parses Hacker News story, comment and profile HTML into an inert document model,
then renders it through FerroUI controls. It has no WebView, JavaScript execution, XAML loading,
remote-image fetching, or network dependency. The HTML5 parser handles HN's omitted paragraph end
tags, malformed formatting and named/numeric character references.

```rust
use ember_content::{HtmlDocument, RenderOptions, render};
use std::rc::Rc;

let document = HtmlDocument::parse("<p>Hello <b>HN</b>.</p><pre><code>let x = 42;</code></pre>");
let options = RenderOptions { font_size: 17.0, link_min_height: 44.0, ..Default::default() };
let content = render(&document, &options, Rc::new(|url| {
    // Route HN item/profile URLs in the app or launch through the platform's ILauncher.
    // Capture the owner weakly when the owner retains `content`.
    println!("Activate {url}");
}));
```

## Document model

Paragraphs, headings, blockquotes, ordered/unordered nested lists, code blocks, horizontal rules,
bold, emphasis, underline, strike-through, inline code, links and explicit line breaks retain their
semantics. Remote images become alt text. Tables degrade to readable rows/cells. Unsupported inert
tags retain their text. Scripts, styles, frames, embedded objects, forms, SVG, MathML and other
active content are removed with their subtrees. Attributes never become FerroUI property names or
markup. The parser performs entity decoding exactly once.

`HtmlDocument::plain_text()` supports previews, copying and local search. `links()` returns unique
normalized destinations in source order. Both work with `--no-default-features`, independently of
FerroUI and any platform backend.

## Resource and URL boundaries

Default parsing limits are 256 KiB of UTF-8 input, 4,096 visited document nodes and a depth of 32.
The input cut respects UTF-8 boundaries; source parsing uses html5ever through scraper's arena tree.
Depth limits apply when constructing the output model, and the parser arena does not recursively
drop a chain of nested DOM nodes. `HtmlDocument::truncated` reports every exhausted budget. The
renderer displays an explanation when content is truncated; the host can offer the original HN URL.

Only parsed HTTP and HTTPS destinations with a host are actionable. Embedded credentials, control
characters and backslashes are rejected. Relative URLs resolve against `https://news.ycombinator.com/`.
Validation runs both when parsing and rendering, so a manually constructed document cannot bypass
the link boundary. The renderer calls a host callback; it does not automatically navigate a second
time. Redirect handling and external navigation remain the host's responsibility.

## Rendering and themes

SelectableTextBlock, native Inline/Run/Bold/Italic/Span/Underline elements, inline HyperlinkButton
controls, Grid, Border and ScrollViewer render the document. Code retains whitespace and scrolls
horizontally; long code also scrolls vertically inside a `max_code_height` viewport (420 logical
pixels by default), with scroll chaining back to the containing thread. Text wraps within the
caller's finite width. Every link is a real focusable control
with an accessible name and destination description. `link_min_height` supports 44-pixel touch
targets; `max_link_width` limits long link captions on a narrow viewport. Text surrounding links
remains selectable. Paragraphs containing links use natural line metrics with reader spacing so
tall touch targets reserve their full height without overlapping subsequent content. Embedded
link controls are separate selection units, so use `plain_text()` for
copying the entire document including link captions.

The host supplies the current theme's text, link, code-surface and quote brushes through
`RenderOptions`. Unspecified text foreground inherits from the containing FerroUI control. Class
hooks `ember-html`, `ember-html-heading`, `ember-html-quote`, `ember-html-list`, `ember-html-code`
and `ember-html-link` allow application styles. Font size, line height, paragraph spacing and code
font are configurable; invalid floating-point inputs are normalized before reaching layout.

## Validation

Build output goes to `$CARGO_TARGET_DIR` (`target` when it is not set).

```sh
cargo test -p ember-content --no-default-features
cargo check -p ember-content
cargo test -p ember-content --features render-tests
```

Parser tests cover HN paragraphs, malformed nesting, entity decoding, active subtrees, URL attacks,
preformatted whitespace, semantic blocks, UTF-8 limits, deeply nested input and output budgets.
The optional render tests use SampleTesting's actual Skia/HarfBuzz services to verify finite-width
native layout, a link callback and renderer-side URL validation.

The parser and renderer target HN's content rather than arbitrary website article extraction.
CSS layout, embedded media, remote images, forms and browser scripting are deliberately outside
this document model.

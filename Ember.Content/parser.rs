use crate::{Block, HtmlDocument, Inline};
use scraper::{ElementRef, Html, Node};
use url::Url;

/// Limits bound parser input and the native control tree produced from a remote document.
#[derive(Clone, Debug)]
pub struct ParseOptions {
    pub max_input_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            max_input_bytes: 256 * 1024,
            max_nodes: 4096,
            max_depth: 32,
        }
    }
}

/// Normalize a destination from already entity-decoded HTML. Relative links resolve against HN.
/// Credentials, control characters, non-web schemes, and malformed authorities are rejected.
pub fn safe_link(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.contains('\\') {
        return None;
    }
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let base = Url::parse("https://news.ycombinator.com/").ok()?;
    let url = base.join(value).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.into())
}

pub(crate) fn parse(html: &str, options: &ParseOptions) -> HtmlDocument {
    let mut end = html.len().min(options.max_input_bytes);
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    // scraper uses html5ever and an arena-backed tree. Unlike a nested Rc tree, releasing an
    // adversarially deep source DOM does not recurse through a chain of child destructors.
    let fragment = Html::parse_fragment(&html[..end]);
    let mut state = Parser {
        remaining: options.max_nodes,
        max_depth: options.max_depth.min(64),
        truncated: end < html.len(),
    };
    let blocks = state.blocks(fragment.root_element(), 0);
    HtmlDocument {
        blocks,
        truncated: state.truncated,
    }
}

struct Parser {
    remaining: usize,
    max_depth: usize,
    truncated: bool,
}

impl Parser {
    fn enter(&mut self, depth: usize) -> bool {
        if depth > self.max_depth || self.remaining == 0 {
            self.truncated = true;
            return false;
        }
        self.remaining -= 1;
        true
    }

    fn blocks(&mut self, parent: ElementRef<'_>, depth: usize) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut pending = Vec::new();
        for child in parent.children() {
            if !self.enter(depth) {
                break;
            }
            let Some(element) = ElementRef::wrap(child) else {
                if let Node::Text(text) = child.value() {
                    append_text(&mut pending, text, false);
                }
                continue;
            };
            let name = element.value().name();
            if forbidden(name) {
                continue;
            }
            match name {
                "p" | "div" | "section" | "article" | "main" | "header" | "footer" | "address"
                | "figure" | "figcaption" | "details" | "summary" | "dl" | "dt" | "dd"
                | "table" | "thead" | "tbody" | "tfoot" | "tr" => {
                    flush(&mut pending, &mut blocks);
                    blocks.extend(self.blocks(element, depth + 1));
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush(&mut pending, &mut blocks);
                    let mut content = self.inlines(element, depth + 1, false);
                    normalize_inline_spaces(&mut content);
                    blocks.push(Block::Heading {
                        level: name.as_bytes()[1] - b'0',
                        content,
                    });
                }
                "blockquote" => {
                    flush(&mut pending, &mut blocks);
                    let children = self.blocks(element, depth + 1);
                    if !children.is_empty() {
                        blocks.push(Block::Quote(children));
                    }
                }
                "pre" => {
                    flush(&mut pending, &mut blocks);
                    let text = self.pre_text(element, depth + 1);
                    if !text.is_empty() {
                        blocks.push(Block::Code(text));
                    }
                }
                "ul" | "ol" => {
                    flush(&mut pending, &mut blocks);
                    let ordered = name == "ol";
                    let start = element
                        .attr("start")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    let mut items = Vec::new();
                    for li in element.children().filter_map(ElementRef::wrap) {
                        if !self.enter(depth + 1) {
                            break;
                        }
                        if li.value().name() == "li" {
                            items.push(self.blocks(li, depth + 2));
                        }
                    }
                    if !items.is_empty() {
                        blocks.push(Block::List {
                            ordered,
                            start,
                            items,
                        });
                    }
                }
                "hr" => {
                    flush(&mut pending, &mut blocks);
                    blocks.push(Block::Rule);
                }
                "td" | "th" => {
                    pending.extend(self.inlines(element, depth + 1, false));
                    append_text(&mut pending, "  ", false);
                }
                _ => pending.extend(self.inline_element(element, depth + 1, false)),
            }
        }
        flush(&mut pending, &mut blocks);
        blocks
    }

    fn inlines(&mut self, parent: ElementRef<'_>, depth: usize, preserve: bool) -> Vec<Inline> {
        let mut output = Vec::new();
        for child in parent.children() {
            if !self.enter(depth) {
                break;
            }
            match child.value() {
                Node::Text(text) => append_text(&mut output, text, preserve),
                Node::Element(_) => {
                    if let Some(element) = ElementRef::wrap(child) {
                        output.extend(self.inline_element(element, depth + 1, preserve));
                    }
                }
                _ => {}
            }
        }
        output
    }

    fn inline_element(
        &mut self,
        element: ElementRef<'_>,
        depth: usize,
        preserve: bool,
    ) -> Vec<Inline> {
        if depth > self.max_depth {
            self.truncated = true;
            return Vec::new();
        }
        let name = element.value().name();
        if forbidden(name) {
            return Vec::new();
        }
        if name == "br" {
            return vec![Inline::LineBreak];
        }
        if name == "img" {
            // Alt text is useful even though the remote image is never fetched.
            let mut result = Vec::new();
            if let Some(alt) = element.attr("alt").filter(|alt| !alt.is_empty()) {
                append_text(&mut result, &format!("[Image: {alt}]"), false);
            }
            return result;
        }
        let children = self.inlines(
            element,
            depth + 1,
            preserve || matches!(name, "code" | "kbd" | "samp"),
        );
        if children.is_empty() {
            return Vec::new();
        }
        vec![match name {
            "strong" | "b" => Inline::Strong(children),
            "em" | "i" => Inline::Emphasis(children),
            "u" => Inline::Underline(children),
            "s" | "del" | "strike" => Inline::Strike(children),
            "code" | "kbd" | "samp" => Inline::Code(Inline::plain_text(&children)),
            "a" => {
                let Some(url) = element.attr("href").and_then(safe_link) else {
                    return children;
                };
                Inline::Link {
                    url,
                    content: children,
                }
            }
            _ => return children,
        }]
    }

    fn pre_text(&mut self, parent: ElementRef<'_>, depth: usize) -> String {
        let mut text = String::new();
        self.collect_pre(parent, depth, &mut text);
        text
    }

    fn collect_pre(&mut self, parent: ElementRef<'_>, depth: usize, output: &mut String) {
        for child in parent.children() {
            if !self.enter(depth) {
                break;
            }
            match child.value() {
                Node::Text(text) => output.push_str(text),
                Node::Element(_) => {
                    if let Some(element) = ElementRef::wrap(child) {
                        if forbidden(element.value().name()) {
                            continue;
                        }
                        if element.value().name() == "br" {
                            output.push('\n');
                        } else {
                            self.collect_pre(element, depth + 1, output);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn forbidden(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "iframe"
            | "frame"
            | "frameset"
            | "object"
            | "embed"
            | "applet"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "template"
            | "noscript"
            | "svg"
            | "math"
            | "audio"
            | "video"
            | "canvas"
            | "meta"
            | "link"
            | "base"
    )
}

fn append_text(output: &mut Vec<Inline>, text: &str, preserve: bool) {
    if preserve {
        if !text.is_empty() {
            output.push(Inline::Text(text.to_owned()));
        }
        return;
    }
    // Collapse only HTML's five whitespace characters. NBSP and other Unicode spacing remain
    // significant, as they are in an HTML renderer.
    let mut normalized = String::with_capacity(text.len());
    let mut previous_space =
        matches!(output.last(), Some(Inline::Text(last)) if last.ends_with(' '));
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            if !previous_space {
                normalized.push(' ');
            }
            previous_space = true;
        } else {
            normalized.push(c);
            previous_space = false;
        }
    }
    if normalized.is_empty() {
        return;
    }
    if let Some(Inline::Text(previous)) = output.last_mut() {
        previous.push_str(&normalized);
    } else {
        output.push(Inline::Text(normalized));
    }
}

fn flush(inlines: &mut Vec<Inline>, blocks: &mut Vec<Block>) {
    if inlines.is_empty() {
        return;
    }
    normalize_inline_spaces(inlines);
    let content = std::mem::take(inlines);
    if !Inline::plain_text(&content).trim().is_empty() {
        blocks.push(Block::Paragraph(content));
    }
}

fn normalize_inline_spaces(inlines: &mut Vec<Inline>) {
    fn collapse(inlines: &mut Vec<Inline>, preceding_space: &mut bool) {
        for inline in inlines.iter_mut() {
            match inline {
                Inline::Text(text) => {
                    if *preceding_space && text.starts_with(' ') {
                        text.remove(0);
                    }
                    if !text.is_empty() {
                        *preceding_space = text.ends_with(' ');
                    }
                }
                Inline::Code(text) => {
                    if !text.is_empty() {
                        *preceding_space = text.ends_with(' ');
                    }
                }
                Inline::Strong(children)
                | Inline::Emphasis(children)
                | Inline::Underline(children)
                | Inline::Strike(children)
                | Inline::Link {
                    content: children, ..
                } => collapse(children, preceding_space),
                Inline::LineBreak => *preceding_space = true,
            }
        }
        inlines.retain(|inline| match inline {
            Inline::Text(text) | Inline::Code(text) => !text.is_empty(),
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Underline(children)
            | Inline::Strike(children)
            | Inline::Link {
                content: children, ..
            } => !children.is_empty(),
            Inline::LineBreak => true,
        });
    }
    fn trim_end(inlines: &mut [Inline]) {
        match inlines.last_mut() {
            Some(Inline::Text(text)) => *text = text.trim_end_matches(' ').to_owned(),
            Some(Inline::Strong(children))
            | Some(Inline::Emphasis(children))
            | Some(Inline::Underline(children))
            | Some(Inline::Strike(children))
            | Some(Inline::Link {
                content: children, ..
            }) => trim_end(children),
            _ => {}
        }
    }
    collapse(inlines, &mut true);
    trim_end(inlines);
}

use crate::parser::{parse, ParseOptions};

/// The supported, inert subset of HTML. A document owns its text and contains no DOM handles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlDocument {
    pub blocks: Vec<Block>,
    /// True when any input, depth, or output budget was exhausted.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: i64,
        items: Vec<Vec<Block>>,
    },
    Code(String),
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Underline(Vec<Inline>),
    Strike(Vec<Inline>),
    Code(String),
    Link { url: String, content: Vec<Inline> },
    LineBreak,
}

/// A navigable link, after URL normalization and the HTTP/HTTPS scheme allowlist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub label: String,
    pub url: String,
}

impl HtmlDocument {
    pub fn parse(html: &str) -> Self {
        parse(html, &ParseOptions::default())
    }

    pub fn parse_with_options(html: &str, options: &ParseOptions) -> Self {
        parse(html, options)
    }

    /// Plain text for copying, search, previews, speech and accessibility.
    pub fn plain_text(&self) -> String {
        let mut result = String::new();
        append_blocks(&self.blocks, &mut result);
        result.trim().to_owned()
    }

    /// First appearance order, deduplicated by normalized destination.
    pub fn links(&self) -> Vec<Link> {
        let mut links = Vec::new();
        collect_block_links(&self.blocks, &mut links);
        links
    }
}

impl Inline {
    pub fn plain_text(content: &[Self]) -> String {
        let mut result = String::new();
        append_inlines(content, &mut result);
        result
    }
}

fn append_inlines(inlines: &[Inline], output: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) => output.push_str(text),
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Underline(children)
            | Inline::Strike(children)
            | Inline::Link {
                content: children, ..
            } => append_inlines(children, output),
            Inline::LineBreak => output.push('\n'),
        }
    }
}

fn append_blocks(blocks: &[Block], output: &mut String) {
    for block in blocks {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        match block {
            Block::Paragraph(inlines)
            | Block::Heading {
                content: inlines, ..
            } => append_inlines(inlines, output),
            Block::Quote(blocks) => append_blocks(blocks, output),
            Block::List {
                ordered,
                start,
                items,
            } => {
                for (index, item) in items.iter().enumerate() {
                    if !output.is_empty() && !output.ends_with('\n') {
                        output.push('\n');
                    }
                    let mut body = String::new();
                    append_blocks(item, &mut body);
                    if *ordered {
                        output.push_str(&format!("{}. ", start.saturating_add(index as i64)));
                    } else {
                        output.push_str("• ");
                    }
                    output.push_str(body.trim());
                }
            }
            Block::Code(text) => output.push_str(text),
            Block::Rule => output.push('—'),
        }
        output.push('\n');
    }
}

fn collect_block_links(blocks: &[Block], links: &mut Vec<Link>) {
    for block in blocks {
        match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => {
                collect_inline_links(content, links)
            }
            Block::Quote(children) => collect_block_links(children, links),
            Block::List { items, .. } => {
                for item in items {
                    collect_block_links(item, links);
                }
            }
            _ => {}
        }
    }
}

fn collect_inline_links(inlines: &[Inline], links: &mut Vec<Link>) {
    for inline in inlines {
        match inline {
            Inline::Link { url, content } => {
                if !links.iter().any(|link| link.url == *url) {
                    links.push(Link {
                        label: Inline::plain_text(content),
                        url: url.clone(),
                    });
                }
            }
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Underline(children)
            | Inline::Strike(children) => collect_inline_links(children, links),
            _ => {}
        }
    }
}

//! An HTML content boundary for a native Hacker News reader.
//!
//! HTML5 parsing handles omitted paragraph end tags, malformed nesting and entities. Only a
//! small, inert document model crosses into the renderer. No supplied markup is ever loaded
//! as XAML, passed to a browser, executed, or used to fetch an image. Resource limits apply
//! before parsing and while constructing the model. See [`ParseOptions`].

mod document;
mod parser;

pub use document::{Block, HtmlDocument, Inline, Link};
pub use parser::{safe_link, ParseOptions};

#[cfg(feature = "render")]
mod find;
#[cfg(feature = "render")]
mod renderer;
#[cfg(feature = "render")]
pub use renderer::{
    render, render_with_code_toolbar, render_with_find, CodeToolbar, FindHighlight, RenderOptions,
};

#[cfg(test)]
mod tests;

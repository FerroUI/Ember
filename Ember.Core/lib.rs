//! Ember's platform-neutral reader model and offline data layer.
//!
//! The official Hacker News API is public and read-only. Account actions are
//! explicit browser handoffs; local saved/read/hidden state never impersonates
//! an HN account action. See <https://github.com/HackerNews/API>.

mod model;
mod repository;
mod thread;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

pub use model::*;
pub use repository::*;
pub use thread::*;

#[cfg(test)]
mod tests;

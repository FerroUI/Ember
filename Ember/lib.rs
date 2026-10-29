//! Ember's shared retained-mode UI. Every platform hosts the same control tree.
extern crate self as ember;
mod app;
pub mod controls;
mod icons;
pub mod markup;
mod reader_view;
mod register_types;
mod search_panel;
mod services;
mod settings;
mod theme;
mod thread_view;
mod widgets;
ferroui_markup_xaml::include_compiled_xaml!();
pub use app::App;
pub use markup::SAMPLE;
pub use reader_view::ReaderView;
pub use reader_view::{navigate_location, set_location_changed};
pub use register_types::{register_types, ASSEMBLY};
pub use services::{set_database_path, set_initial_location, set_repository_factory};
#[cfg(test)]
mod tests;

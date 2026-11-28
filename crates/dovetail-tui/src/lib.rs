//! Dovetail's terminal UI: buffers, regions, keymap, theme and rendering.
//!
//! Depends only on `dovetail-api`: it holds its own copy of the [`dovetail_api::Snapshot`],
//! changes it only through [`dovetail_api::apply`] and asks the core for changes with
//! [`dovetail_api::Command`]s.

pub mod app;
pub mod buffers;
pub mod canvas;
pub mod capture;
pub mod chrome;
pub mod event_loop;
pub mod fmt;
pub mod input;
pub mod keymap;
pub mod layout;
pub mod mcp_menu;
pub mod model_select;
pub mod picker;
pub mod render;
pub mod switcher;
pub mod theme;
pub mod usage_menu;

pub use app::App;
pub use event_loop::{Clock, run};
pub use keymap::Mode;
pub use theme::Theme;

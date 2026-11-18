//! Dovetail's terminal UI: buffers, regions, keymap, theme and rendering.
//!
//! Depends only on `dovetail-api`: it holds its own copy of the [`dovetail_api::Snapshot`],
//! changes it only through [`dovetail_api::apply`] and asks the core for changes with
//! [`dovetail_api::Command`]s.

pub mod action;
pub mod activity;
pub mod app;
pub mod buffers;
pub mod canvas;
pub mod capture;
pub mod command;
pub mod complete;
pub mod event_loop;
pub mod fmt;
pub mod help;
pub mod input;
pub mod keymap;
pub mod mode;
pub mod model_select;
pub mod picker;
pub mod region;
pub mod render;
pub mod slash;
pub mod switcher;
pub mod tabs;
pub mod theme;
pub mod usage;

pub use app::App;
pub use event_loop::{Clock, run};
pub use mode::Mode;
pub use theme::Theme;

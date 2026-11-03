//! Dovetail's terminal UI: buffers, regions, keymap, theme and rendering.
//!
//! Depends only on `dovetail-api`: it holds its own copy of the [`dovetail_api::Snapshot`],
//! changes it only through [`dovetail_api::apply`] and asks the core for changes with
//! [`dovetail_api::Command`]s.

pub mod action;
pub mod app;
pub mod buffers;
pub mod command;
pub mod event_loop;
pub mod fmt;
pub mod input;
pub mod keymap;
pub mod mode;
pub mod region;
pub mod theme;

pub use app::App;
pub use event_loop::{Clock, run};
pub use mode::Mode;
pub use theme::Theme;

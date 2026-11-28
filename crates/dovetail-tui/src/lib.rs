#![doc = include_str!("../ARCHITECTURE.md")]

pub mod app;
pub mod buffers;
pub mod canvas;
pub mod chrome;
pub mod event_loop;
pub mod fmt;
pub mod input;
pub mod keymap;
pub mod layout;
pub mod overlay;
pub mod render;
pub mod theme;

pub use app::App;
pub use event_loop::{Clock, run};
pub use keymap::Mode;
pub use overlay::capture;
pub use theme::Theme;

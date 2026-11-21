//! The `app` feature: the app itself: resync, quit, open a URL, command failures.

mod command;
mod event;
pub use command::AppCommand;
pub use event::AppEvent;

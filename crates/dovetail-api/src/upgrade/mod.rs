//! The `upgrade` feature: building and swapping in the next binary.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::UpgradeCommand;
pub use event::UpgradeEvent;

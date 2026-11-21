//! The `terminal` feature: handing the user's terminal to an editor or to native `claude`.

mod command;
mod event;
pub use command::TerminalCommand;
pub use event::TerminalEvent;

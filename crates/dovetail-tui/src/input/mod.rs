//! Text editing, `@` path completion, `/` command interception and the chat input bar.

pub mod chat;
pub mod complete;
pub mod slash;
mod text;

pub use chat::{ChatInput, ChatKeyOutcome};
pub use text::*;

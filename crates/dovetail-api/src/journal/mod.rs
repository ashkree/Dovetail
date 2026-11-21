//! The `journal` feature: the end-of-day journal and the coordinator's handoffs.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::JournalCommand;
pub use event::JournalEvent;

use serde::{Deserialize, Serialize};

/// Where the project's end-of-day journal stands (ADR 0012).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalState {
    /// The date of the newest journal file, `YYYY-MM-DD` (local time); `None` when there is
    /// none yet.
    pub latest: Option<String>,
    /// `:eod` is waiting for the coordinator's handoff before it writes the day's file.
    pub closing: bool,
    /// The handoff `closing` waits for is a `:fresh` start's, not `:eod`'s.
    #[serde(default)]
    pub fresh: bool,
}

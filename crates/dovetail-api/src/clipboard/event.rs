//! The `clipboard` feature's events.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::clipboard::Paste;

/// A change to the client's snapshot, or an answer, in the feature `clipboard`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardEvent {
    /// The clipboard was read, in answer to [`ClipboardCommand::Paste`]. Changes nothing in the
    /// snapshot; the UI puts the result in that agent's chat bar if it is still open.
    Pasted {
        /// The agent whose chat bar asked.
        agent: AgentId,
        /// What the clipboard gave.
        paste: Paste,
    },
}

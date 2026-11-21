//! The `terminal` feature's events.

use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `terminal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEvent {
    /// The core accepted [`super::TerminalCommand::OpenEditor`]: the UI lets go of the terminal and answers
    /// [`super::TerminalCommand::Released`]. Changes nothing in the snapshot.
    Lend,
    /// The program run in the user's terminal ([`super::TerminalCommand::OpenEditor`], [`super::TerminalCommand::TakeOver`])
    /// has exited, so the UI takes the terminal back. Changes nothing in the snapshot.
    Returned {
        /// Why the program could not run or failed, for the status bar; `None` when it exited
        /// cleanly.
        error: Option<String>,
    },
}

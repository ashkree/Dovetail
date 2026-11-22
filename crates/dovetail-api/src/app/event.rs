//! The `app` feature's events.

#[allow(unused_imports)]
use super::AppCommand;
use super::Paste;
use crate::agents::agent::AgentId;
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, from the app. The terminal, clipboard and
/// upgrade answers live here too; only `UpgradeBuilding` changes the snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEvent {
    /// A command could not be carried out; the UI shows the message.
    CommandFailed {
        /// What went wrong.
        message: String,
    },
    /// The core accepted [`AppCommand::OpenEditor`]: the UI lets go of the terminal and answers
    /// [`AppCommand::TerminalReleased`]. Changes nothing in the snapshot.
    TerminalLend,
    /// The program run in the user's terminal ([`AppCommand::OpenEditor`], [`AppCommand::TakeOver`])
    /// has exited, so the UI takes the terminal back. Changes nothing in the snapshot.
    TerminalReturned {
        /// Why the program could not run or failed, for the status bar; `None` when it exited
        /// cleanly.
        error: Option<String>,
    },
    /// The clipboard was read, in answer to [`AppCommand::Paste`]. Changes nothing in the
    /// snapshot; the UI puts the result in that agent's chat bar if it is still open.
    Pasted {
        /// The agent whose chat bar asked.
        agent: AgentId,
        /// What the clipboard gave.
        paste: Paste,
    },
    /// `:upgrade` started building the next binary (`true`) or the build finished, failed or was
    /// cancelled (`false`); the result arrives as a Needs-you item.
    UpgradeBuilding(bool),
    /// Every agent is idle and the user said `y` on "Upgrade ready" (ADR 0014): the core is
    /// quitting, and the UI quits too, without asking. The binary then swaps the staged build in
    /// and starts it in the same terminal. Changes nothing in the snapshot.
    UpgradeRestarting,
}

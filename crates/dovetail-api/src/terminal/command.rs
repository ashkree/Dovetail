//! The `terminal` feature's commands.

use crate::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The terminal: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCommand {
    /// Open a folder or file in the user's editor (`$EDITOR`, else `nvim`), in the terminal the
    /// UI runs in. A request: the UI still holds the terminal. The core checks the path first;
    /// a missing one comes back as [`crate::app::AppEvent::CommandFailed`] and the UI never lets go (no
    /// flicker). Otherwise the core answers [`super::TerminalEvent::Lend`], the UI lets go of the
    /// terminal (leaves the alternate screen and raw mode, stops reading keys) and sends
    /// [`TerminalCommand::Released`], and the core runs the editor. The UI takes the terminal
    /// back on [`super::TerminalEvent::Returned`], which the core sends once the editor exits or fails
    /// to start.
    OpenEditor {
        /// What to open; the editor also starts in it when it is a folder.
        path: PathBuf,
    },
    /// The UI has let go of the terminal, as [`super::TerminalEvent::Lend`] asked: the core runs the
    /// editor it accepted with [`TerminalCommand::OpenEditor`]. Ignored when none is waiting.
    Released,
    /// Hand a session to native `claude` in the UI's terminal (`<leader>st`, M14). Like
    /// [`TerminalCommand::OpenEditor`], the UI sends it only after it has let go of the terminal, and
    /// takes the terminal back on [`super::TerminalEvent::Returned`], which the core always sends: at
    /// once when it refuses, else when native exits. Headless resumes after.
    TakeOver {
        /// The session.
        agent: AgentId,
    },
}

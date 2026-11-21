//! The `clipboard` feature's commands.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::clipboard::Paste;

/// The clipboard: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardCommand {
    /// Read the system clipboard for an agent's chat bar (Ctrl-V). The core answers with
    /// [`ClipboardEvent::Pasted`]: an image saved as an attachment, text, or a refusal.
    Paste {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
    },
    /// Text pasted into an agent's chat bar (a bracketed paste or a dropped file). The core
    /// answers with [`ClipboardEvent::Pasted`]: an image attachment when the text is a path to
    /// an existing image (`~`, `file://` and shell quoting understood), else the text itself.
    PasteText {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
        /// What the terminal delivered.
        text: String,
    },
    /// Put text on the system clipboard (`c` in the worktree view copies a path). A failure
    /// comes back as [`crate::app::AppEvent::CommandFailed`]; success sends nothing.
    Copy {
        /// The text.
        text: String,
    },
}

//! The `app` feature's commands.

use crate::agents::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The app: what the UI asks the core to do. Besides the app itself this covers handing over the
/// terminal (editor, native `claude`), the system clipboard and `:upgrade`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCommand {
    /// Open a web page in the user's browser (`gx` in reading mode, ADR 0018): `$BROWSER` if
    /// set, else `xdg-open`. Only `http` and `https` URLs; anything else, or a launch that fails,
    /// comes back as [`AppEvent::CommandFailed`]. Success sends nothing.
    OpenExternal {
        /// The page.
        url: String,
    },
    /// Send a fresh snapshot (after a [`crate::ApplyResult::Gap`]).
    Resync,
    /// Shut down.
    Quit {
        /// Also end running sessions instead of leaving them to resume.
        end_sessions: bool,
    },
    /// Open a folder or file in the user's editor (`$EDITOR`, else `nvim`), in the terminal the
    /// UI runs in. A request: the UI still holds the terminal. The core checks the path first;
    /// a missing one comes back as [`AppEvent::CommandFailed`] and the UI never lets go (no
    /// flicker). Otherwise the core answers [`AppEvent::TerminalLend`], the UI lets go of the
    /// terminal (leaves the alternate screen and raw mode, stops reading keys) and sends
    /// [`AppCommand::TerminalReleased`], and the core runs the editor. The UI takes the terminal
    /// back on [`AppEvent::TerminalReturned`], which the core sends once the editor exits or fails
    /// to start.
    OpenEditor {
        /// What to open; the editor also starts in it when it is a folder.
        path: PathBuf,
    },
    /// The UI has let go of the terminal, as [`AppEvent::TerminalLend`] asked: the core runs the
    /// editor it accepted with [`AppCommand::OpenEditor`]. Ignored when none is waiting.
    TerminalReleased,
    /// Hand a session to native `claude` in the UI's terminal (`<leader>st`, M14). Like
    /// [`AppCommand::OpenEditor`], the UI sends it only after it has let go of the terminal, and
    /// takes the terminal back on [`AppEvent::TerminalReturned`], which the core always sends: at
    /// once when it refuses, else when native exits. Headless resumes after.
    TakeOver {
        /// The session.
        agent: AgentId,
    },
    /// Read the system clipboard for an agent's chat bar (Ctrl-V). The core answers with
    /// [`AppEvent::Pasted`]: an image saved as an attachment, text, or a refusal.
    Paste {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
    },
    /// Text pasted into an agent's chat bar (a bracketed paste or a dropped file). The core
    /// answers with [`AppEvent::Pasted`]: an image attachment when the text is a path to
    /// an existing image (`~`, `file://` and shell quoting understood), else the text itself.
    PasteText {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
        /// What the terminal delivered.
        text: String,
    },
    /// Put text on the system clipboard (`c` in the worktree view copies a path). A failure
    /// comes back as [`AppEvent::CommandFailed`]; success sends nothing.
    Copy {
        /// The text.
        text: String,
    },
    /// `:upgrade` (ADR 0014): build the next binary in the background. An "Upgrade ready" item
    /// appears in Needs you when it is done, an Error item when the build fails; a refusal comes
    /// back as [`AppEvent::CommandFailed`].
    StartUpgrade {
        /// Build the project's checkout instead of `origin/main` (`:upgrade --local`).
        local: bool,
    },
}

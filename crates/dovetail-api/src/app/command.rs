//! The `app` feature's commands.

use serde::{Deserialize, Serialize};

/// The app: what the UI asks the core to do.
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
}

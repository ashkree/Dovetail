//! Which CLI runs a session (task #46, `docs/spikes/agy-headless.md`).

use serde::{Deserialize, Serialize};

/// The CLI a session runs on. Fixed for the life of a session: switching means a new session.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    /// Claude Code (`claude`), over its stream-json protocol.
    #[default]
    Claude,
    /// Antigravity (`agy acp`), over the Agent Client Protocol.
    Agy,
}

impl DriverKind {
    /// Every driver, in the order the pickers show them.
    pub const ALL: [DriverKind; 2] = [DriverKind::Claude, DriverKind::Agy];

    /// The stored and serialized name.
    pub fn as_str(self) -> &'static str {
        match self {
            DriverKind::Claude => "claude",
            DriverKind::Agy => "agy",
        }
    }

    /// Parses [`DriverKind::as_str`]'s name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Claude => "Claude Code",
            DriverKind::Agy => "agy",
        }
    }
}

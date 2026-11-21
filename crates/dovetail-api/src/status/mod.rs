//! The `status` feature: the status bar: totals and warnings.

mod apply;
mod event;
pub(crate) use apply::apply;
pub use event::StatusEvent;

use serde::{Deserialize, Serialize};

/// Identifies a status-bar warning, so it can be replaced or cleared.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WarningId(pub String);

/// Id of the warning shown while the open folder is not a Dovetail project yet (ADR 0005). The
/// TUI shows it at the right of the status bar, ahead of the usage line.
pub const UNREGISTERED_WARNING: &str = "unregistered";

/// Status-bar totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    /// Memory across all sessions, in MB.
    pub memory_mb: u32,
    /// Items in Needs you.
    pub needs_you: u32,
    /// Cost across all agents, in API-equivalent US dollars.
    pub cost_usd: f64,
}

/// A status-bar warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// Warning id.
    pub id: WarningId,
    /// What to tell the user.
    pub text: String,
}

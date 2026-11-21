//! The `upgrade` feature's commands.

use serde::{Deserialize, Serialize};

/// Upgrades: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeCommand {
    /// `:upgrade` (ADR 0014): build the next binary in the background. An "Upgrade ready" item
    /// appears in Needs you when it is done, an Error item when the build fails; a refusal comes
    /// back as [`crate::app::AppEvent::CommandFailed`].
    Start {
        /// Build the project's checkout instead of `origin/main` (`:upgrade --local`).
        local: bool,
    },
}

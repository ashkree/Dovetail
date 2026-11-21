//! The `worktree` feature's commands.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;

/// Worktrees: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeCommand {
    /// Remove a slice's worktree.
    Remove {
        /// The slice.
        agent: AgentId,
        /// The user has confirmed losing unpushed work.
        confirmed: bool,
    },
    /// Gather every slice worktree's git and PR state again (sent when the worktree view
    /// opens): `git fetch origin <base>` first, then local git and `gh pr list`. The core
    /// answers with one [`WorktreeEvent::Upserted`] per worktree.
    Refresh,
}

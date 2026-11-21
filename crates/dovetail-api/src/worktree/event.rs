//! The `worktree` feature's events.

use crate::worktree::WorktreeInfo;
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `worktree`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeEvent {
    /// A slice's worktree state was gathered afresh. A worktree's entry goes with its agent
    /// ([`crate::agent::AgentEvent::Removed`]).
    Upserted(WorktreeInfo),
}

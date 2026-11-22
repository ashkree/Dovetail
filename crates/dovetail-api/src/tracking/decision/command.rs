//! The `decision` feature's commands.

use crate::decision::{Alternative, DecisionId};
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// Decision logs: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCommand {
    /// Add an entry to a task's decision log, as the user.
    Log(NewDecision),
}

/// Fields for a new decision-log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDecision {
    /// The task it belongs to.
    pub task: TaskId,
    /// The decision, in one line.
    pub text: String,
    /// Why, if given.
    pub why: Option<String>,
    /// Options considered and not taken.
    pub alternatives: Vec<Alternative>,
    /// The earlier entry it replaces.
    pub supersedes: Option<DecisionId>,
}

//! The `decision` feature: decision logs: what was decided on a task, and why.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::{DecisionCommand, NewDecision};
pub use event::DecisionEvent;

use crate::agent::AgentId;
use crate::research::Evidence;
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// Identifies an entry in a task's decision log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DecisionId(pub u64);

/// One entry in a task's decision log. Entries are never edited: a later one can supersede it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Entry id, increasing.
    pub id: DecisionId,
    /// The task it belongs to.
    pub task: TaskId,
    /// When it was logged, in Unix milliseconds.
    pub at_ms: i64,
    /// Who logged it.
    pub by: DecisionAuthor,
    /// The decision, in one line.
    pub text: String,
    /// Why, if given.
    pub why: Option<String>,
    /// Options that were considered and not taken.
    pub alternatives: Vec<Alternative>,
    /// The earlier entry this one replaces.
    pub supersedes: Option<DecisionId>,
    /// The findings and sources it rests on (decisions accepted from `research_propose`).
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

/// An option considered for a decision and not taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alternative {
    /// The option, in one line.
    pub option: String,
    /// Why it lost, if given.
    pub why_not: Option<String>,
}

/// Who logged a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAuthor {
    /// The user, by hand.
    User,
    /// An agent, through `core_log_decision`.
    Agent(AgentId),
}

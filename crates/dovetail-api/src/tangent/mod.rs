//! The `tangent` feature: tangents: questions that do not belong to the open project.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::TangentCommand;
pub use event::TangentEvent;

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::research::ResearchSummary;
use crate::task::{Task, TaskId, TaskStatus};

/// The id of Tangents, the built-in research project every tangent is saved in (ADR 0017,
/// amended 2026-11-11). It is reserved: no other project can take it, it is always registered
/// and it cannot be archived.
pub const TANGENTS_PROJECT: &str = "tangents";

/// A tangent: a question that did not belong to the project it was asked from, saved as a task
/// in Tangents' Unsorted list with its answers and sources (ADR 0017, amended 2026-11-11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TangentSummary {
    /// Its task in Tangents (the id is Tangents' own, not the open project's).
    pub task: Task,
    /// Its answers and sources, once one was filed.
    pub research: Option<ResearchSummary>,
}

impl TangentSummary {
    /// Not triaged yet: still to do and in Tangents' Unsorted list. The morning's "new
    /// tangents" item counts these.
    pub fn is_new(&self) -> bool {
        self.task.bucket.is_none() && self.task.status == TaskStatus::Todo
    }
}

/// The agent id of the tangent session whose tangent is task `task` of Tangents.
pub fn tangent_agent(task: TaskId) -> AgentId {
    AgentId(format!("tangent-{}", task.0))
}

/// The Tangents task a tangent session's agent id names (see [`tangent_agent`]).
pub fn tangent_task(agent: &AgentId) -> Option<TaskId> {
    agent.0.strip_prefix("tangent-")?.parse().ok().map(TaskId)
}

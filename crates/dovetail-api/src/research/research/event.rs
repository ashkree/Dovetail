//! The `research` feature's events.

use crate::research::{ResearchContent, ResearchSummary, ResearchWhat};
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `research`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchEvent {
    /// A task's research changed: a round opened or closed, a finding was filed.
    Upserted(ResearchSummary),
    /// Research content, in answer to [`super::ResearchCommand::Load`]. Changes nothing in the
    /// snapshot.
    Loaded {
        /// The task.
        task: TaskId,
        /// What was asked for.
        what: ResearchWhat,
        /// The answer.
        content: ResearchContent,
    },
}

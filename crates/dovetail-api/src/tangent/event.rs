//! The `tangent` feature's events.

use serde::{Deserialize, Serialize};
use crate::research::{ResearchContent, ResearchWhat};
use crate::tangent::TangentSummary;
use crate::task::TaskId;

/// A change to the client's snapshot, or an answer, in the feature `tangent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TangentEvent {
    /// A tangent was saved or changed: asked, answered, triaged (ADR 0017).
    Upserted(TangentSummary),
    /// A tangent left the list: dropped, filed into a project, or attached to a task (its
    /// Tangents task is archived).
    Removed(TaskId),
    /// A tangent's content, in answer to [`TangentCommand::Load`]. Changes nothing in the
    /// snapshot.
    Loaded {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// What was asked for.
        what: ResearchWhat,
        /// The answer.
        content: ResearchContent,
    },
}

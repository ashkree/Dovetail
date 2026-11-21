//! The `tangent` feature's commands.

use serde::{Deserialize, Serialize};
use crate::project::ProjectId;
use crate::research::ResearchWhat;
use crate::task::TaskId;

/// Tangents: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TangentCommand {
    /// Ask a tangent (`:tangent [question]`, `T` in the side panel; ADR 0017): a question that
    /// does not belong to the open project. The core saves it as a task in Tangents' Unsorted
    /// list and starts a read-only session for it in Tangents' folder (side chats' launch lists,
    /// Core suite only), shown in the side panel. Its answers and the sources it and its
    /// subagents reached are filed on that task as findings. The core answers with
    /// [`TangentEvent::Upserted`] and [`crate::agent::AgentEvent::Upserted`].
    New {
        /// The question, also its first prompt.
        question: String,
    },
    /// Attach a tangent to a task of the open project (`a` in the side panel or the tangents
    /// list): its answers and sources become quick findings of `task`, and the tangent leaves
    /// Tangents (its task there is archived, with a note saying where it went).
    Attach {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// The open project's task.
        task: TaskId,
    },
    /// File a tangent into a project (`f` in the tangents list): a new task in that project's
    /// Unsorted list with the tangent's question, notes, answers and sources; the tangent leaves
    /// Tangents (archived there, with a note). Refused for Tangents itself.
    File {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// The project.
        project: ProjectId,
    },
    /// Start a task from a tangent (`s` in the tangents list): a new task in the project's
    /// Unsorted list whose notes link back to the tangent, which stays in Tangents, marked done.
    StartFrom {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// The project.
        project: ProjectId,
    },
    /// Drop a tangent (`d` in the tangents list): its task in Tangents is archived, nothing is
    /// deleted.
    Drop {
        /// The tangent's task in Tangents.
        tangent: TaskId,
    },
    /// Fetch a tangent's content, as [`crate::research::ResearchCommand::Load`] does for the open project's
    /// tasks. The core answers with [`TangentEvent::Loaded`].
    Load {
        /// The tangent's task in Tangents.
        tangent: TaskId,
        /// What to fetch.
        what: ResearchWhat,
    },
}

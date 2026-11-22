//! The `task` feature's commands.

use crate::effort::{CriterionId, EffortId};
use crate::task::{Bucket, PLevel, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

/// Tasks: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCommand {
    /// Create a task.
    Create(NewTask),
    /// Change fields of a task.
    Update {
        /// The task.
        id: TaskId,
        /// Changes, applied in order.
        patch: Vec<TaskChange>,
    },
    /// Put a task in an effort and say which of its criteria the task serves. Moving to another
    /// effort with `criterion: None` clears the criterion. Refused for a closed effort, or a
    /// criterion of another effort. The core answers with [`super::TaskEvent::Upserted`].
    Move {
        /// The task.
        id: TaskId,
        /// The effort it belongs to from now on.
        effort: EffortId,
        /// The criterion it serves, if any.
        criterion: Option<CriterionId>,
    },
    /// Archive a task.
    Archive {
        /// The task.
        id: TaskId,
    },
    /// Move a task up (negative) or down within its bucket.
    Reorder {
        /// The task.
        id: TaskId,
        /// Places to move.
        delta: i32,
    },
}

/// Fields for a new task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    /// One-line title.
    pub title: String,
    /// P-level, if given.
    pub p_level: Option<PLevel>,
    /// Bucket; `None` puts it in Unsorted.
    pub bucket: Option<Bucket>,
    /// Notes.
    #[serde(default)]
    pub notes: String,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Linked pull request number.
    pub pr: Option<u32>,
    /// Tasks it is related to (both ways).
    #[serde(default)]
    pub related: Vec<TaskId>,
    /// The effort it belongs to; `None` puts it in Unsorted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortId>,
    /// The criterion of that effort it serves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<CriterionId>,
}

/// One field change in [`TaskCommand::Update`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskChange {
    /// New title.
    Title(String),
    /// New status.
    Status(TaskStatus),
    /// New P-level, or none.
    PLevel(Option<PLevel>),
    /// New bucket, or Unsorted.
    Bucket(Option<Bucket>),
    /// New notes.
    Notes(String),
    /// New dependencies.
    Deps(Vec<TaskId>),
    /// New linked PR, or none.
    Pr(Option<u32>),
    /// The user has seen a coordinator-created task.
    Seen,
    /// Mark a task as not yet seen by the user: the coordinator moved it into Now.
    Unseen,
    /// Relate this task to another, both ways.
    Relate(TaskId),
    /// Remove a relation, both ways.
    Unrelate(TaskId),
}

impl TaskChange {
    /// A few words for the Activity feed; `None` for changes that are not worth a line.
    pub fn describe(&self) -> Option<String> {
        Some(match self {
            Self::Title(t) => format!("renamed to \"{t}\""),
            Self::Status(s) => format!("status {s:?}").to_lowercase(),
            Self::PLevel(Some(p)) => format!("priority {p:?}"),
            Self::PLevel(None) => "priority cleared".to_owned(),
            Self::Bucket(Some(b)) => format!("moved to {b:?}"),
            Self::Bucket(None) => "moved to Unsorted".to_owned(),
            Self::Notes(_) => "notes edited".to_owned(),
            Self::Deps(_) => "dependencies changed".to_owned(),
            Self::Pr(Some(n)) => format!("linked PR #{n}"),
            Self::Pr(None) => "PR unlinked".to_owned(),
            Self::Relate(t) => format!("related to #{}", t.0),
            Self::Unrelate(t) => format!("unrelated from #{}", t.0),
            Self::Seen | Self::Unseen => return None,
        })
    }
}

/// The Activity text for a task patch, `None` when it holds nothing worth a line.
pub fn describe_task_patch(id: TaskId, title: &str, patch: &[TaskChange]) -> Option<String> {
    let parts: Vec<String> = patch.iter().filter_map(TaskChange::describe).collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("task #{} {title}: {}", id.0, parts.join(", ")))
}

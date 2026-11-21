//! The `task` feature's events.

use serde::{Deserialize, Serialize};
use crate::task::{Task, TaskId};

/// A change to the client's snapshot, or an answer, in the feature `task`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEvent {
    /// A task was created or changed.
    Upserted(Task),
    /// A task was deleted.
    Removed(TaskId),
}

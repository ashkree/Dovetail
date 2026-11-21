//! The `task` feature: tasks: the user's own work items, their status, order and links.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::{TaskCommand, NewTask, TaskChange, describe_task_patch};
pub use event::TaskEvent;

use serde::{Deserialize, Serialize};
use crate::effort::{CriterionId, EffortId};

/// Identifies one of the user's tasks. Matches the `tasks.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub i64);

/// A piece of the user's own work, with or without agents attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Task id.
    pub id: TaskId,
    /// One-line title.
    pub title: String,
    /// Where the task stands.
    pub status: TaskStatus,
    /// How important it is; `None` for some unsorted tasks.
    pub p_level: Option<PLevel>,
    /// When the user will do it; `None` means Unsorted.
    pub bucket: Option<Bucket>,
    /// Position within its bucket, lowest first.
    pub order: i64,
    /// Tasks this one waits for.
    pub deps: Vec<TaskId>,
    /// Free-form notes.
    pub notes: String,
    /// Linked pull request number.
    pub pr: Option<u32>,
    /// The task this one follows up.
    pub follow_up_of: Option<TaskId>,
    /// Created by the coordinator and not yet seen by the user.
    pub coordinator_created: bool,
    /// Tasks related to this one, both ways; no effect on order or status.
    #[serde(default)]
    pub related: Vec<TaskId>,
    /// The effort it belongs to. A peer that does not send it means Unsorted.
    #[serde(default)]
    pub effort: EffortId,
    /// The acceptance criterion of that effort it serves; `None` flags a task that serves none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<CriterionId>,
}

/// Where a task stands. Whether it needs you is not a status: it follows from the inbox
/// items of its agents (plan B4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Not started.
    Todo,
    /// Started; set when its first slice starts, and kept after slices close.
    InProgress,
    /// The work is up for review: its slice's pull request is open, or its agent reported it
    /// done (ADR 0024). Dovetail sets it; agents cannot.
    Review,
    /// Done: its pull request was merged, or the user marked it done.
    Done,
    /// Hidden from the tree.
    Archived,
}

/// How important a task is, from P0 (most) to P3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PLevel {
    /// Most important.
    P0,
    /// Important.
    P1,
    /// Normal.
    P2,
    /// Least important.
    P3,
}

/// When the user will do a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    /// Doing it now.
    Now,
    /// Up next.
    Next,
    /// Some time later.
    Later,
}

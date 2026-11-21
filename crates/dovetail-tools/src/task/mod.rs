//! Tools of the `tasks` suite: create, list, get, update, and archive tasks.

use dovetail_api::task::{Bucket, PLevel, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::{ToolError, task_id, text};

/// Tools in the `tasks` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTool {
    /// `tasks_create` (ADR 0002).
    Create(CreateTask),
    /// `tasks_list` (task #20).
    List(ListTasks),
    /// `tasks_get` (task #20).
    Get(GetTask),
    /// `tasks_update` (task #25).
    Update(UpdateTask),
    /// `tasks_archive` (task #9).
    Archive(ArchiveTask),
}

impl TaskTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            TaskTool::Create(_) => "tasks_create",
            TaskTool::List(_) => "tasks_list",
            TaskTool::Get(_) => "tasks_get",
            TaskTool::Update(_) => "tasks_update",
            TaskTool::Archive(_) => "tasks_archive",
        }
    }

    pub(crate) fn validate(&self, _role: Role) -> Result<(), ToolError> {
        match self {
            TaskTool::Create(c) => {
                text("title", &c.title)?;
                if c.bucket == Some(Bucket::Now) {
                    return Err(ToolError::BucketNow);
                }
                Ok(())
            }
            TaskTool::List(_) => Ok(()),
            TaskTool::Get(g) => task_id("id", g.id),
            TaskTool::Update(u) => {
                task_id("id", u.id)?;
                if let Some(title) = &u.title {
                    text("title", title)?;
                }
                match u.status {
                    Some(TaskStatus::Archived) => return Err(ToolError::UpdateToArchived),
                    Some(_) => return Err(ToolError::StatusIsSetByDovetail),
                    None => {}
                }
                if u.bucket.is_none()
                    && u.p_level.is_none()
                    && u.title.is_none()
                    && u.notes.is_none()
                {
                    return Err(ToolError::NothingToUpdate);
                }
                Ok(())
            }
            TaskTool::Archive(a) => task_id("id", a.id),
        }
    }
}

/// `tasks_create`: a task filed by the coordinator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTask {
    /// Short title, 6 words at most.
    pub title: String,
    /// Context, acceptance criteria, links.
    #[serde(default)]
    pub notes: String,
    /// Priority, if given.
    pub p_level: Option<PLevel>,
    /// `next` or `later`; `None` lands in Unsorted. `now` fails [`crate::Tool::validate`].
    pub bucket: Option<Bucket>,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Related tasks.
    #[serde(default)]
    pub related: Vec<TaskId>,
}

/// A bucket as `tasks_list` filters it: the three [`Bucket`]s plus Unsorted (no bucket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketFilter {
    /// [`Bucket::Now`].
    Now,
    /// [`Bucket::Next`].
    Next,
    /// No bucket yet.
    Unsorted,
    /// [`Bucket::Later`].
    Later,
}

impl BucketFilter {
    /// Whether a task with `bucket` is in this one.
    pub fn matches(self, bucket: Option<Bucket>) -> bool {
        match self {
            BucketFilter::Now => bucket == Some(Bucket::Now),
            BucketFilter::Next => bucket == Some(Bucket::Next),
            BucketFilter::Unsorted => bucket.is_none(),
            BucketFilter::Later => bucket == Some(Bucket::Later),
        }
    }
}

/// `tasks_list`: one line per task, Now, Next, Unsorted, Later (task #20). Every field is
/// optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListTasks {
    /// Only tasks in this bucket.
    pub bucket: Option<BucketFilter>,
    /// Only tasks with this status; `archived` lists archived tasks without
    /// `include_archived`.
    pub status: Option<TaskStatus>,
    /// Also list archived tasks.
    pub include_archived: bool,
}

/// `tasks_get`: one task in full, with its agents and decision log (task #20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetTask {
    /// The task.
    pub id: TaskId,
}

/// `tasks_update`: change some fields of a task (task #25). Only the fields given change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateTask {
    /// The task.
    pub id: TaskId,
    /// No longer settable: Dovetail sets task status itself (ADR 0024). Still parsed, so a
    /// caller that sends it gets [`crate::Tool::validate`]'s explanation, not a schema error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    /// `now`, `next`, `later` or `unsorted`. A task the coordinator moves into Now is marked
    /// unseen until the user sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<BucketFilter>,
    /// Absent: unchanged; `null`: no priority; a level: that level.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub p_level: Option<Option<PLevel>>,
    /// New title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// New notes, replacing the old.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl UpdateTask {
    /// Where `bucket` moves the task: `Some(None)` is Unsorted, `None` leaves it.
    pub fn new_bucket(&self) -> Option<Option<Bucket>> {
        self.bucket.map(|b| match b {
            BucketFilter::Now => Some(Bucket::Now),
            BucketFilter::Next => Some(Bucket::Next),
            BucketFilter::Unsorted => None,
            BucketFilter::Later => Some(Bucket::Later),
        })
    }
}

/// `tasks_archive`: archive a task, and first its live slices, with `project_archive_slice`'s rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveTask {
    /// The task.
    pub id: TaskId,
}

/// Deserializes a field that is present, so `null` becomes `Some(None)`; only an absent field
/// (through `#[serde(default)]`) stays `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

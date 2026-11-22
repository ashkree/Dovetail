//! Tools of the `project` suite: slice lifecycle and project registration.

use dovetail_api::model::{DriverKind, ReasoningEffort};
use dovetail_api::task::TaskId;
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::{ToolError, task_id, text};

/// Tools in the `project` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTool {
    /// `project_start_slice` (ADR 0004).
    StartSlice(StartSlice),
    /// `project_archive_slice` (task #9).
    ArchiveSlice(ArchiveSlice),
    /// `project_add` (ADR 0005).
    AddProject(AddProject),
}

impl ProjectTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            ProjectTool::StartSlice(_) => "project_start_slice",
            ProjectTool::ArchiveSlice(_) => "project_archive_slice",
            ProjectTool::AddProject(_) => "project_add",
        }
    }

    pub(crate) fn validate(&self, _role: Role) -> Result<(), ToolError> {
        match self {
            ProjectTool::StartSlice(s) => {
                task_id("task", s.task)?;
                text("name", &s.name)?;
                if let Some(model) = &s.model {
                    text("model", model)?;
                }
                text("brief", &s.brief)
            }
            ProjectTool::ArchiveSlice(a) => text("name", &a.name),
            ProjectTool::AddProject(_) => Ok(()),
        }
    }
}

/// The first word of a `project_archive_slice` / `tasks_archive` answer that did not fail.
pub mod archive_result {
    /// The slice is gone: session stopped, worktree and branch removed, its place freed.
    pub const ARCHIVED: &str = "archived";
    /// The worktree has uncommitted or unpushed work: the user is asked before it goes.
    pub const WAITING_FOR_USER: &str = "waiting_for_user";
}

/// `project_start_slice`: start a slice on an existing task (ADR 0004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartSlice {
    /// An existing task.
    pub task: TaskId,
    /// Slice name, used for the branch and worktree.
    pub name: String,
    /// The slice's first prompt.
    pub brief: String,
    /// The repo it works in, by name (ADR 0021). May be left out when the project has one;
    /// required with several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The model the session starts on: an alias (`haiku`, `sonnet`, `opus`) or a full model id.
    /// Left out, the slice starts on `sonnet`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The effort it starts at. Left out, Claude Code's default applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    /// The driver the slice runs on (`claude` by default). When set to `agy`, `model` names
    /// an agy family (`claude-opus-5-5`, `gemini-3.8-flash`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<DriverKind>,
}

/// `project_archive_slice`: archive a slice by name (task #9). Clean, pushed work goes at once; unsaved
/// work waits for the user's yes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSlice {
    /// The slice's name, as given to `project_start_slice`.
    pub name: String,
}

/// `project_add`: register the folder the coordinator runs in as a Dovetail project (ADR 0005).
/// Takes no arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddProject {}

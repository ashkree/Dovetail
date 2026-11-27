//! Errors of the core's actions. Each is shown to the user (`CommandFailed`) or to an agent (a
//! tool result) through its `Display`.

use crate::db::DbError;
pub use crate::sessions::SessionError;
use crate::worktree::{SUBAGENT_PREFIX, WorktreeError};

/// Why a slice could not start (`project_start_slice` answers `denied: <this>`).
#[derive(Debug, thiserror::Error)]
pub enum SliceError {
    /// The name is not usable as a directory and branch name.
    #[error("{0}: use letters, digits, `-`, `_` or `.`, not starting with `.` or `-`")]
    BadName(#[source] WorktreeError),
    /// The name has the subagent prefix.
    #[error(
        "slice names may not start with `{SUBAGENT_PREFIX}`: that prefix marks subagent \
         worktrees, which skip setup"
    )]
    SubagentPrefix,
    /// The name is the coordinator's.
    #[error("`coordinator` is taken")]
    Reserved,
    /// Another slice, running or queued, has the name.
    #[error("a slice named `{0}` already exists")]
    Taken(String),
    /// The task does not exist.
    #[error("there is no task #{0}; create it first with `tasks_create`")]
    NoTask(i64),
    /// The open project is a research project: it has no repo, so no slices.
    #[error(
        "this is a research project: it has no repo, so it runs no slices. Use research rounds \
         and subagents here; code work belongs in a code project"
    )]
    ResearchProject,
    /// The project has several repos and the request named none (ADR 0021).
    #[error("this project has several repos: name one with `repo` ({0})")]
    RepoRequired(String),
    /// No repo of the project has this name.
    #[error("there is no repo `{name}` in this project; its repos are: {known}")]
    UnknownRepo {
        /// The name asked for.
        name: String,
        /// The project's repo names, comma-separated.
        known: String,
    },
    /// The `model` asked for is not one the driver lists (task #42).
    #[error("there is no model `{model}`; the models are: {known}")]
    UnknownModel {
        /// The name asked for.
        model: String,
        /// The models the driver listed, comma-separated.
        known: String,
    },
    /// `project_archive_slice` named the coordinator.
    #[error("the coordinator cannot be archived")]
    ArchiveCoordinator,
    /// `project_archive_slice` named no open slice.
    #[error("there is no slice named {0} (unknown or already archived)")]
    NoSlice(String),
    /// The session did not start.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// Storage failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// Why a slice's worktree setup failed, as the hook (or the wait for it) reported it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SetupFailure(pub String);

/// Registering the open folder as a project (ADR 0005).
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// Storage failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

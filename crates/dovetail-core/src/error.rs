//! Errors of the core's actions. Each is shown to the user (`CommandFailed`) or to an agent (a
//! tool result) through its `Display`.

use dovetail_driver::DriverError;

use crate::db::DbError;
use crate::worktree::{SUBAGENT_PREFIX, WorktreeError};

/// Starting, resuming or sending to a session.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No such agent.
    #[error("no agent {0}")]
    NoAgent(String),
    /// The agent has no running session.
    #[error("{0} is not running")]
    NotRunning(String),
    /// A slice whose session never started cannot be resumed.
    #[error("slice {0} never started; archive it and start a new one")]
    NeverStarted(String),
    /// Subagents run inside their parent's session.
    #[error("subagents have no session of their own")]
    Subagent,
    /// The project's Claude profile cannot be used (ADR 0009).
    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),
    /// The launch files could not be written.
    #[error("writing launch settings: {0}")]
    LaunchFiles(#[source] std::io::Error),
    /// More images than one message may carry.
    #[error("at most {0} images per message")]
    TooManyImages(usize),
    /// A pasted image could not be read from its saved file.
    #[error("could not read image {path}: {source}")]
    ImageUnreadable {
        /// The saved file.
        path: std::path::PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// A saved image is over [`dovetail_api::MAX_ATTACHMENT_BYTES`].
    #[error("image {} is over {}", .0.display(), dovetail_api::attachment_limit_text())]
    ImageTooLarge(std::path::PathBuf),
    /// The session is taken over in a native terminal (M14): headless does not start.
    #[error(
        "{0} is taken over in a native terminal: write to it there; Dovetail resumes it \
         headless when that claude exits"
    )]
    TakenOver(String),
    /// The driver failed.
    #[error("{name}: {source}")]
    Driver {
        /// The agent's name.
        name: String,
        /// The cause.
        source: DriverError,
    },
}

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

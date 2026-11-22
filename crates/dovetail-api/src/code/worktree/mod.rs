//! The `worktree` feature: slice worktrees, with their git and pull-request state.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::WorktreeCommand;
pub use event::WorktreeEvent;

use crate::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Git and pull-request state of one slice's worktree, for the worktree view (`<leader>g`).
///
/// The core fills it in full on [`WorktreeCommand::Refresh`] (fetching the base and
/// asking `gh`), and refreshes the local git parts after a slice's turn finishes; the PR then
/// stays as last looked up. Each part is looked up on its own: one that could not be read is `None` (or
/// [`PrLookup::Unknown`]) and the rest still shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// The slice that owns the worktree.
    pub agent: AgentId,
    /// Worktree directory.
    pub path: PathBuf,
    /// The slice's branch (`dovetail/<name>`).
    pub branch: String,
    /// The branch it is compared with: the one checked out in the main checkout, normally
    /// `main`.
    pub base: String,
    /// Commits ahead of and behind the base; `None` if git could not tell.
    pub sync: Option<SyncState>,
    /// Uncommitted changes, Dovetail's own symlinks excluded; `None` if git could not tell.
    pub changes: Option<Changes>,
    /// Commits on the branch not pushed anywhere: ahead of its upstream, or, with no upstream,
    /// on no remote and not in the base. `None` if git could not tell.
    pub unpushed: Option<u32>,
    /// The branch's pull request, from `gh`.
    pub pr: PrLookup,
    /// The recipe's setup commands in this worktree.
    pub setup: WorktreeSetup,
    /// When this was gathered, in Unix milliseconds.
    pub checked_ms: i64,
}

/// Where a branch stands against its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// Commits on the branch that the base lacks.
    pub ahead: u32,
    /// Commits on the base that the branch lacks.
    pub behind: u32,
    /// Commits on `origin/<base>` that the branch lacks, as of the last fetch: non-zero means
    /// the remote base has moved since the branch last took it in. `None` without such a
    /// remote branch, or when the fetch on [`WorktreeCommand::Refresh`] failed.
    pub behind_origin: Option<u32>,
}

/// Counts of uncommitted entries in a worktree, as `git status` reports them. An entry both
/// staged and changed again counts in both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    /// Entries with changes in the index.
    pub staged: u32,
    /// Tracked entries changed in the working tree but not staged.
    pub unstaged: u32,
    /// Untracked entries.
    pub untracked: u32,
    /// Entries with merge conflicts.
    pub conflicted: u32,
}

impl Changes {
    /// True when nothing is uncommitted.
    pub fn is_clean(&self) -> bool {
        *self == Changes::default()
    }
}

/// What looking up a branch's pull request found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrLookup {
    /// `gh` is missing, failed or timed out, or has not been asked yet.
    Unknown {
        /// Why, in a few words.
        reason: String,
    },
    /// The branch has no pull request.
    None,
    /// The branch's pull request: an open one if there is one, else the newest.
    Found(PullRequest),
}

/// A pull request on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// PR number.
    pub number: u32,
    /// Open, merged or closed.
    pub state: PrState,
    /// Its web address.
    pub url: String,
}

/// State of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    /// Open (drafts included).
    Open,
    /// Merged.
    Merged,
    /// Closed without merging.
    Closed,
}

/// The recipe's setup commands in one worktree, as the `WorktreeCreate` hook recorded them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeSetup {
    /// Nothing recorded: the recipe has no setup commands.
    None,
    /// The commands are running.
    Running,
    /// Every command succeeded.
    Done,
    /// A command failed, or the run was interrupted.
    Failed {
        /// What went wrong, if recorded.
        error: Option<String>,
    },
}

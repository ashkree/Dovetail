//! The `project` feature: registered projects, their repos and account.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::ProjectCommand;
pub use event::ProjectEvent;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Identifies a registered project: the slug of its repo folder, numbered on collision.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(pub String);

/// Identifies one repo of a project: its row id in the registry's `repos` table (ADR 0021). A
/// slice stores it, so renaming the repo does not orphan the slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoId(pub i64);

/// A registered project (ADR 0021): a name with zero or more repos and one account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    /// Project id.
    pub id: ProjectId,
    /// Display name.
    pub name: String,
    /// The first repo's main checkout, or, for a project with no repos, Dovetail's own state
    /// folder for it. Multi-repo clients read `repos`.
    pub repo_path: PathBuf,
    /// `Research` when the project has no repos. A peer that does not send it means a code
    /// project.
    #[serde(default)]
    pub kind: ProjectKind,
    /// Its repos, in the order they were added. Empty: a research-only project.
    #[serde(default)]
    pub repos: Vec<RepoSummary>,
    /// The Claude account (a profile name from `config.toml`) its sessions run under; `None`
    /// for the recipe's `profile`, else the default Claude home (ADR 0009).
    #[serde(default)]
    pub account: Option<String>,
    /// When it was last opened, in Unix milliseconds.
    #[serde(default)]
    pub last_opened_ms: Option<i64>,
    /// What Home shows about it, read from its `state.db`.
    #[serde(default)]
    pub counts: ProjectCounts,
}

/// One repo of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSummary {
    /// Registry id.
    pub id: RepoId,
    /// Lower kebab case: what agents and the user call it (`shopfront-api`).
    pub name: String,
    /// The main checkout (or, for a folder project without git, the folder).
    pub path: PathBuf,
}

/// A project's counts as Home lists them. All zero when its `state.db` could not be read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCounts {
    /// Open efforts, not counting Unsorted.
    pub efforts: u32,
    /// Open efforts with an open slice on one of their tasks.
    pub running: u32,
    /// Items waiting on the user: open inbox items and claimed criteria.
    pub needs_you: u32,
}

/// What a project is for. A research project has no repo: no worktrees, no slices, no git; its
/// sessions run in `~/.local/share/dovetail/projects/<id>/`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    /// A repo or folder of the user's: the default.
    #[default]
    Code,
    /// Macro research only (glossary "Research-only project").
    Research,
}

impl ProjectKind {
    /// The name stored in the registry.
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Code => "code",
            ProjectKind::Research => "research",
        }
    }

    /// The kind stored as `name`, if it is one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "code" => Some(ProjectKind::Code),
            "research" => Some(ProjectKind::Research),
            _ => None,
        }
    }
}

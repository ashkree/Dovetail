//! The `project` feature's commands.

use crate::project::{ProjectId, RepoId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Projects: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectCommand {
    /// Switch to another registered project (B24): the core answers with
    /// [`super::ProjectEvent::Opening`] and quits, ending this project's sessions, and the binary starts
    /// again on the other one.
    Open {
        /// The project.
        id: ProjectId,
    },
    /// Create and register a research project named `name` (`:project new <name> --research`).
    /// It has no repo; its folder is made under Dovetail's state folder. The core answers with a
    /// fresh snapshot, whose `projects` lists it.
    NewResearch {
        /// Display name; the id is its slug.
        name: String,
    },
    /// Create a project (ADR 0021): a name, an optional account and an optional first repo.
    /// Answered with [`super::ProjectEvent::Upserted`]. Home and the core both take it.
    Create {
        /// Display name; the id is its slug, numbered when taken.
        name: String,
        /// A profile name from `config.toml`; `None` for the default Claude home.
        #[serde(default)]
        account: Option<String>,
        /// A folder inside a git repo, added as its first repo.
        #[serde(default)]
        repo: Option<PathBuf>,
    },
    /// Rename a project. Its id, and so its folders, stay.
    Rename {
        /// The project.
        id: ProjectId,
        /// The new display name.
        name: String,
    },
    /// Set the Claude account a project's sessions run under; `None` goes back to the recipe's
    /// `profile`, else the default. Running sessions keep theirs until they next start.
    SetAccount {
        /// The project.
        id: ProjectId,
        /// A profile name from `config.toml`.
        account: Option<String>,
    },
    /// Add a repo to a project, by a path inside it. The open project's coordinator gets it at
    /// its next start, at once when it is idle.
    AddRepo {
        /// The project.
        project: ProjectId,
        /// A folder inside the repo.
        path: PathBuf,
        /// What agents call it; `None` for the slug of the folder's name.
        #[serde(default)]
        name: Option<String>,
    },
    /// Remove a repo from a project. Refused while a slice works in it. Nothing on disk changes.
    RemoveRepo {
        /// The project.
        project: ProjectId,
        /// The repo.
        repo: RepoId,
    },
    /// Register a repo as a project.
    Register {
        /// Path inside the repo.
        path: PathBuf,
    },
    /// Uncomment `profile = "<profile>"` in the starter recipe [`super::ProjectEvent::RecipeStarted`] named
    /// (task #31). Fails when the line is gone (the file was edited) or config.toml lacks it.
    SetRecipeProfile {
        /// The open project.
        project: ProjectId,
        /// A profile name from `config.toml`.
        profile: String,
    },
}

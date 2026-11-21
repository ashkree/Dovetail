//! The `project` feature's events.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::project::{ProjectId, ProjectSummary};

/// A change to the client's snapshot, or an answer, in the feature `project`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEvent {
    /// The user picked another project (B24): the core is quitting so the binary can start on
    /// `id`, and the UI quits too, without asking. Changes nothing in the snapshot.
    Opening {
        /// The project to open next.
        id: ProjectId,
    },
    /// A project was created or changed: renamed, its account set, a repo added or removed
    /// (ADR 0021). Replaces it in `projects`, or adds it; replaces `project` too when it is the
    /// open one.
    Upserted(ProjectSummary),
    /// [`ProjectCommand::Register`] wrote a starter recipe for the project, which had none
    /// (task #31). The UI offers `profiles` (config.toml's, empty for none) and answers with
    /// [`ProjectCommand::SetRecipeProfile`]. Changes nothing in the snapshot.
    RecipeStarted {
        /// The project just added.
        project: ProjectId,
        /// Its display name.
        name: String,
        /// The recipe file written.
        path: PathBuf,
        /// The profile names in `config.toml`, sorted.
        profiles: Vec<String>,
    },
}

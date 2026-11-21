//! The `library` feature's commands.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::library::BrowseWhat;
use crate::project::ProjectId;

/// The library: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryCommand {
    /// List the project's read-only docs (reading mode's Docs · ADRs). The core answers with
    /// [`LibraryEvent::DocsListed`].
    ListDocs,
    /// Fetch one doc listed by [`LibraryEvent::DocsListed`]. The core answers with
    /// [`LibraryEvent::DocLoaded`], or [`crate::app::AppEvent::CommandFailed`] for a path outside the docs.
    LoadDoc {
        /// As [`DocEntry::path`] names it.
        path: PathBuf,
    },
    /// Load another registered project's library read-only (`<leader>fr` in reading mode, ADR
    /// 0018 decided 2026-11-11 later): its research, decision log and docs, from its
    /// `state.db` opened read-only and its files. Nothing of that project is written and the
    /// open project does not change. The core answers with [`LibraryEvent::ProjectBrowsed`], or
    /// [`crate::app::AppEvent::CommandFailed`].
    BrowseProject {
        /// The project.
        id: ProjectId,
    },
    /// Fetch one document of a browsed project, read-only. The core answers with
    /// [`LibraryEvent::BrowsedLoaded`], or [`crate::app::AppEvent::CommandFailed`] when there is no such thing.
    LoadBrowsed {
        /// The project.
        project: ProjectId,
        /// The document.
        what: BrowseWhat,
    },
}

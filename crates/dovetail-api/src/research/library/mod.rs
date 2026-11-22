//! The `library` feature: reading mode's library, project docs and other projects' research.

mod command;
mod event;
pub use command::LibraryCommand;
pub use event::LibraryEvent;

use crate::decision::Decision;
use crate::project::{ProjectId, ProjectSummary};
use crate::research::{ResearchListing, ResearchSummary, ResearchWhat};
use crate::task::{Task, TaskId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// An image a document shows, found by the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocImage {
    /// The image's address as the markdown writes it.
    pub src: String,
    /// The file, absolute.
    pub path: PathBuf,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// A read-only markdown doc of the project's repo (reading mode's Docs · ADRs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEntry {
    /// Relative to the repo's `docs/` folder: `research.md`, `adr/0018-reading-mode-layout.md`.
    pub path: PathBuf,
    /// Its first `# ` heading, else the file name.
    pub title: String,
}

/// Another project's library, loaded read-only for reading mode's `<leader>fr` (ADR 0018,
/// decided 2026-11-11 later), in answer to [`LibraryCommand::BrowseProject`]. A copy taken when
/// asked: it is not kept up to date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowsedProject {
    /// The project browsed.
    pub project: ProjectSummary,
    /// Its tasks, archived ones included (for research task titles and decision tasks).
    pub tasks: Vec<Task>,
    /// Its decision log, oldest first.
    pub decisions: Vec<Decision>,
    /// Each task's research, for tasks that have any.
    pub research: Vec<ResearchSummary>,
    /// Each research task's findings and sources.
    pub listings: Vec<BrowsedListing>,
    /// Markdown under its repo's `docs/`, sorted by path; empty for a research project.
    pub docs: Vec<DocEntry>,
}

/// One task's findings and sources in a [`BrowsedProject`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowsedListing {
    /// The task.
    pub task: TaskId,
    /// Its findings and sources.
    pub listing: ResearchListing,
}

/// A document of a browsed project, for [`LibraryCommand::LoadBrowsed`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowseWhat {
    /// A research document (not [`ResearchWhat::Listing`]: listings come with the project).
    Research {
        /// The task, in that project.
        task: TaskId,
        /// Which document.
        what: ResearchWhat,
    },
    /// A repo doc, as [`DocEntry::path`] names it.
    Doc(PathBuf),
}

/// Names a browsed project's document in [`LibraryEvent::BrowsedLoaded`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowsedDoc {
    /// The project.
    pub project: ProjectId,
    /// The document.
    pub what: BrowseWhat,
}

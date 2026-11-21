//! The `library` feature's events.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::library::{BrowsedDoc, BrowsedProject, DocEntry};
use crate::research::ResearchDocument;

/// A change to the client's snapshot, or an answer, in the feature `library`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryEvent {
    /// The project's read-only docs, in answer to [`LibraryCommand::ListDocs`]. Changes nothing in the
    /// snapshot.
    DocsListed {
        /// Markdown files under the repo's `docs/`, sorted by path; empty for a project with no
        /// repo.
        docs: Vec<DocEntry>,
    },
    /// One doc's text, in answer to [`LibraryCommand::LoadDoc`]. Changes nothing in the snapshot.
    DocLoaded {
        /// The doc, as [`DocEntry::path`] names it.
        path: PathBuf,
        /// Its text; HTML is left as it is, for the UI to convert.
        document: ResearchDocument,
    },
    /// Another project's library, in answer to [`LibraryCommand::BrowseProject`]. Changes nothing in
    /// the snapshot.
    ProjectBrowsed(Box<BrowsedProject>),
    /// A browsed project's document, in answer to [`LibraryCommand::LoadBrowsed`]. Changes nothing in
    /// the snapshot.
    BrowsedLoaded {
        /// Which project's document.
        doc: BrowsedDoc,
        /// Its text; HTML is left as it is, for the UI to convert.
        document: ResearchDocument,
    },
}

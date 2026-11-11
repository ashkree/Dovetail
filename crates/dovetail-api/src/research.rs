//! Research capture (R1, `docs/research.md` §3–§5, ADR 0015): rounds, findings and sources as
//! the UI sees them.
//!
//! The snapshot carries one small [`ResearchSummary`] per task that has research; everything
//! else is fetched with [`crate::Command::LoadResearch`] and arrives as
//! [`crate::Event::ResearchLoaded`], as maps do.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{AgentId, TaskId};

/// Identifies a finding: the `findings.id` row id. Shown to agents as `F<id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FindingId(pub i64);

/// Identifies a source: the `sources.id` row id. Shown to agents as `S<id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub i64);

/// One task's research as the library lists it, without content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchSummary {
    /// The task (the question).
    pub task: TaskId,
    /// The task's research folder (`research/<task-id>-<slug>/` under the project's research
    /// root); it exists once a round or finding does.
    pub dir: PathBuf,
    /// Rounds, oldest first.
    pub rounds: Vec<RoundSummary>,
    /// Findings of the task, rounds and quick ones together.
    pub findings: u32,
    /// Quick findings: those filed with no round open (`docs/research.md` §5.7).
    pub quick_findings: u32,
    /// Distinct sources the task's findings cite.
    pub sources: u32,
    /// When the latest `report.md` was written, in Unix milliseconds; `None` before the first.
    pub report_ms: Option<i64>,
}

impl ResearchSummary {
    /// The round still open, if any.
    pub fn open_round(&self) -> Option<&RoundSummary> {
        self.rounds.iter().find(|r| r.closed_ms.is_none())
    }
}

/// One round: a fan-out of subagents under one brief.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundSummary {
    /// 1-based within the task.
    pub n: u32,
    /// The user's criteria, as given when it was opened.
    pub brief: String,
    /// The round's folder, `rounds/01-2026-11-10`, relative to [`ResearchSummary::dir`].
    pub dir: PathBuf,
    /// When it was opened, in Unix milliseconds.
    pub opened_ms: i64,
    /// When it was closed; `None` while open.
    pub closed_ms: Option<i64>,
    /// Findings filed under it.
    pub findings: u32,
}

/// What [`crate::Command::LoadResearch`] asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchWhat {
    /// Every finding and source of the task, without bodies.
    Listing,
    /// A round's `brief.md`.
    Brief {
        /// The round's number within the task.
        round: u32,
    },
    /// A finding's file.
    Finding(FindingId),
    /// A source's cached copy.
    Source(SourceId),
    /// The latest `report.md`.
    Report,
}

/// The answer to [`crate::Command::LoadResearch`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchContent {
    /// Answer to [`ResearchWhat::Listing`].
    Listing(ResearchListing),
    /// Answer to every other [`ResearchWhat`]: one markdown document.
    Document(ResearchDocument),
}

/// Every finding and source of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchListing {
    /// Findings, oldest first.
    pub findings: Vec<FindingSummary>,
    /// Sources the findings cite, by id.
    pub sources: Vec<SourceSummary>,
}

/// A finding without its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingSummary {
    /// Its id (`F<id>` in reports).
    pub id: FindingId,
    /// The round it was filed under; `None` for a quick finding.
    pub round: Option<u32>,
    /// 1-based within its round (or among the task's quick findings).
    pub n: u32,
    /// The subagent that wrote it.
    pub agent: AgentId,
    /// Slug used in the file name, from the subagent's description.
    pub label: String,
    /// The file, absolute.
    pub path: PathBuf,
    /// When it was filed, in Unix milliseconds.
    pub created_ms: i64,
    /// What it fetched or read, in the order it did.
    pub sources: Vec<SourceId>,
}

/// How a source was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A web page: fetched (`WebFetch`) or listed by a search (`WebSearch`).
    Web,
    /// A local file (`Read`, or a `Grep`/`Glob` over a path).
    File,
    /// A web search itself: its query and the result text, kept as untrusted text.
    Search,
}

impl SourceKind {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::File => "file",
            Self::Search => "search",
        }
    }

    /// Reads the stored spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "web" => Self::Web,
            "file" => Self::File,
            "search" => Self::Search,
            _ => return None,
        })
    }
}

/// A source without its cached copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSummary {
    /// Its id (`S<id>` in reports).
    pub id: SourceId,
    /// Web page, file or search.
    pub kind: SourceKind,
    /// URL, absolute file path, or search query.
    pub locator: String,
    /// The tool that reached it: `WebFetch`, `WebSearch`, `Read`, `Grep`, `Glob`.
    pub tool: String,
    /// The cached copy (what the tool returned to the agent, not an archive of the page),
    /// absolute; `None` for tools that are listed but not cached.
    pub path: Option<PathBuf>,
    /// The tool call failed; the cached copy is its error.
    pub is_error: bool,
    /// When it was first fetched, in Unix milliseconds.
    pub fetched_ms: i64,
}

/// One research document, as markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchDocument {
    /// Its file, absolute (for the editor hand-off); `None` for a source that is listed but not
    /// cached (`Grep`, `Glob`).
    pub path: Option<PathBuf>,
    /// The file's content, from the database (the files are a readable copy).
    pub markdown: String,
}

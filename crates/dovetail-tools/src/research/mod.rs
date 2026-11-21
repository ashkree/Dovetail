//! Tools of the `research` suite: rounds, reports, proposals, and stored documents.

use dovetail_api::decision::Alternative;
use dovetail_api::research::{
    Evidence, FindingId, ProposalKind, ResearchKind, ResearchWhat, SourceId,
};
use dovetail_api::task::TaskId;
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::{ToolError, task_id, text};

/// Tools in the `research` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchTool {
    /// `research_start_round` (ADR 0015).
    StartRound(ResearchStartRound),
    /// `research_write_report` (ADR 0015).
    WriteReport(ResearchWriteReport),
    /// `research_propose` (ADR 0015).
    Propose(ResearchPropose),
    /// `research_get`: read a stored research document.
    Get(ResearchGet),
}

impl ResearchTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            ResearchTool::StartRound(_) => "research_start_round",
            ResearchTool::WriteReport(_) => "research_write_report",
            ResearchTool::Propose(_) => "research_propose",
            ResearchTool::Get(_) => "research_get",
        }
    }

    pub(crate) fn validate(&self, _role: Role) -> Result<(), ToolError> {
        match self {
            ResearchTool::StartRound(r) => {
                task_id("task", r.task)?;
                text("brief", &r.brief)
            }
            ResearchTool::WriteReport(r) => {
                task_id("task", r.task)?;
                text("body", &r.body)
            }
            ResearchTool::Propose(p) => {
                task_id("task", p.task)?;
                text("text", &p.text)?;
                p.parsed_evidence()?;
                if p.kind == ProposalKind::Decision && p.evidence.is_empty() {
                    return Err(ToolError::DecisionNeedsEvidence);
                }
                Ok(())
            }
            ResearchTool::Get(g) => {
                task_id("task", g.task)?;
                g.target().map(|_| ())
            }
        }
    }
}

/// `research_start_round`: open the next round of a research task under the user's criteria
/// (`docs/research.md` §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchStartRound {
    /// An existing task (the question).
    pub task: TaskId,
    /// The user's criteria for this round, verbatim where possible.
    pub brief: String,
    /// Whether the task asks a `question` or makes a `decision`; sets the task's research kind
    /// (a decision is held to the decision template). Omit to leave it as it is.
    #[serde(default)]
    pub kind: Option<ResearchKind>,
}

/// `research_write_report`: store the report of a task, closing its open round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchWriteReport {
    /// The research task.
    pub task: TaskId,
    /// The whole report in markdown, citing `[F3]` findings and `[S12]` sources.
    pub body: String,
}

/// `research_propose`: an idea or decision for the user to accept from Needs you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchPropose {
    /// The research task.
    pub task: TaskId,
    /// `idea` or `decision`.
    pub kind: ProposalKind,
    /// The idea, or the decision in one line.
    pub text: String,
    /// Finding and source ids, like `F3`, `S12`. Required for a decision.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Why, if any.
    pub why: Option<String>,
    /// Options considered and rejected.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
}

impl ResearchPropose {
    /// The `evidence` strings as ids, without duplicates, in the order given.
    pub fn parsed_evidence(&self) -> Result<Vec<Evidence>, ToolError> {
        let mut out = Vec::new();
        for text in &self.evidence {
            let e = Evidence::parse(text).ok_or_else(|| ToolError::BadEvidence(text.clone()))?;
            if !out.contains(&e) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

/// `research_get`: read one stored research document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchGet {
    /// The research task.
    pub task: TaskId,
    /// `report`, `round:N` (its brief), `finding:N` or `source:N` (the `F`/`S` ids).
    pub what: String,
}

impl ResearchGet {
    /// What `what` asks for.
    pub fn target(&self) -> Result<ResearchWhat, ToolError> {
        let bad = || ToolError::BadWhat(self.what.clone());
        let what = self.what.trim();
        if what == "report" {
            return Ok(ResearchWhat::Report);
        }
        let (kind, n) = what.split_once(':').ok_or_else(bad)?;
        let n = n.trim();
        // `finding:F3` and `finding:3` both work: the ids are the ones the report cites.
        let n = n.trim_start_matches(['F', 'S']);
        let n: i64 = n.parse().ok().filter(|n| *n > 0).ok_or_else(bad)?;
        match kind.trim() {
            "round" => Ok(ResearchWhat::Brief {
                round: u32::try_from(n).map_err(|_| bad())?,
            }),
            "finding" => Ok(ResearchWhat::Finding(FindingId(n))),
            "source" => Ok(ResearchWhat::Source(SourceId(n))),
            _ => Err(bad()),
        }
    }
}

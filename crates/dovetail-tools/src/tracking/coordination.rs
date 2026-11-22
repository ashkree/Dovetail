//! Tools of the `coordination` suite: slice reports and criterion claims.

use dovetail_api::effort::CriterionId;
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::{ToolError, text};

/// Tools in the `coordination` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationTool {
    /// `coordination_report` (ADR 0022): a slice's report to the coordinator.
    Report(Report),
    /// `coordination_claim_criterion` (decided 2026-11-16): say an acceptance criterion is met,
    /// with evidence; only the user approves it.
    ClaimCriterion(ClaimCriterion),
}

impl CoordinationTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            CoordinationTool::Report(_) => "coordination_report",
            CoordinationTool::ClaimCriterion(_) => "coordination_claim_criterion",
        }
    }

    pub(crate) fn validate(&self, role: Role) -> Result<(), ToolError> {
        match self {
            CoordinationTool::Report(r) => {
                if role != Role::Slice {
                    return Err(ToolError::ReportFromNonSlice);
                }
                text("summary", &r.summary)
            }
            CoordinationTool::ClaimCriterion(c) => {
                if !matches!(role, Role::Slice | Role::Coordinator) {
                    return Err(ToolError::ClaimFromOtherRole);
                }
                if c.criterion.0 <= 0 {
                    return Err(ToolError::NotACriterionId);
                }
                text("evidence", &c.evidence)
            }
        }
    }
}

/// `coordination_report`: what a slice tells the coordinator about its task (ADR 0022). It
/// goes through Dovetail, never by session name, so it cannot reach another project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Where the work stands.
    pub state: ReportState,
    /// What happened, in a line or two.
    pub summary: String,
    /// The pull request the report is about, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
}

/// Where a slice says its work stands, in a [`Report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportState {
    /// Still working; something worth knowing happened.
    Progress,
    /// The work is ready for review.
    Done,
    /// It cannot go on without the coordinator.
    Blocked,
}

impl ReportState {
    /// The word shown in the report line.
    pub fn as_str(self) -> &'static str {
        match self {
            ReportState::Progress => "progress",
            ReportState::Done => "done",
            ReportState::Blocked => "blocked",
        }
    }
}

/// `coordination_claim_criterion`: an agent says an acceptance criterion is met and why. The
/// criterion becomes claimed and waits in the user's inbox; only the user can make it met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimCriterion {
    /// The criterion's id.
    pub criterion: CriterionId,
    /// What shows it is met: a pull request, a test, a finding.
    pub evidence: String,
}

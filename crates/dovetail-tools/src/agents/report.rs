//! Tools of the `core` suite (the `report` feature): questions, status, and the decision log.

use dovetail_api::decision::{Alternative, DecisionId};
use dovetail_api::task::TaskId;
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::{ToolError, task_id, text};

/// Tools in the `core` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportTool {
    /// `core_ask_user`.
    AskUser(AskUser),
    /// `core_report_status`.
    ReportStatus(ReportStatus),
    /// `core_log_decision`.
    LogDecision(LogDecision),
}

impl ReportTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            ReportTool::AskUser(_) => "core_ask_user",
            ReportTool::ReportStatus(_) => "core_report_status",
            ReportTool::LogDecision(_) => "core_log_decision",
        }
    }

    pub(crate) fn validate(&self, role: Role) -> Result<(), ToolError> {
        match self {
            ReportTool::AskUser(a) => text("question", &a.question),
            ReportTool::ReportStatus(r) => text("status", &r.status),
            ReportTool::LogDecision(d) => {
                match (role, d.task) {
                    // A side chat has no task to default to, like the coordinator.
                    (Role::Coordinator | Role::SideChat | Role::Ask, None) => {
                        return Err(ToolError::DecisionNeedsTask);
                    }
                    (_, Some(task)) => task_id("task", task)?,
                    (Role::Slice, None) => {}
                }
                text("decision", &d.decision)
            }
        }
    }
}

/// `core_ask_user`: a question for the user's inbox. It never blocks: the call returns at once and the
/// answer arrives as a user message (ADR 0007). An old caller's `blocking` field is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskUser {
    /// The question, self-contained.
    pub question: String,
}

/// `core_report_status`: the agent's one-line status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportStatus {
    /// One line; the core cuts it at 120 characters.
    pub status: String,
}

/// `core_log_decision`: an entry in a task's decision log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogDecision {
    /// The task; required for the coordinator, a slice's own task by default.
    pub task: Option<TaskId>,
    /// The decision, one line.
    pub decision: String,
    /// The reason.
    pub why: Option<String>,
    /// Options considered and rejected.
    #[serde(default)]
    pub alternatives: Vec<Alternative>,
    /// The earlier entry it replaces.
    pub supersedes: Option<DecisionId>,
}

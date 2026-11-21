//! Validation and argument-parsing errors for MCP tool calls.

use dovetail_api::task::TaskId;

/// Why a tool call is refused before it reaches a handler. `Display` is the message for the
/// agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// No tool has this name.
    Unknown(String),
    /// The arguments do not fit the tool's schema.
    BadArguments {
        /// The tool.
        tool: String,
        /// What serde said.
        message: String,
    },
    /// A text field is empty.
    Empty(&'static str),
    /// A task id is not positive.
    NotATaskId(&'static str),
    /// `tasks_create` asked for Now (ADR 0002).
    BucketNow,
    /// The coordinator logged a decision without naming its task.
    DecisionNeedsTask,
    /// `tasks_update` asked for `archived`, which is `tasks_archive`'s job.
    UpdateToArchived,
    /// `tasks_update` asked for a status: Dovetail sets task status itself (ADR 0024).
    StatusIsSetByDovetail,
    /// `coordination_report` from a session that is not a slice.
    ReportFromNonSlice,
    /// `coordination_claim_criterion` from a session that is neither a slice nor the
    /// coordinator.
    ClaimFromOtherRole,
    /// A criterion id is not positive.
    NotACriterionId,
    /// `tasks_update` named no field to change.
    NothingToUpdate,
    /// `research_propose` of a decision with no evidence (ADR 0015).
    DecisionNeedsEvidence,
    /// An `evidence` entry is not `F<id>` or `S<id>`.
    BadEvidence(String),
    /// A card id is not positive.
    NotACardId(&'static str),
    /// `canvas_add_card` of an idea or question with no text, or of a reference with no id, or
    /// of an idea or question that also names a reference.
    BadCard(&'static str),
    /// `design_declare_variants` from a session that is not a slice.
    DeclareFromNonSlice,
    /// `design_declare_variants` with a list that cannot be used; the text says what is wrong.
    BadVariants(String),
    /// `research_get`'s `what` is none of `report`, `round:N`, `finding:N`, `source:N`.
    BadWhat(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Unknown(name) => write!(f, "unknown tool `{name}`"),
            ToolError::BadArguments { tool, message } => {
                write!(f, "bad arguments for `{tool}`: {message}")
            }
            ToolError::Empty(field) => write!(f, "`{field}` must be a non-empty string"),
            ToolError::NotATaskId(field) => {
                write!(f, "`{field}` must be a task id (a positive integer)")
            }
            ToolError::BucketNow => f.write_str(
                "`bucket` must be `next` or `later`: the user decides what is current (ADR 0002)",
            ),
            ToolError::DecisionNeedsTask => {
                f.write_str("the coordinator must name the `task` this decision belongs to")
            }
            ToolError::UpdateToArchived => {
                f.write_str("`status` cannot be set here: archive a task with `tasks_archive`")
            }
            ToolError::StatusIsSetByDovetail => f.write_str(
                "`status` cannot be set: Dovetail moves a task to in progress when its slice \
                 starts, to review when its pull request opens or its slice reports done, and \
                 to done when the pull request is merged. For work with no pull request, ask \
                 the user to mark it done",
            ),
            ToolError::ReportFromNonSlice => {
                f.write_str("`coordination_report` is for slices: it reports to the coordinator")
            }
            ToolError::ClaimFromOtherRole => {
                f.write_str("`coordination_claim_criterion` is for slices and the coordinator")
            }
            ToolError::NotACriterionId => {
                f.write_str("`criterion` must be a criterion id (a positive integer)")
            }
            ToolError::NothingToUpdate => f.write_str(
                "give at least one field to change: `bucket`, `p_level`, `title` or `notes`",
            ),
            ToolError::DecisionNeedsEvidence => f.write_str(
                "a decision needs `evidence`: the finding and source ids it rests on, like \
                 [\"F3\", \"S12\"]. Propose it as an `idea` if nothing backs it yet",
            ),
            ToolError::BadEvidence(text) => write!(
                f,
                "evidence `{text}` is not a finding or source id: write F<id> or S<id>, like                  F3 or S12"
            ),
            ToolError::NotACardId(field) => {
                write!(f, "`{field}` must be a card id (a positive integer)")
            }
            ToolError::BadCard(why) => f.write_str(why),
            ToolError::DeclareFromNonSlice => f.write_str(
                "`design_declare_variants` is for slices: they declare the variants of the page they build",
            ),
            ToolError::BadVariants(why) => write!(f, "bad variants: {why}"),
            ToolError::BadWhat(text) => write!(
                f,
                "`what` must be `report`, `round:N`, `finding:N` or `source:N`, not `{text}`"
            ),
        }
    }
}

impl std::error::Error for ToolError {}

pub(crate) fn text(field: &'static str, value: &str) -> Result<(), ToolError> {
    if value.trim().is_empty() {
        Err(ToolError::Empty(field))
    } else {
        Ok(())
    }
}

pub(crate) fn task_id(field: &'static str, value: TaskId) -> Result<(), ToolError> {
    if value.0 > 0 {
        Ok(())
    } else {
        Err(ToolError::NotATaskId(field))
    }
}

#![doc = include_str!("../ARCHITECTURE.md")]
#![warn(missing_docs)]

pub mod agents;
pub mod catalog;
pub mod env;
pub mod error;
pub mod research;
pub mod socket;
pub mod tracking;
pub mod visual;

// Short paths: every feature module is also reachable at the crate root, so callers write
// `dovetail_tools::task::ListTasks` as well as `dovetail_tools::tracking::task::ListTasks`.
pub use agents::report;
pub use tracking::{coordination, project, task};
pub use visual::{canvas, design, map};

pub use catalog::{
    Role, Suite, TOOLS, ToolSpec, deprecated_alias, deprecation_note, tool_spec, tools_in,
};
pub use error::ToolError;
pub use socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};

use serde::{Deserialize, Serialize};

use canvas::CanvasTool;
use coordination::CoordinationTool;
use design::DesignTool;
use map::MapTool;
use project::ProjectTool;
use report::ReportTool;
use research::ResearchTool;
use task::TaskTool;

/// One tool call: the suite feature and its tool arguments.
///
/// On the socket wire, `Tool` serializes as a nested externally-tagged object such as
/// `{"task": {"list": {..}}}`. [`Tool::parse`] accepts both current MCP tool names and the
/// deprecated ADR 0016 aliases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    /// Tools in the `core` suite.
    Report(ReportTool),
    /// Tools in the `tasks` suite.
    Task(TaskTool),
    /// Tools in the `project` suite.
    Project(ProjectTool),
    /// Tools in the `coordination` suite.
    Coordination(CoordinationTool),
    /// Tools in the `research` suite.
    Research(ResearchTool),
    /// Tools in the `maps` suite.
    Map(MapTool),
    /// Tools in the `canvas` suite.
    Canvas(CanvasTool),
    /// Tools in the `design` suite.
    Design(DesignTool),
}

impl Tool {
    /// The tool's name, without the server prefix.
    pub fn name(&self) -> &'static str {
        match self {
            Tool::Report(t) => t.name(),
            Tool::Task(t) => t.name(),
            Tool::Project(t) => t.name(),
            Tool::Coordination(t) => t.name(),
            Tool::Research(t) => t.name(),
            Tool::Map(t) => t.name(),
            Tool::Canvas(t) => t.name(),
            Tool::Design(t) => t.name(),
        }
    }

    /// Parses MCP `tools/call` arguments for the tool `name` (without the server prefix).
    /// An old name (ADR 0016) parses as its new one.
    pub fn parse(name: &str, arguments: serde_json::Value) -> Result<Tool, ToolError> {
        let Some(spec) = tool_spec(name) else {
            return Err(ToolError::Unknown(name.to_owned()));
        };
        fn args<T: serde::de::DeserializeOwned>(
            name: &str,
            value: serde_json::Value,
        ) -> Result<T, ToolError> {
            serde_json::from_value(value).map_err(|e| ToolError::BadArguments {
                tool: name.to_owned(),
                message: e.to_string(),
            })
        }
        let tool = match spec.name {
            "core_ask_user" => Tool::Report(ReportTool::AskUser(args(name, arguments)?)),
            "core_report_status" => Tool::Report(ReportTool::ReportStatus(args(name, arguments)?)),
            "core_log_decision" => Tool::Report(ReportTool::LogDecision(args(name, arguments)?)),
            "tasks_create" => Tool::Task(TaskTool::Create(args(name, arguments)?)),
            "tasks_list" => Tool::Task(TaskTool::List(args(name, arguments)?)),
            "tasks_get" => Tool::Task(TaskTool::Get(args(name, arguments)?)),
            "tasks_update" => Tool::Task(TaskTool::Update(args(name, arguments)?)),
            "tasks_archive" => Tool::Task(TaskTool::Archive(args(name, arguments)?)),
            "project_start_slice" => Tool::Project(ProjectTool::StartSlice(args(name, arguments)?)),
            "project_archive_slice" => {
                Tool::Project(ProjectTool::ArchiveSlice(args(name, arguments)?))
            }
            "project_add" => Tool::Project(ProjectTool::AddProject(args(name, arguments)?)),
            "coordination_report" => {
                Tool::Coordination(CoordinationTool::Report(args(name, arguments)?))
            }
            "coordination_claim_criterion" => {
                Tool::Coordination(CoordinationTool::ClaimCriterion(args(name, arguments)?))
            }
            "research_start_round" => {
                Tool::Research(ResearchTool::StartRound(args(name, arguments)?))
            }
            "research_write_report" => {
                Tool::Research(ResearchTool::WriteReport(args(name, arguments)?))
            }
            "research_propose" => Tool::Research(ResearchTool::Propose(args(name, arguments)?)),
            "research_get" => Tool::Research(ResearchTool::Get(args(name, arguments)?)),
            "maps_draw" => Tool::Map(MapTool::Draw(args(name, arguments)?)),
            "canvas_add_block" => Tool::Canvas(CanvasTool::AddBlock(args(name, arguments)?)),
            "canvas_add_card" => Tool::Canvas(CanvasTool::AddCard(args(name, arguments)?)),
            "canvas_link_cards" => Tool::Canvas(CanvasTool::LinkCards(args(name, arguments)?)),
            "design_declare_variants" => {
                Tool::Design(DesignTool::DeclareVariants(args(name, arguments)?))
            }
            other => return Err(ToolError::Unknown(other.to_owned())),
        };
        Ok(tool)
    }

    /// Whether a session with `suites` may call the tool named `name` (current or old name).
    pub fn allowed(name: &str, suites: &[Suite]) -> bool {
        tool_spec(name).is_some_and(|t| suites.contains(&t.suite))
    }

    /// Checks what serde cannot: non-empty text, positive ids, the bucket rule, and that the
    /// coordinator names the task of a decision.
    pub fn validate(&self, role: Role) -> Result<(), ToolError> {
        match self {
            Tool::Report(t) => t.validate(role),
            Tool::Task(t) => t.validate(role),
            Tool::Project(t) => t.validate(role),
            Tool::Coordination(t) => t.validate(role),
            Tool::Research(t) => t.validate(role),
            Tool::Map(t) => t.validate(role),
            Tool::Canvas(t) => t.validate(role),
            Tool::Design(t) => t.validate(role),
        }
    }
}

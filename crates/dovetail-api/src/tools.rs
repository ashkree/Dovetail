//! Inputs of the tools Dovetail serves to sessions over MCP, and the environment the bridge reads.
//!
//! These types are the single definition shared by the bridge (`dovetail mcp`), which parses
//! Claude Code's arguments into them, and the core, which handles them. A schema change is a
//! change here, so both sides fail to compile instead of disagreeing at run time.

use serde::{Deserialize, Serialize};

use crate::{Alternative, Bucket, DecisionId, PLevel, TaskId};

/// Names of the environment variables Dovetail passes to the processes it launches.
pub mod env {
    /// The session's agent id; set for the MCP bridge and the worktree hook.
    pub const AGENT_ID: &str = "DOVETAIL_AGENT_ID";
    /// Path of the app's unix socket; set for the MCP bridge.
    pub const SOCKET: &str = "DOVETAIL_SOCKET";
    /// `coordinator` or `slice` (see [`super::Role`]); set for the MCP bridge.
    pub const ROLE: &str = "DOVETAIL_ROLE";
    /// The project id; set for the worktree hook.
    pub const PROJECT: &str = "DOVETAIL_PROJECT";
    /// Replaces `$XDG_DATA_HOME/dovetail` (databases, worktrees). With the three below, lets a
    /// dev build in a slice run on its own state without touching Claude Code's environment.
    pub const DATA_DIR: &str = "DOVETAIL_DATA_DIR";
    /// Replaces `$XDG_CONFIG_HOME/dovetail` (config, role prompts, recipes).
    pub const CONFIG_DIR: &str = "DOVETAIL_CONFIG_DIR";
    /// Replaces `$XDG_STATE_HOME/dovetail` (logs, launch files, setup state).
    pub const STATE_DIR: &str = "DOVETAIL_STATE_DIR";
    /// Replaces `$XDG_RUNTIME_DIR/dovetail` (sockets; keep it short, ~108-byte limit).
    pub const RUNTIME_DIR: &str = "DOVETAIL_RUNTIME_DIR";
}

/// Which tools a session gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The coordinator: every tool.
    Coordinator,
    /// A slice: no `create_task`, `start_slice` or `add_project`.
    Slice,
}

impl Role {
    /// The value of [`env::ROLE`].
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Coordinator => "coordinator",
            Role::Slice => "slice",
        }
    }

    /// Parses [`env::ROLE`]; anything but `coordinator` is a slice, the safer default.
    pub fn from_env_value(value: &str) -> Role {
        if value == "coordinator" {
            Role::Coordinator
        } else {
            Role::Slice
        }
    }
}

/// One tool call: the tool and its arguments. On the wire, `{"tool": "<name>", "input": {..}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", content = "input", rename_all = "snake_case")]
pub enum Tool {
    /// `ask_user`.
    AskUser(AskUser),
    /// `report_status`.
    ReportStatus(ReportStatus),
    /// `create_task`, coordinator only.
    CreateTask(CreateTask),
    /// `log_decision`.
    LogDecision(LogDecision),
    /// `start_slice`, coordinator only (ADR 0004).
    StartSlice(StartSlice),
    /// `add_project`, coordinator only (ADR 0005).
    AddProject(AddProject),
}

/// Every tool name, in the order `tools/list` shows them.
pub const TOOL_NAMES: [&str; 6] = [
    "ask_user",
    "report_status",
    "create_task",
    "log_decision",
    "start_slice",
    "add_project",
];

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
    /// `create_task` asked for Now (ADR 0002).
    BucketNow,
    /// The coordinator logged a decision without naming its task.
    DecisionNeedsTask,
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
        }
    }
}

impl std::error::Error for ToolError {}

impl Tool {
    /// Parses MCP `tools/call` arguments for the tool `name` (without the server prefix).
    pub fn parse(name: &str, arguments: serde_json::Value) -> Result<Tool, ToolError> {
        if !TOOL_NAMES.contains(&name) {
            return Err(ToolError::Unknown(name.to_owned()));
        }
        let wire = serde_json::json!({ "tool": name, "input": arguments });
        serde_json::from_value(wire).map_err(|e| ToolError::BadArguments {
            tool: name.to_owned(),
            message: e.to_string(),
        })
    }

    /// The tool's name, without the server prefix.
    pub fn name(&self) -> &'static str {
        match self {
            Tool::AskUser(_) => "ask_user",
            Tool::ReportStatus(_) => "report_status",
            Tool::CreateTask(_) => "create_task",
            Tool::LogDecision(_) => "log_decision",
            Tool::StartSlice(_) => "start_slice",
            Tool::AddProject(_) => "add_project",
        }
    }

    /// Whether a session with `role` may call the tool named `name`.
    pub fn allowed(name: &str, role: Role) -> bool {
        match name {
            "create_task" | "start_slice" | "add_project" => role == Role::Coordinator,
            other => TOOL_NAMES.contains(&other),
        }
    }

    /// Checks what serde cannot: non-empty text, positive ids, the bucket rule, and that the
    /// coordinator names the task of a decision.
    pub fn validate(&self, role: Role) -> Result<(), ToolError> {
        fn text(field: &'static str, value: &str) -> Result<(), ToolError> {
            if value.trim().is_empty() {
                Err(ToolError::Empty(field))
            } else {
                Ok(())
            }
        }
        fn id(field: &'static str, value: TaskId) -> Result<(), ToolError> {
            if value.0 > 0 {
                Ok(())
            } else {
                Err(ToolError::NotATaskId(field))
            }
        }
        match self {
            Tool::AskUser(a) => text("question", &a.question),
            Tool::ReportStatus(r) => text("status", &r.status),
            Tool::CreateTask(c) => {
                text("title", &c.title)?;
                if c.bucket == Some(Bucket::Now) {
                    return Err(ToolError::BucketNow);
                }
                Ok(())
            }
            Tool::LogDecision(d) => {
                match (role, d.task) {
                    (Role::Coordinator, None) => return Err(ToolError::DecisionNeedsTask),
                    (_, Some(task)) => id("task", task)?,
                    (Role::Slice, None) => {}
                }
                text("decision", &d.decision)
            }
            Tool::StartSlice(s) => {
                id("task", s.task)?;
                text("name", &s.name)?;
                text("brief", &s.brief)
            }
            Tool::AddProject(_) => Ok(()),
        }
    }
}

/// `ask_user`: a question for the user's inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskUser {
    /// The question, self-contained.
    pub question: String,
    /// Wait for the answer (true) or get it later as a user message (false).
    #[serde(default)]
    pub blocking: bool,
}

/// `report_status`: the agent's one-line status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportStatus {
    /// One line; the core cuts it at 120 characters.
    pub status: String,
}

/// `create_task`: a task filed by the coordinator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTask {
    /// Short imperative title.
    pub title: String,
    /// Context, acceptance criteria, links.
    #[serde(default)]
    pub notes: String,
    /// Priority, if given.
    pub p_level: Option<PLevel>,
    /// `next` or `later`; `None` lands in Unsorted. `now` fails [`Tool::validate`].
    pub bucket: Option<Bucket>,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Related tasks.
    #[serde(default)]
    pub related: Vec<TaskId>,
}

/// `log_decision`: an entry in a task's decision log.
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

/// `start_slice`: start a slice on an existing task (ADR 0004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartSlice {
    /// An existing task.
    pub task: TaskId,
    /// Slice name, used for the branch and worktree.
    pub name: String,
    /// The slice's first prompt.
    pub brief: String,
}

/// `add_project`: register the folder the coordinator runs in as a Dovetail project (ADR 0005).
/// Takes no arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddProject {}

//! Inputs of the tools Dovetail serves to sessions over MCP, and the environment the bridge reads.
//!
//! These types are the single definition shared by the bridge (`dovetail mcp`), which parses
//! Claude Code's arguments into them, and the core, which handles them. A schema change is a
//! change here, so both sides fail to compile instead of disagreeing at run time.

use serde::{Deserialize, Serialize};

use crate::{Alternative, Bucket, DecisionId, PLevel, TaskId, TaskStatus};

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
    /// A slice: none of the [`Access::CoordinatorOnly`] tools.
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

/// Who may call a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Every session.
    Everyone,
    /// The coordinator only; refused to slices.
    CoordinatorOnly,
}

/// A row of [`TOOLS`]: a tool's name and who may call it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    /// The name, without the server prefix.
    pub name: &'static str,
    /// Who may call it.
    pub access: Access,
}

/// Defines [`Tool`], [`TOOLS`] and [`Tool::name`] from one list, so a tool's variant, wire name
/// and access rule are written once.
macro_rules! tools {
    ($( $(#[doc = $doc:literal])* $variant:ident($input:ident) = $name:literal, $access:ident; )*) => {
        /// One tool call: the tool and its arguments. On the wire,
        /// `{"tool": "<name>", "input": {..}}`.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "tool", content = "input")]
        pub enum Tool {
            $( $(#[doc = $doc])* #[serde(rename = $name)] $variant($input), )*
        }

        /// Every tool, in the order `tools/list` shows them.
        pub const TOOLS: &[ToolSpec] = &[
            $( ToolSpec { name: $name, access: Access::$access }, )*
        ];

        impl Tool {
            /// The tool's name, without the server prefix.
            pub fn name(&self) -> &'static str {
                match self {
                    $( Tool::$variant(_) => $name, )*
                }
            }
        }
    };
}

tools! {
    /// `ask_user`.
    AskUser(AskUser) = "ask_user", Everyone;
    /// `report_status`.
    ReportStatus(ReportStatus) = "report_status", Everyone;
    /// `create_task`, coordinator only.
    CreateTask(CreateTask) = "create_task", CoordinatorOnly;
    /// `log_decision`.
    LogDecision(LogDecision) = "log_decision", Everyone;
    /// `start_slice`, coordinator only (ADR 0004).
    StartSlice(StartSlice) = "start_slice", CoordinatorOnly;
    /// `add_project`, coordinator only (ADR 0005).
    AddProject(AddProject) = "add_project", CoordinatorOnly;
    /// `archive_slice`, coordinator only (task #9).
    ArchiveSlice(ArchiveSlice) = "archive_slice", CoordinatorOnly;
    /// `archive_task`, coordinator only (task #9).
    ArchiveTask(ArchiveTask) = "archive_task", CoordinatorOnly;
    /// `list_tasks`, coordinator only (task #20).
    ListTasks(ListTasks) = "list_tasks", CoordinatorOnly;
    /// `get_task`, coordinator only (task #20).
    GetTask(GetTask) = "get_task", CoordinatorOnly;
}

/// The row of [`TOOLS`] named `name`.
pub fn tool_spec(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The first word of an `archive_slice` / `archive_task` answer that did not fail.
pub mod archive_result {
    /// The slice is gone: session stopped, worktree and branch removed, its place freed.
    pub const ARCHIVED: &str = "archived";
    /// The worktree has uncommitted or unpushed work: the user is asked before it goes.
    pub const WAITING_FOR_USER: &str = "waiting_for_user";
}

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
        if tool_spec(name).is_none() {
            return Err(ToolError::Unknown(name.to_owned()));
        }
        let wire = serde_json::json!({ "tool": name, "input": arguments });
        serde_json::from_value(wire).map_err(|e| ToolError::BadArguments {
            tool: name.to_owned(),
            message: e.to_string(),
        })
    }

    /// Whether a session with `role` may call the tool named `name`.
    pub fn allowed(name: &str, role: Role) -> bool {
        tool_spec(name).is_some_and(|t| match t.access {
            Access::Everyone => true,
            Access::CoordinatorOnly => role == Role::Coordinator,
        })
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
            Tool::ArchiveSlice(a) => text("name", &a.name),
            Tool::ArchiveTask(a) => id("id", a.id),
            Tool::ListTasks(_) => Ok(()),
            Tool::GetTask(g) => id("id", g.id),
        }
    }
}

/// `ask_user`: a question for the user's inbox. It never blocks: the call returns at once and the
/// answer arrives as a user message (ADR 0007). An old caller's `blocking` field is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskUser {
    /// The question, self-contained.
    pub question: String,
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

/// `archive_slice`: archive a slice by name (task #9). Clean, pushed work goes at once; unsaved
/// work waits for the user's yes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSlice {
    /// The slice's name, as given to `start_slice`.
    pub name: String,
}

/// `archive_task`: archive a task, and first its live slices, with `archive_slice`'s rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveTask {
    /// The task.
    pub id: TaskId,
}

/// A bucket as `list_tasks` filters it: the three [`Bucket`]s plus Unsorted (no bucket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketFilter {
    /// [`Bucket::Now`].
    Now,
    /// [`Bucket::Next`].
    Next,
    /// No bucket yet.
    Unsorted,
    /// [`Bucket::Later`].
    Later,
}

impl BucketFilter {
    /// Whether a task with `bucket` is in this one.
    pub fn matches(self, bucket: Option<Bucket>) -> bool {
        match self {
            BucketFilter::Now => bucket == Some(Bucket::Now),
            BucketFilter::Next => bucket == Some(Bucket::Next),
            BucketFilter::Unsorted => bucket.is_none(),
            BucketFilter::Later => bucket == Some(Bucket::Later),
        }
    }
}

/// `list_tasks`: one line per task, Now, Next, Unsorted, Later (task #20). Every field is
/// optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListTasks {
    /// Only tasks in this bucket.
    pub bucket: Option<BucketFilter>,
    /// Only tasks with this status; `archived` lists archived tasks without
    /// `include_archived`.
    pub status: Option<TaskStatus>,
    /// Also list archived tasks.
    pub include_archived: bool,
}

/// `get_task`: one task in full, with its agents and decision log (task #20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetTask {
    /// The task.
    pub id: TaskId,
}

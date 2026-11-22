//! The `agent` feature: coordinators, slices, side chats and subagents, and their native to-do lists.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::AgentCommand;
pub use event::AgentEvent;

use crate::model::{DriverKind, ReasoningEffort};
use crate::task::TaskId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Identifies an agent: a coordinator, slice or subagent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub String);

/// Identifies one of Claude's native to-do lists (`CLAUDE_CODE_TASK_LIST_ID`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TodoListId(pub String);

/// A slash command or skill a session offers (`/compact`, `/code-review`), typed as `/name` in
/// a chat bar and sent to the session as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    /// Name without the slash.
    pub name: String,
    /// What it does, as its author wrote it.
    pub description: String,
    /// What it takes after the name, e.g. `<pr-number>`; empty when nothing.
    #[serde(default)]
    pub argument_hint: String,
}

/// A coordinator, slice or subagent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// Agent id.
    pub id: AgentId,
    /// Display name.
    pub name: String,
    /// Coordinator, slice or subagent.
    pub kind: AgentKind,
    /// The session that spawned it (subagents only).
    pub parent: Option<AgentId>,
    /// The task it works on.
    pub task: Option<TaskId>,
    /// Its worktree (slices only; subagents share their parent's).
    pub worktree: Option<WorktreeRef>,
    /// What it is doing.
    pub status: AgentStatus,
    /// One-line status text, from `core_report_status` or the stream.
    pub status_line: String,
    /// Memory use in MB (sessions only).
    pub memory_mb: Option<u32>,
    /// Cost so far, in API-equivalent US dollars.
    pub cost_usd: f64,
    /// Current permission mode.
    pub permission_mode: PermissionMode,
    /// How full the context window is, 0–100 (sessions only).
    pub context_pct: Option<u8>,
    /// Progress of a subagent (subagents only).
    pub progress: Option<SubagentProgress>,
    /// Last stream activity, in Unix milliseconds.
    pub last_activity_ms: i64,
    /// Whether Dovetail or a native terminal drives the session.
    pub lock: SessionLock,
    /// The model the session runs, as Claude Code names it (`claude-opus-5-5`); `None` until
    /// the session reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The session's reasoning effort, as Claude Code last confirmed it; `None` until it does,
    /// or when its model has no effort levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    /// Tokens its subagents used, summed from their `task_*` events (sessions only; digest §3:
    /// the final `result` undercounts). Includes subagents whose nodes are gone.
    #[serde(default)]
    pub subagent_tokens: u64,
    /// The tool the session is running right now (sessions only); `None` between tools, while
    /// the model thinks or writes, and when idle. Not stored: a restart starts without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<RunningTool>,
    /// The CLI the session runs on (task #46); fixed for the life of a session. Older data
    /// reads as Claude Code.
    #[serde(default)]
    pub driver: DriverKind,
}

/// A tool call in flight: what the working line says the agent is doing ("Reading plan.md").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningTool {
    /// Claude Code's tool name (`Read`, `Bash`, `mcp__dovetail__ask_user`).
    pub name: String,
    /// The call's main argument: a path, command, pattern or URL; empty when it has none.
    pub arg: String,
}

/// What an agent is. Coordinators and slices are their own Claude Code sessions; subagents
/// run inside one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// The session the user works with directly, in the main checkout.
    Coordinator,
    /// A long-lived session with its own worktree.
    Slice,
    /// A read-only session the user opens for a question or research: no worktree, no task
    /// (ADR 0011).
    SideChat,
    /// Reading mode's Ask (ADR 0018): a read-only session about one research task's report,
    /// in its research folder; [`Agent::task`] is the task. Shown only in the Ask column.
    Ask,
    /// A tangent (ADR 0017): a read-only session like a side chat, run in Tangents' folder
    /// whatever project is open, whose answers and sources are saved in Tangents. Its tangent is
    /// the Tangents task its id names ([`crate::tangent::tangent_task`]); [`Agent::task`] stays `None`, since
    /// that task is not the open project's. Shown in the side panel, not in the tree.
    Tangent,
    /// Spawned by a session for self-contained work.
    Subagent {
        /// Runs in the background with no to-do items.
        background: bool,
    },
}

impl AgentKind {
    /// Whether the agent is its own Claude Code session rather than a subagent.
    pub fn is_session(self) -> bool {
        !matches!(self, AgentKind::Subagent { .. })
    }
}

/// A slice's worktree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorktreeRef {
    /// Worktree directory.
    pub path: PathBuf,
    /// Checked-out branch.
    pub branch: String,
}

/// What an agent is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Its worktree is being prepared.
    SettingUp,
    /// Mid-turn.
    Working,
    /// Waiting for input.
    Idle,
    /// Has an item in Needs you.
    NeedsYou,
    /// Finished for good.
    Done,
    /// Crashed or failed.
    Error,
}

/// A session's permission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// The classifier approves ordinary actions (the default).
    Auto,
    /// File edits are approved; other tools ask.
    AcceptEdits,
    /// Every tool use asks.
    Manual,
}

/// Progress of a subagent, from `task_*` events.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentProgress {
    /// What the subagent was asked to do.
    pub description: String,
    /// The tool it used last.
    pub last_tool: Option<String>,
    /// Tool uses so far.
    pub tool_uses: u32,
    /// Final summary, once finished.
    pub summary: Option<String>,
    /// Tokens used so far (`usage.total_tokens`).
    #[serde(default)]
    pub total_tokens: u64,
    /// Time it has run, in milliseconds (`usage.duration_ms`).
    #[serde(default)]
    pub duration_ms: u64,
    /// The file Claude Code writes its transcript to, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file: Option<PathBuf>,
}

/// Whether Dovetail or a native terminal drives a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLock {
    /// Dovetail drives it headless.
    Headless,
    /// The user has taken it over in a native terminal (`<leader>st`): headless never starts
    /// until the native run ends. The lock is stored, so it outlives a crash of Dovetail.
    TakenOver,
}

/// An entry on Claude's native to-do list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    /// The list it belongs to.
    pub list: TodoListId,
    /// Item id within the list.
    pub id: String,
    /// What to do.
    pub subject: String,
    /// Where it stands.
    pub status: TodoStatus,
    /// Items it waits for.
    pub blocked_by: Vec<String>,
    /// The agent that created it, when attributed.
    pub created_by: Option<AgentId>,
}

/// Where a to-do item stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Being worked on.
    InProgress,
    /// Finished.
    Completed,
}

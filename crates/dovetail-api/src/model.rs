//! The plain-data state the core shares with the UI.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Identifies a registered project: the slug of its repo folder, numbered on collision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(pub String);

/// Identifies one of the user's tasks. Matches the `tasks.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(pub i64);

/// Identifies an agent: a coordinator, slice or subagent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub String);

/// Identifies an inbox item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InboxId(pub u64);

/// Identifies a conversation entry. Ids grow with time within one agent's conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub u64);

/// Identifies one of Claude's native to-do lists (`CLAUDE_CODE_TASK_LIST_ID`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TodoListId(pub String);

/// Identifies an entry in a task's decision log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DecisionId(pub u64);

/// Identifies a status-bar warning, so it can be replaced or cleared.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WarningId(pub String);

/// The whole client-visible state at one point in the event stream.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Sequence number of the last event folded into this snapshot.
    pub seq: u64,
    /// Core clock when the snapshot was taken, in Unix milliseconds.
    pub now_ms: i64,
    /// The project open in this window, if any.
    pub project: Option<ProjectSummary>,
    /// Every registered project, for the project switcher.
    pub projects: Vec<ProjectSummary>,
    /// The open project's tasks, archived ones included.
    pub tasks: Vec<Task>,
    /// Every agent in the open project.
    pub agents: Vec<Agent>,
    /// To-do items from Claude's native lists.
    pub todos: Vec<TodoItem>,
    /// Needs you: everything waiting on the user.
    pub inbox: Vec<InboxItem>,
    /// The Activity feed, oldest first, bounded to [`ACTIVITY_TAIL`] items.
    pub activity: Vec<ActivityItem>,
    /// Messages between agents, oldest first, bounded to [`MESSAGE_TAIL`] items.
    pub messages: Vec<AgentMessage>,
    /// The latest entries of each agent's conversation.
    pub conversations: BTreeMap<AgentId, ConversationTail>,
    /// Status-bar totals.
    pub totals: Totals,
    /// Status-bar warnings.
    pub warnings: Vec<Warning>,
    /// Every task's decision log, oldest first.
    #[serde(default)]
    pub decisions: Vec<Decision>,
}

impl Snapshot {
    /// The snapshot as a core sends it: each conversation cut to its newest
    /// [`CONVERSATION_TAIL`] entries, older ones left for `LoadConversation`. A client keeps
    /// what it pages in beyond that, up to [`CONVERSATION_KEEP`].
    pub fn trimmed(&self) -> Snapshot {
        let mut out = self.clone();
        for tail in out.conversations.values_mut() {
            if tail.entries.len() > CONVERSATION_TAIL {
                let excess = tail.entries.len() - CONVERSATION_TAIL;
                tail.entries.drain(..excess);
                tail.has_more = true;
            }
        }
        out
    }
}

/// Most activity items a client keeps.
pub const ACTIVITY_TAIL: usize = 500;
/// Most agent messages a client keeps.
pub const MESSAGE_TAIL: usize = 500;
/// Most conversation entries a snapshot carries per agent; older ones are paged in on request.
pub const CONVERSATION_TAIL: usize = 200;
/// Most conversation entries a client keeps per agent, paged-in history included, before the
/// oldest are dropped again.
pub const CONVERSATION_KEEP: usize = 2_000;

/// A registered project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSummary {
    /// Project id.
    pub id: ProjectId,
    /// Display name.
    pub name: String,
    /// The main checkout.
    pub repo_path: PathBuf,
}

/// A piece of the user's own work, with or without agents attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Task id.
    pub id: TaskId,
    /// One-line title.
    pub title: String,
    /// Where the task stands.
    pub status: TaskStatus,
    /// How important it is; `None` for some unsorted tasks.
    pub p_level: Option<PLevel>,
    /// When the user will do it; `None` means Unsorted.
    pub bucket: Option<Bucket>,
    /// Position within its bucket, lowest first.
    pub order: i64,
    /// Tasks this one waits for.
    pub deps: Vec<TaskId>,
    /// Free-form notes.
    pub notes: String,
    /// Linked pull request number.
    pub pr: Option<u32>,
    /// The task this one follows up.
    pub follow_up_of: Option<TaskId>,
    /// Created by the coordinator and not yet seen by the user.
    pub coordinator_created: bool,
    /// Tasks related to this one, both ways; no effect on order or status.
    #[serde(default)]
    pub related: Vec<TaskId>,
}

/// Where a task stands. Whether it needs you is not a status: it follows from the inbox
/// items of its agents (plan B4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Not started.
    Todo,
    /// Started; set when its first slice starts, and kept after slices close.
    InProgress,
    /// Marked done by the user.
    Done,
    /// Hidden from the tree.
    Archived,
}

/// How important a task is, from P0 (most) to P3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PLevel {
    /// Most important.
    P0,
    /// Important.
    P1,
    /// Normal.
    P2,
    /// Least important.
    P3,
}

/// When the user will do a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    /// Doing it now.
    Now,
    /// Up next.
    Next,
    /// Some time later.
    Later,
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
    /// One-line status text, from `report_status` or the stream.
    pub status_line: String,
    /// Memory use in MB (sessions only).
    pub memory_mb: Option<u32>,
    /// Cost so far, in API-equivalent US dollars.
    pub cost_usd: f64,
    /// Current permission mode.
    pub permission_mode: PermissionMode,
    /// How full the context window is, 0–100 (sessions only).
    pub context_pct: Option<u8>,
    /// Progress of a background subagent.
    pub progress: Option<SubagentProgress>,
    /// Last stream activity, in Unix milliseconds.
    pub last_activity_ms: i64,
    /// Whether Dovetail or a native terminal drives the session.
    pub lock: SessionLock,
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

/// Progress of a background subagent, from `task_*` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentProgress {
    /// What the subagent was asked to do.
    pub description: String,
    /// The tool it used last.
    pub last_tool: Option<String>,
    /// Tool uses so far.
    pub tool_uses: u32,
    /// Final summary, once finished.
    pub summary: Option<String>,
}

/// Whether Dovetail or a native terminal drives a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLock {
    /// Dovetail drives it headless.
    Headless,
    /// The user has taken it over in a native terminal.
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

/// An entry in Needs you.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    /// Item id.
    pub id: InboxId,
    /// The agent it came from.
    pub agent: AgentId,
    /// When it arrived, in Unix milliseconds.
    pub at_ms: i64,
    /// What it asks for.
    pub kind: InboxKind,
}

/// What an inbox item asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    /// Allow or deny a tool use.
    Permission {
        /// Tool name.
        tool: String,
        /// Tool input as sent by Claude Code.
        input: serde_json::Value,
        /// Why it was escalated, when given.
        reason: Option<String>,
        /// Rules offered for "always allow".
        suggestions: Vec<RuleSuggestion>,
    },
    /// An `ask_user` question.
    Question {
        /// The question.
        text: String,
        /// Whether the agent waits for the answer.
        blocking: bool,
    },
    /// The agent's turn ended; at most one per agent (ADR 0001).
    FinishedTurn {
        /// Start of its last message.
        preview: String,
    },
    /// A driver error, crash or memory kill.
    Error {
        /// What went wrong.
        text: String,
    },
}

/// A permission rule Claude Code offers for "always allow".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSuggestion {
    /// Short human-readable form, such as `Bash(cargo test:*)`.
    pub label: String,
    /// The suggestion as Claude Code sent it, to echo back.
    pub raw: serde_json::Value,
}

/// An item in the Activity feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityItem {
    /// Item id, increasing.
    pub id: u64,
    /// When it happened, in Unix milliseconds.
    pub at_ms: i64,
    /// The agent involved, if any.
    pub agent: Option<AgentId>,
    /// What kind of thing happened.
    pub kind: ActivityKind,
    /// One-line description.
    pub text: String,
}

/// What kind of thing an activity item reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    /// A pull request was opened, pushed or merged.
    Pr,
    /// A CI run changed status.
    Ci,
    /// Agents messaged each other.
    Message,
    /// An agent started, finished or was closed.
    Lifecycle,
}

/// A message sent between agents with `SendMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Message id, increasing.
    pub id: u64,
    /// When it was sent, in Unix milliseconds.
    pub at_ms: i64,
    /// Sender.
    pub from: AgentId,
    /// Recipient.
    pub to: AgentId,
    /// Message text.
    pub text: String,
}

/// The latest part of one agent's conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationTail {
    /// Entries, oldest first.
    pub entries: Vec<ConversationEntry>,
    /// Older entries exist and can be fetched with `LoadConversation`.
    pub has_more: bool,
}

/// One entry in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationEntry {
    /// Entry id.
    pub id: EntryId,
    /// When it started, in Unix milliseconds.
    pub at_ms: i64,
    /// Who or what it is.
    pub kind: EntryKind,
    /// Markdown text; assistant entries grow through `AssistantDelta`.
    pub text: String,
}

/// Who or what a conversation entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// The user's message.
    User,
    /// Assistant text.
    Assistant,
    /// A turn's tool activity, folded into one summary line.
    ToolSummary,
    /// A note from Dovetail, such as "interrupted".
    Notice,
}

/// Status-bar totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    /// Memory across all sessions, in MB.
    pub memory_mb: u32,
    /// Items in Needs you.
    pub needs_you: u32,
    /// Cost across all agents, in API-equivalent US dollars.
    pub cost_usd: f64,
}

/// One entry in a task's decision log. Entries are never edited: a later one can supersede it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Entry id, increasing.
    pub id: DecisionId,
    /// The task it belongs to.
    pub task: TaskId,
    /// When it was logged, in Unix milliseconds.
    pub at_ms: i64,
    /// Who logged it.
    pub by: DecisionAuthor,
    /// The decision, in one line.
    pub text: String,
    /// Why, if given.
    pub why: Option<String>,
    /// Options that were considered and not taken.
    pub alternatives: Vec<Alternative>,
    /// The earlier entry this one replaces.
    pub supersedes: Option<DecisionId>,
}

/// An option considered for a decision and not taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alternative {
    /// The option, in one line.
    pub option: String,
    /// Why it lost, if given.
    pub why_not: Option<String>,
}

/// Who logged a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAuthor {
    /// The user, by hand.
    User,
    /// An agent, through `log_decision`.
    Agent(AgentId),
}

/// A status-bar warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// Warning id.
    pub id: WarningId,
    /// What to tell the user.
    pub text: String,
}

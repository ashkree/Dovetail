//! The plain-data state the core shares with the UI.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::map::MapSummary;
use crate::research::ResearchSummary;

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

/// Id of the warning shown while the open folder is not a Dovetail project yet (ADR 0005). The
/// TUI shows it in the header row instead of the status bar.
pub const UNREGISTERED_WARNING: &str = "unregistered";

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
    /// The open project's Claude usage limits, as last reported by a session of its profile
    /// (limits are per account).
    #[serde(default)]
    pub usage: UsageLimits,
    /// The Claude profile the open project's sessions run under (ADR 0009); `None` when nothing
    /// names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Git and PR state of each slice's worktree, as last gathered (empty until the first
    /// refresh).
    #[serde(default)]
    pub worktrees: Vec<WorktreeInfo>,
    /// The map shelf: every map of the open project, most recently drawn first. Content is
    /// fetched with [`crate::Command::LoadMap`].
    #[serde(default)]
    pub maps: Vec<MapSummary>,
    /// Each task's research (rounds, finding and source counts), for tasks that have any.
    /// Content is fetched with [`crate::Command::LoadResearch`].
    #[serde(default)]
    pub research: Vec<ResearchSummary>,
    /// The slash commands and skills each session offers, as Claude Code last listed them.
    /// Sessions of one project share a profile, so one agent's list stands in for another's
    /// until that one reports.
    #[serde(default)]
    pub commands: BTreeMap<AgentId, Vec<SlashCommand>>,
    /// The project's end-of-day journal (ADR 0012); `None` when the open folder is not a
    /// registered project, so it has no journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal: Option<JournalState>,
}

/// Where the project's end-of-day journal stands (ADR 0012).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalState {
    /// The date of the newest journal file, `YYYY-MM-DD` (local time); `None` when there is
    /// none yet.
    pub latest: Option<String>,
    /// `:eod` is waiting for the coordinator's handoff before it writes the day's file.
    pub closing: bool,
}

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
    /// The folder its sessions run in: the main checkout, the folder of a project without git,
    /// or, for a research project, Dovetail's own state folder for it.
    pub repo_path: PathBuf,
    /// Code or research. A peer that does not send it means a code project.
    #[serde(default)]
    pub kind: ProjectKind,
}

/// What a project is for. A research project has no repo: no worktrees, no slices, no git; its
/// sessions run in `~/.local/share/dovetail/projects/<id>/`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    /// A repo or folder of the user's: the default.
    #[default]
    Code,
    /// Macro research only (glossary "Research-only project").
    Research,
}

impl ProjectKind {
    /// The name stored in the registry.
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectKind::Code => "code",
            ProjectKind::Research => "research",
        }
    }

    /// The kind stored as `name`, if it is one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "code" => Some(ProjectKind::Code),
            "research" => Some(ProjectKind::Research),
            _ => None,
        }
    }
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
    /// Tokens its subagents used, summed from their `task_*` events (sessions only; digest §3:
    /// the final `result` undercounts). Includes subagents whose nodes are gone.
    #[serde(default)]
    pub subagent_tokens: u64,
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
    /// A `core_ask_user` question.
    Question {
        /// The question.
        text: String,
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
    /// A research finding of a coordinator subagent that ran with no round open, stored with no
    /// task until the user files it (`y`) or drops it (`d`).
    UnfiledFinding {
        /// The stored finding (its own id sequence until it is filed).
        finding: i64,
        /// Its label, from the subagent's description.
        label: String,
        /// How many sources it fetched or read.
        sources: u32,
    },
    /// A `research_propose` call: an idea or a decision the coordinator wants the user to accept
    /// (ADR 0015). Nothing is written until `y`.
    Proposal {
        /// The research task (the question).
        task: TaskId,
        /// Idea or decision.
        kind: crate::research::ProposalKind,
        /// The idea, or the decision in one line.
        text: String,
        /// The findings and sources behind it; never empty for a decision.
        evidence: Vec<crate::research::Evidence>,
        /// Why, if given.
        #[serde(default)]
        why: Option<String>,
        /// Options considered and not taken (decisions).
        #[serde(default)]
        alternatives: Vec<Alternative>,
    },
}

/// The `tool` of the Permission item Dovetail raises itself for a `project_start_slice` that needs
/// approval (ADR 0004). Its input is `{task, name, brief}`.
pub const START_SLICE_PERMISSION: &str = "start_slice";

/// The `tool` of the Permission item that asks before removing a slice worktree with unsaved
/// work. Its input is `{worktree, branch, uncommitted: [..], unpushed_commits}`.
pub const REMOVE_WORKTREE_PERMISSION: &str = "Remove worktree";

/// The `tool` of the Permission item raised when a fresh coordinator started on the first open
/// of a day (ADR 0012): `y` goes back to yesterday's session, `d` keeps the fresh one. Its input
/// is `{journal, session_id, transcript_path}`: the handoff's date and the old session.
pub const KEEP_COORDINATOR_PERMISSION: &str = "Keep yesterday's coordinator";

/// The `tool` of the Permission item raised when `:fresh` started a new coordinator mid-day
/// (ADR 0012, amendment): `y` goes back to the previous session, `d` keeps the fresh one. Its
/// input is `{journal, session_id, transcript_path}`, as for [`KEEP_COORDINATOR_PERMISSION`];
/// `journal` is the date of the file holding the handoff.
pub const FRESH_COORDINATOR_PERMISSION: &str = "Fresh coordinator started";

/// The `tool` of the Permission item raised at open when a session's take-over lock was left by
/// a crash and Dovetail cannot tell whether its native `claude` still runs (M14). `y` releases
/// the lock, `d` keeps it. Its input is `{session, reason}`.
pub const RELEASE_TAKEOVER_PERMISSION: &str = "Release take-over lock";

/// The `tool` of the Permission item raised when an upgrade build is staged (ADR 0014): `y`
/// restarts once every agent is idle, `d` leaves the build for the next ordinary quit. Its input
/// is `{source, differing: [..], waiting: [..], restarting}`: where the build came from, the role
/// prompts and recipe whose installed copy differs (never overwritten), the agents `y` still
/// waits for, and whether `y` was pressed.
pub const UPGRADE_READY_PERMISSION: &str = "Upgrade ready";

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
    /// The user answered a permission request.
    Permission,
    /// A task was created or changed.
    Task,
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
    /// Images sent with a user message, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// Most images one message can carry.
pub const MAX_ATTACHMENTS: usize = 10;

/// Bytes in a megabyte, as Dovetail shows sizes (binary units).
const MB: u64 = 1024 * 1024;

/// Largest image file accepted, in bytes (5 MB).
pub const MAX_ATTACHMENT_BYTES: u64 = 5 * MB;

/// A file size as the UI and its notices show it: `412 KB` or `1.2 MB` (binary units, at least
/// `1 KB`). Shared so the core's messages and the TUI's chips agree.
pub fn size_text(size: u64) -> String {
    if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else {
        format!("{} KB", size.div_ceil(1024).max(1))
    }
}

/// [`MAX_ATTACHMENT_BYTES`] for messages: `5 MB`.
pub fn attachment_limit_text() -> String {
    format!("{} MB", MAX_ATTACHMENT_BYTES / MB)
}

/// An image attached to a user message. The UI saves the file under Dovetail's data dir and
/// sends its path; the core reads it when sending, so no raw bytes cross the protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// The saved image file.
    pub path: PathBuf,
    /// Its format.
    pub media_type: ImageType,
    /// File size in bytes.
    pub size: u64,
}

/// An image format Claude accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageType {
    /// `image/png`.
    #[serde(rename = "image/png")]
    Png,
    /// `image/jpeg`.
    #[serde(rename = "image/jpeg")]
    Jpeg,
    /// `image/gif`.
    #[serde(rename = "image/gif")]
    Gif,
    /// `image/webp`.
    #[serde(rename = "image/webp")]
    Webp,
}

impl ImageType {
    /// Every accepted format, in order of preference when a source offers several.
    pub const ALL: [ImageType; 4] = [Self::Png, Self::Jpeg, Self::Webp, Self::Gif];

    /// The MIME type, as Claude's image blocks name it.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// The format a MIME type names (`image/jpg` counts as JPEG).
    pub fn from_mime(mime: &str) -> Option<Self> {
        match mime.trim().to_ascii_lowercase().as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }

    /// The format the file's first bytes show, if it is one of the accepted ones.
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(Self::Webp)
        } else {
            None
        }
    }

    /// File extension for a saved image.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }
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

/// How close the account is to its Claude usage limits. Limits are per account, so this is the
/// latest report from any session. Nothing is known until a session finishes a turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageLimits {
    /// The rolling 5-hour window.
    pub five_hour: Option<UsageWindow>,
    /// The weekly (7-day) window.
    pub seven_day: Option<UsageWindow>,
}

/// One usage-limit window, from Claude Code's `rate_limit_event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    /// How much of the window is used, 0–1 (may exceed 1 when over the limit).
    pub utilization: f64,
    /// When the window resets, in Unix milliseconds like every time in the API; `None` when not
    /// reported.
    pub resets_at_ms: Option<i64>,
    /// Claude Code's status for the window.
    pub status: UsageStatus,
}

/// Claude Code's status word for a usage window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    /// `allowed`: under the limit.
    Allowed,
    /// `allowed_warning`: under the limit, close to it.
    AllowedWarning,
    /// `rejected`: over the limit.
    Rejected,
    /// A word this version does not know (or none): new Claude Code values never break parsing.
    #[serde(other)]
    Unknown,
}

impl UsageStatus {
    /// Parses Claude Code's word; anything unrecognised is [`UsageStatus::Unknown`].
    pub fn from_word(word: &str) -> UsageStatus {
        serde_json::from_value(serde_json::Value::String(word.to_owned()))
            .unwrap_or(UsageStatus::Unknown)
    }
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
    /// The findings and sources it rests on (decisions accepted from `research_propose`).
    #[serde(default)]
    pub evidence: Vec<crate::research::Evidence>,
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
    /// An agent, through `core_log_decision`.
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

/// Git and pull-request state of one slice's worktree, for the worktree view (`<leader>g`).
///
/// The core fills it in full on [`crate::Command::RefreshWorktrees`] (fetching the base and
/// asking `gh`), and refreshes the local git parts after a slice's turn finishes; the PR then
/// stays as last looked up. Each part is looked up on its own: one that could not be read is `None` (or
/// [`PrLookup::Unknown`]) and the rest still shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// The slice that owns the worktree.
    pub agent: AgentId,
    /// Worktree directory.
    pub path: PathBuf,
    /// The slice's branch (`dovetail/<name>`).
    pub branch: String,
    /// The branch it is compared with: the one checked out in the main checkout, normally
    /// `main`.
    pub base: String,
    /// Commits ahead of and behind the base; `None` if git could not tell.
    pub sync: Option<SyncState>,
    /// Uncommitted changes, Dovetail's own symlinks excluded; `None` if git could not tell.
    pub changes: Option<Changes>,
    /// Commits on the branch not pushed anywhere: ahead of its upstream, or, with no upstream,
    /// on no remote and not in the base. `None` if git could not tell.
    pub unpushed: Option<u32>,
    /// The branch's pull request, from `gh`.
    pub pr: PrLookup,
    /// The recipe's setup commands in this worktree.
    pub setup: WorktreeSetup,
    /// When this was gathered, in Unix milliseconds.
    pub checked_ms: i64,
}

/// Where a branch stands against its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// Commits on the branch that the base lacks.
    pub ahead: u32,
    /// Commits on the base that the branch lacks.
    pub behind: u32,
    /// Commits on `origin/<base>` that the branch lacks, as of the last fetch: non-zero means
    /// the remote base has moved since the branch last took it in. `None` without such a
    /// remote branch, or when the fetch on [`crate::Command::RefreshWorktrees`] failed.
    pub behind_origin: Option<u32>,
}

/// Counts of uncommitted entries in a worktree, as `git status` reports them. An entry both
/// staged and changed again counts in both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    /// Entries with changes in the index.
    pub staged: u32,
    /// Tracked entries changed in the working tree but not staged.
    pub unstaged: u32,
    /// Untracked entries.
    pub untracked: u32,
    /// Entries with merge conflicts.
    pub conflicted: u32,
}

impl Changes {
    /// True when nothing is uncommitted.
    pub fn is_clean(&self) -> bool {
        *self == Changes::default()
    }
}

/// What looking up a branch's pull request found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrLookup {
    /// `gh` is missing, failed or timed out, or has not been asked yet.
    Unknown {
        /// Why, in a few words.
        reason: String,
    },
    /// The branch has no pull request.
    None,
    /// The branch's pull request: an open one if there is one, else the newest.
    Found(PullRequest),
}

/// A pull request on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// PR number.
    pub number: u32,
    /// Open, merged or closed.
    pub state: PrState,
    /// Its web address.
    pub url: String,
}

/// State of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    /// Open (drafts included).
    Open,
    /// Merged.
    Merged,
    /// Closed without merging.
    Closed,
}

/// The recipe's setup commands in one worktree, as the `WorktreeCreate` hook recorded them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeSetup {
    /// Nothing recorded: the recipe has no setup commands.
    None,
    /// The commands are running.
    Running,
    /// Every command succeeded.
    Done,
    /// A command failed, or the run was interrupted.
    Failed {
        /// What went wrong, if recorded.
        error: Option<String>,
    },
}

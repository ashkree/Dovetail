//! Messages between the core and the UI.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use serde::{Deserialize, Serialize};

use crate::map::{Map, MapId, MapSummary};
use crate::model::{
    ActivityItem, Agent, AgentId, AgentMessage, Alternative, Attachment, Bucket, ConversationEntry,
    Decision, DecisionId, EntryId, InboxId, InboxItem, JournalState, PLevel, ProjectId,
    SlashCommand, Snapshot, Task, TaskId, TaskStatus, TodoItem, TodoListId, Totals, UsageLimits,
    Warning, WarningId, WorktreeInfo,
};
use crate::research::{ResearchContent, ResearchSummary, ResearchWhat};

/// Version of this protocol, sent in [`ServerMsg::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Everything the core sends: a hello, then a snapshot, then numbered events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// One snapshot per connection or resync against many events: boxing it would only add an
// allocation to every match on the stream.
#[allow(clippy::large_enum_variant)]
#[serde(rename_all = "snake_case")]
pub enum ServerMsg {
    /// First message on a connection.
    Hello {
        /// The core's [`PROTOCOL_VERSION`].
        protocol: u32,
    },
    /// The full state; replaces whatever the client held.
    Snapshot(Snapshot),
    /// One change, to fold in with [`crate::apply`].
    Event {
        /// Must be one more than the client's current `seq`.
        seq: u64,
        /// The change.
        event: Event,
    },
}

/// One change to the client's [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// A task was created or changed.
    TaskUpserted(Task),
    /// A task was deleted.
    TaskRemoved(TaskId),
    /// An agent was created or changed.
    AgentUpserted(Agent),
    /// An agent was removed, with its conversation.
    AgentRemoved(AgentId),
    /// A native to-do list was rewritten.
    TodosReplaced {
        /// The list.
        list: TodoListId,
        /// Its items now.
        items: Vec<TodoItem>,
    },
    /// An item arrived in Needs you, or an existing one changed.
    InboxUpserted(InboxItem),
    /// An item left Needs you.
    InboxResolved {
        /// The item.
        id: InboxId,
    },
    /// A slice's worktree state was gathered afresh. A worktree's entry goes with its agent
    /// ([`Event::AgentRemoved`]).
    WorktreeUpserted(WorktreeInfo),
    /// Something happened in the Activity feed.
    ActivityAdded(ActivityItem),
    /// Agents messaged each other.
    MessageAdded(AgentMessage),
    /// A conversation entry was added or replaced.
    ConversationAppended {
        /// Whose conversation.
        agent: AgentId,
        /// The entry.
        entry: ConversationEntry,
    },
    /// More streamed text for an assistant entry.
    AssistantDelta {
        /// Whose conversation.
        agent: AgentId,
        /// The entry it extends.
        entry: EntryId,
        /// Text to append.
        text: String,
    },
    /// Older entries, in answer to [`Command::LoadConversation`].
    ConversationPage {
        /// Whose conversation.
        agent: AgentId,
        /// Entries older than the tail, oldest first.
        entries: Vec<ConversationEntry>,
        /// Still older entries exist.
        has_more: bool,
    },
    /// Status-bar totals changed.
    TotalsChanged(Totals),
    /// A session reported new usage-limit figures.
    UsageChanged(UsageLimits),
    /// A warning was raised or reworded.
    WarningSet(Warning),
    /// A warning no longer applies.
    WarningCleared(WarningId),
    /// An entry was added to a task's decision log.
    DecisionAdded(Decision),
    /// A map was drawn (version 1) or redrawn (a later version): its shelf entry now.
    MapUpserted(MapSummary),
    /// A map version's content, in answer to [`Command::LoadMap`]. Changes nothing in the
    /// snapshot; the UI keeps what it shows.
    MapLoaded(Map),
    /// A task's research changed: a round opened or closed, a finding was filed.
    ResearchUpserted(ResearchSummary),
    /// Research content, in answer to [`Command::LoadResearch`]. Changes nothing in the
    /// snapshot.
    ResearchLoaded {
        /// The task.
        task: TaskId,
        /// What was asked for.
        what: ResearchWhat,
        /// The answer.
        content: ResearchContent,
    },
    /// A command could not be carried out; the UI shows the message.
    CommandFailed {
        /// What went wrong.
        message: String,
    },
    /// The clipboard was read, in answer to [`Command::PasteClipboard`]. Changes nothing in the
    /// snapshot; the UI puts the result in that agent's chat bar if it is still open.
    ClipboardPasted {
        /// The agent whose chat bar asked.
        agent: AgentId,
        /// What the clipboard gave.
        paste: Paste,
    },
    /// A session listed its slash commands and skills (at start, and when they change).
    CommandsChanged {
        /// The session.
        agent: AgentId,
        /// Everything it offers, in its own order.
        commands: Vec<SlashCommand>,
    },
    /// Paths for `@` completion in a chat bar, in answer to [`Command::ListPaths`]. Changes
    /// nothing in the snapshot.
    PathsListed {
        /// The agent whose chat bar asked.
        agent: AgentId,
        /// The directory asked about, as typed; `None` for the session's whole tree.
        dir: Option<String>,
        /// Paths relative to the session's folder (whole tree), or names in `dir` with a
        /// trailing `/` on directories. Empty when nothing could be listed.
        paths: Vec<String>,
    },
    /// The program run in the user's terminal ([`Command::OpenEditor`], [`Command::TakeOver`])
    /// has exited, so the UI takes the terminal back. Changes nothing in the snapshot.
    TerminalReturned {
        /// Why the program could not run or failed, for the status bar; `None` when it exited
        /// cleanly.
        error: Option<String>,
    },
    /// The end-of-day journal changed: `:eod` started waiting for the handoff, or a day's file
    /// was written (ADR 0012).
    JournalChanged(JournalState),
    /// Every agent is idle and the user said `y` on "Upgrade ready" (ADR 0014): the core is
    /// quitting, and the UI quits too, without asking. The binary then swaps the staged build in
    /// and starts it in the same terminal. Changes nothing in the snapshot.
    UpgradeRestarting,
}

/// What the core found on the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Paste {
    /// An accepted image (PNG, JPEG, GIF or WebP, at most [`crate::MAX_ATTACHMENT_BYTES`]),
    /// already saved under Dovetail's `attachments/` folder: the UI adds it as a chip.
    Image(Attachment),
    /// Text, to insert at the cursor.
    Text(String),
    /// Nothing could be pasted; a short notice for the status bar saying why.
    Refused(String),
}

/// Something the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Send the user's prompt to an agent.
    Prompt {
        /// Recipient.
        agent: AgentId,
        /// Prompt text; may be empty when images are attached.
        text: String,
        /// Images to send with it, saved by the core on [`Command::PasteClipboard`]; the core
        /// reads the files.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
    },
    /// Read the system clipboard for an agent's chat bar (Ctrl-V). The core answers with
    /// [`Event::ClipboardPasted`]: an image saved as an attachment, text, or a refusal.
    PasteClipboard {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
    },
    /// Text pasted into an agent's chat bar (a bracketed paste or a dropped file). The core
    /// answers with [`Event::ClipboardPasted`]: an image attachment when the text is a path to
    /// an existing image (`~`, `file://` and shell quoting understood), else the text itself.
    PasteText {
        /// The agent whose chat bar is pasting.
        agent: AgentId,
        /// What the terminal delivered.
        text: String,
    },
    /// List paths for `@` completion in an agent's chat bar. `dir: None` lists the files of the
    /// agent's folder (the main checkout or its worktree) that git does not ignore; `Some` lists
    /// one directory, `~/`, absolute, or relative to that folder. The core answers with
    /// [`Event::PathsListed`].
    ListPaths {
        /// The agent whose chat bar is completing.
        agent: AgentId,
        /// The directory, as typed after `@`, ending in `/`.
        dir: Option<String>,
    },
    /// Put text on the system clipboard (`c` in the worktree view copies a path). A failure
    /// comes back as [`Event::CommandFailed`]; success sends nothing.
    CopyToClipboard {
        /// The text.
        text: String,
    },
    /// Open a folder or file in the user's editor (`$EDITOR`, else `nvim`), in the terminal the
    /// UI runs in. The UI sends it only after it has let go of the terminal (left the alternate
    /// screen and raw mode, stopped reading keys), and takes the terminal back on
    /// [`Event::TerminalReturned`], which the core always sends once the editor exits or fails
    /// to start.
    OpenEditor {
        /// What to open; the editor also starts in it when it is a folder.
        path: PathBuf,
    },
    /// Stop an agent's current turn.
    Interrupt {
        /// The agent.
        agent: AgentId,
    },
    /// Answer a permission request.
    AnswerPermission {
        /// The inbox item.
        item: InboxId,
        /// The answer.
        answer: PermissionAnswer,
    },
    /// Answer a question.
    AnswerQuestion {
        /// The inbox item.
        item: InboxId,
        /// The answer.
        text: String,
    },
    /// Remove an inbox item without answering it.
    DismissInbox {
        /// The inbox item.
        item: InboxId,
    },
    /// Move a session to its next permission mode.
    CyclePermissionMode {
        /// The session.
        agent: AgentId,
    },
    /// Start a slice.
    NewSlice {
        /// The task it works on.
        task: Option<TaskId>,
        /// Slice name, also used for its branch.
        name: String,
        /// First prompt.
        brief: String,
    },
    /// End a slice session, or a side chat.
    CloseSlice {
        /// The slice or side chat.
        agent: AgentId,
    },
    /// Close the day (`:eod`, ADR 0012): ask the coordinator for its handoff, then write today's
    /// journal file with it and Dovetail's facts. The core answers with
    /// [`Event::JournalChanged`]: `closing` while it waits, then the new `latest`.
    EndOfDay,
    /// Open a side chat with `question` as its first prompt (ADR 0011).
    NewSideChat {
        /// The first prompt.
        question: String,
    },
    /// Hand a session to native `claude` in the UI's terminal (`<leader>st`, M14). Like
    /// [`Command::OpenEditor`], the UI sends it only after it has let go of the terminal, and
    /// takes the terminal back on [`Event::TerminalReturned`], which the core always sends: at
    /// once when it refuses, else when native exits. Headless resumes after.
    TakeOver {
        /// The session.
        agent: AgentId,
    },
    /// `:upgrade` (ADR 0014): build the next binary in the background. An "Upgrade ready" item
    /// appears in Needs you when it is done, an Error item when the build fails; a refusal comes
    /// back as [`Event::CommandFailed`].
    Upgrade {
        /// Build the project's checkout instead of `origin/main` (`:upgrade --local`).
        local: bool,
    },
    /// Create a task.
    CreateTask(NewTask),
    /// Change fields of a task.
    UpdateTask {
        /// The task.
        id: TaskId,
        /// Changes, applied in order.
        patch: Vec<TaskChange>,
    },
    /// Add an entry to a task's decision log, as the user.
    LogDecision(NewDecision),
    /// Archive a task.
    ArchiveTask {
        /// The task.
        id: TaskId,
    },
    /// Move a task up (negative) or down within its bucket.
    ReorderTask {
        /// The task.
        id: TaskId,
        /// Places to move.
        delta: i32,
    },
    /// Remove a slice's worktree.
    RemoveWorktree {
        /// The slice.
        agent: AgentId,
        /// The user has confirmed losing unpushed work.
        confirmed: bool,
    },
    /// Gather every slice worktree's git and PR state again (sent when the worktree view
    /// opens): `git fetch origin <base>` first, then local git and `gh pr list`. The core
    /// answers with one [`Event::WorktreeUpserted`] per worktree.
    RefreshWorktrees,
    /// Switch to another project.
    OpenProject {
        /// The project.
        id: ProjectId,
    },
    /// Register a repo as a project.
    RegisterProject {
        /// Path inside the repo.
        path: PathBuf,
    },
    /// Fetch conversation entries older than the tail.
    LoadConversation {
        /// Whose conversation.
        agent: AgentId,
        /// Fetch entries before this one; `None` means before the tail.
        before: Option<EntryId>,
    },
    /// Fetch a map's content. The core answers with [`Event::MapLoaded`], or
    /// [`Event::CommandFailed`] for an unknown map or version.
    LoadMap {
        /// The map.
        id: MapId,
        /// The version; `None` means the latest.
        version: Option<u32>,
    },
    /// Open a research round on a task (`:round <task> <brief>`, `docs/research.md` §5.1):
    /// subagents the coordinator spawns while it is open are filed under it. Refused while
    /// another round is open, for an unknown or archived task, an empty brief, or a folder that
    /// is not a project yet; the refusal comes back as [`Event::CommandFailed`].
    StartRound {
        /// The task (the question).
        task: TaskId,
        /// The user's criteria, written to the round's `brief.md`.
        brief: String,
    },
    /// Close the open research round of a task (`:round close`).
    CloseRound {
        /// The task.
        task: TaskId,
    },
    /// Fetch research content. The core answers with [`Event::ResearchLoaded`], or
    /// [`Event::CommandFailed`] when there is no such thing.
    LoadResearch {
        /// The task.
        task: TaskId,
        /// What to fetch.
        what: ResearchWhat,
    },
    /// Send a fresh snapshot (after a [`crate::ApplyResult::Gap`]).
    Resync,
    /// Shut down.
    Quit {
        /// Also end running sessions instead of leaving them to resume.
        end_sessions: bool,
    },
}

/// The user's answer to a permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAnswer {
    /// Allow this once.
    Allow,
    /// Allow, and add the offered rule.
    AllowAlways,
    /// Deny, optionally telling Claude why.
    Deny {
        /// Reason passed back to Claude.
        message: Option<String>,
    },
}

/// Fields for a new task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    /// One-line title.
    pub title: String,
    /// P-level, if given.
    pub p_level: Option<PLevel>,
    /// Bucket; `None` puts it in Unsorted.
    pub bucket: Option<Bucket>,
    /// Notes.
    #[serde(default)]
    pub notes: String,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Linked pull request number.
    pub pr: Option<u32>,
    /// Tasks it is related to (both ways).
    #[serde(default)]
    pub related: Vec<TaskId>,
}

/// Fields for a new decision-log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDecision {
    /// The task it belongs to.
    pub task: TaskId,
    /// The decision, in one line.
    pub text: String,
    /// Why, if given.
    pub why: Option<String>,
    /// Options considered and not taken.
    pub alternatives: Vec<Alternative>,
    /// The earlier entry it replaces.
    pub supersedes: Option<DecisionId>,
}

/// One field change in [`Command::UpdateTask`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskChange {
    /// New title.
    Title(String),
    /// New status.
    Status(TaskStatus),
    /// New P-level, or none.
    PLevel(Option<PLevel>),
    /// New bucket, or Unsorted.
    Bucket(Option<Bucket>),
    /// New notes.
    Notes(String),
    /// New dependencies.
    Deps(Vec<TaskId>),
    /// New linked PR, or none.
    Pr(Option<u32>),
    /// The user has seen a coordinator-created task.
    Seen,
    /// Relate this task to another, both ways.
    Relate(TaskId),
    /// Remove a relation, both ways.
    Unrelate(TaskId),
}

impl TaskChange {
    /// A few words for the Activity feed; `None` for changes that are not worth a line.
    pub fn describe(&self) -> Option<String> {
        Some(match self {
            Self::Title(t) => format!("renamed to \"{t}\""),
            Self::Status(s) => format!("status {s:?}").to_lowercase(),
            Self::PLevel(Some(p)) => format!("priority {p:?}"),
            Self::PLevel(None) => "priority cleared".to_owned(),
            Self::Bucket(Some(b)) => format!("moved to {b:?}"),
            Self::Bucket(None) => "moved to Unsorted".to_owned(),
            Self::Notes(_) => "notes edited".to_owned(),
            Self::Deps(_) => "dependencies changed".to_owned(),
            Self::Pr(Some(n)) => format!("linked PR #{n}"),
            Self::Pr(None) => "PR unlinked".to_owned(),
            Self::Relate(t) => format!("related to #{}", t.0),
            Self::Unrelate(t) => format!("unrelated from #{}", t.0),
            Self::Seen => return None,
        })
    }
}

/// The Activity text for a task patch, `None` when it holds nothing worth a line.
pub fn describe_task_patch(id: TaskId, title: &str, patch: &[TaskChange]) -> Option<String> {
    let parts: Vec<String> = patch.iter().filter_map(TaskChange::describe).collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("task #{} {title}: {}", id.0, parts.join(", ")))
}

/// The UI's end of a connection to a core.
///
/// Both the real core and `MockCore` hand one out. In v1 it is a pair of in-process channels;
/// a daemon would carry the same values as NDJSON over a unix socket.
#[derive(Debug)]
pub struct CoreLink {
    /// Commands to the core.
    pub commands: Sender<Command>,
    /// Messages from the core, starting with [`ServerMsg::Hello`].
    pub messages: Receiver<ServerMsg>,
}

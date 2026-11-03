//! Messages between the core and the UI.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use serde::{Deserialize, Serialize};

use crate::model::{
    ActivityItem, Agent, AgentId, AgentMessage, Bucket, ConversationEntry, EntryId, InboxId,
    InboxItem, PLevel, ProjectId, Snapshot, Task, TaskId, TaskStatus, TodoItem, TodoListId, Totals,
    Warning, WarningId,
};

/// Version of this protocol, sent in [`ServerMsg::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Everything the core sends: a hello, then a snapshot, then numbered events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// A warning was raised or reworded.
    WarningSet(Warning),
    /// A warning no longer applies.
    WarningCleared(WarningId),
    /// A command could not be carried out; the UI shows the message.
    CommandFailed {
        /// What went wrong.
        message: String,
    },
}

/// Something the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Send the user's prompt to an agent.
    Prompt {
        /// Recipient.
        agent: AgentId,
        /// Prompt text.
        text: String,
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
    /// End a slice session.
    CloseSlice {
        /// The slice.
        agent: AgentId,
    },
    /// Hand a session to a native terminal.
    TakeOver {
        /// The session.
        agent: AgentId,
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
    pub notes: String,
    /// The task it follows up.
    pub follow_up_of: Option<TaskId>,
    /// Linked pull request number.
    pub pr: Option<u32>,
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

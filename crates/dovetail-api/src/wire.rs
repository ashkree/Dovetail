//! What crosses between the core and a client: the stream, the commands, the events.

use std::sync::mpsc::{Receiver, Sender};

use serde::{Deserialize, Serialize};

use crate::agent::{AgentCommand, AgentEvent};
use crate::app::{AppCommand, AppEvent};
use crate::canvas::{CanvasCommand, CanvasEvent};
use crate::conversation::{ConversationCommand, ConversationEvent};
use crate::decision::{DecisionCommand, DecisionEvent};
use crate::design::{DesignCommand, DesignEvent};
use crate::effort::{EffortCommand, EffortEvent};
use crate::feed::ActivityEvent;
use crate::inbox::{InboxCommand, InboxEvent};
use crate::journal::{JournalCommand, JournalEvent};
use crate::library::{LibraryCommand, LibraryEvent};
use crate::map::{MapCommand, MapEvent};
use crate::mcp::{McpCommand, McpEvent};
use crate::model::{ModelCommand, ModelEvent};
use crate::project::{ProjectCommand, ProjectEvent};
use crate::research::{ResearchCommand, ResearchEvent};
use crate::snapshot::Snapshot;
use crate::status::StatusEvent;
use crate::tangent::{TangentCommand, TangentEvent};
use crate::task::{TaskCommand, TaskEvent};
use crate::worktree::{WorktreeCommand, WorktreeEvent};

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

/// Something the UI asks the core to do: one variant per feature, each holding that feature's
/// commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// A command about the user's tasks.
    Task(TaskCommand),
    /// A command about decision logs.
    Decision(DecisionCommand),
    /// A command about agents and their to-do lists.
    Agent(AgentCommand),
    /// A command about conversations and the chat bar.
    Conversation(ConversationCommand),
    /// A command about Needs you.
    Inbox(InboxCommand),
    /// A command about models.
    Model(ModelCommand),
    /// A command about the MCP menu.
    Mcp(McpCommand),
    /// A command about the journal.
    Journal(JournalCommand),
    /// A command about tangents.
    Tangent(TangentCommand),
    /// A command about canvases.
    Canvas(CanvasCommand),
    /// A command about design previews and variants.
    Design(DesignCommand),
    /// A command about efforts.
    Effort(EffortCommand),
    /// A command about worktrees.
    Worktree(WorktreeCommand),
    /// A command about projects.
    Project(ProjectCommand),
    /// A command about maps.
    Map(MapCommand),
    /// A command about research.
    Research(ResearchCommand),
    /// A command about the library.
    Library(LibraryCommand),
    /// A command about the app.
    App(AppCommand),
}

/// One change to the client's [`Snapshot`], or an answer to a command: one variant per feature,
/// each holding that feature's events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// An event about the user's tasks.
    Task(TaskEvent),
    /// An event about efforts.
    Effort(EffortEvent),
    /// An event about design previews and variants.
    Design(DesignEvent),
    /// An event about canvases.
    Canvas(CanvasEvent),
    /// An event about agents and their to-do lists.
    Agent(AgentEvent),
    /// An event about Needs you.
    Inbox(InboxEvent),
    /// An event about worktrees.
    Worktree(WorktreeEvent),
    /// An event about the Activity feed.
    Activity(ActivityEvent),
    /// An event about conversations and the chat bar.
    Conversation(ConversationEvent),
    /// An event about the status bar.
    Status(StatusEvent),
    /// An event about decision logs.
    Decision(DecisionEvent),
    /// An event about maps.
    Map(MapEvent),
    /// An event about research.
    Research(ResearchEvent),
    /// An event about tangents.
    Tangent(TangentEvent),
    /// An event about the library.
    Library(LibraryEvent),
    /// An event about models.
    Model(ModelEvent),
    /// An event about the MCP menu.
    Mcp(McpEvent),
    /// An event about the app.
    App(AppEvent),
    /// An event about the journal.
    Journal(JournalEvent),
    /// An event about projects.
    Project(ProjectEvent),
}

//! The `conversation` feature's events.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::conversation::{ConversationEntry, EntryId};

/// A change to the client's snapshot, or an answer, in the feature `conversation`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationEvent {
    /// A conversation entry was added or replaced.
    Appended {
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
    /// Older entries, in answer to [`ConversationCommand::Load`].
    Page {
        /// Whose conversation.
        agent: AgentId,
        /// Entries older than the tail, oldest first.
        entries: Vec<ConversationEntry>,
        /// Still older entries exist.
        has_more: bool,
    },
    /// Paths for `@` completion in a chat bar, in answer to [`ConversationCommand::ListPaths`]. Changes
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
}

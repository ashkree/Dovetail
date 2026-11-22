//! The `conversation` feature's commands.

use crate::agent::AgentId;
use crate::conversation::{Attachment, EntryId};
use crate::effort::EffortId;
use serde::{Deserialize, Serialize};

/// Conversations: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationCommand {
    /// Send the user's prompt to an agent.
    Prompt {
        /// Recipient.
        agent: AgentId,
        /// Prompt text; may be empty when images are attached.
        text: String,
        /// Images to send with it, saved by the core on [`crate::app::AppCommand::Paste`]; the core
        /// reads the files.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
        /// The effort tag for this message and the agent's replies to it; `None` sends it
        /// untagged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<EffortId>,
    },
    /// List paths for `@` completion in an agent's chat bar. `dir: None` lists the files of the
    /// agent's folder (the main checkout or its worktree) that git does not ignore; `Some` lists
    /// one directory, `~/`, absolute, or relative to that folder. The core answers with
    /// [`super::ConversationEvent::PathsListed`].
    ListPaths {
        /// The agent whose chat bar is completing.
        agent: AgentId,
        /// The directory, as typed after `@`, ending in `/`.
        dir: Option<String>,
    },
    /// Fetch conversation entries older than the tail.
    Load {
        /// Whose conversation.
        agent: AgentId,
        /// Fetch entries before this one; `None` means before the tail.
        before: Option<EntryId>,
    },
}

//! The `conversation` feature: conversations: each agent's entries, the chat bar's prompts and `@` completion.

mod apply;
mod attachment;
mod command;
mod event;
pub(crate) use apply::apply;
pub use attachment::{MAX_ATTACHMENTS, MAX_ATTACHMENT_BYTES, size_text, attachment_limit_text, Attachment, ImageType};
pub use command::ConversationCommand;
pub use event::ConversationEvent;

use serde::{Deserialize, Serialize};
use crate::activity::AgentMessage;
use crate::conversation::Attachment;
use crate::effort::EffortId;

/// Identifies a conversation entry. Ids grow with time within one agent's conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(pub u64);

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
    /// The effort tag: the line of work this entry belongs to. `None` for entries from before
    /// efforts, and for ones that belong to none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<EffortId>,
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
    /// A message another agent sent this one, such as a subagent's hand-back. The text is the
    /// header line from [`agent_message_text`] and then the body; read it with
    /// [`ConversationEntry::agent_message`].
    AgentMessage,
}

/// The text of an [`EntryKind::AgentMessage`] entry: a header line naming the sender
/// (`subagent <name>` or `agent <name>`), then the message body.
pub fn agent_message_text(from: &str, subagent: bool, body: &str) -> String {
    let who = if subagent { "subagent" } else { "agent" };
    format!("{who} {from}\n{body}")
}

/// A parsed [`EntryKind::AgentMessage`] entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentMessageParts<'a> {
    /// Name of the sending agent.
    pub from: &'a str,
    /// Whether the sender is a subagent.
    pub subagent: bool,
    /// The message itself.
    pub body: &'a str,
}

impl ConversationEntry {
    /// The sender and body of an agent message; `None` for every other kind. A text without
    /// the header line reads as a message from an unknown agent.
    pub fn agent_message(&self) -> Option<AgentMessageParts<'_>> {
        if self.kind != EntryKind::AgentMessage {
            return None;
        }
        let unknown = AgentMessageParts {
            from: "agent",
            subagent: false,
            body: &self.text,
        };
        let Some((head, body)) = self.text.split_once('\n') else {
            return Some(unknown);
        };
        Some(match head.split_once(' ') {
            Some(("subagent", from)) => AgentMessageParts {
                from,
                subagent: true,
                body,
            },
            Some(("agent", from)) => AgentMessageParts {
                from,
                subagent: false,
                body,
            },
            _ => unknown,
        })
    }
}

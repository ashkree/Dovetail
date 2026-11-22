//! The `feed` feature: the Activity feed and the messages agents send each other.

mod apply;
mod event;
pub(crate) use apply::apply;
pub use event::ActivityEvent;

use crate::agent::AgentId;
use serde::{Deserialize, Serialize};

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

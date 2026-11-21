//! The `agent` feature's events.

use serde::{Deserialize, Serialize};
use crate::agent::{Agent, AgentId, SlashCommand, TodoItem, TodoListId};

/// A change to the client's snapshot, or an answer, in the feature `agent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvent {
    /// An agent was created or changed.
    Upserted(Agent),
    /// An agent was removed, with its conversation.
    Removed(AgentId),
    /// A native to-do list was rewritten.
    TodosReplaced {
        /// The list.
        list: TodoListId,
        /// Its items now.
        items: Vec<TodoItem>,
    },
    /// A session listed its slash commands and skills (at start, and when they change).
    CommandsChanged {
        /// The session.
        agent: AgentId,
        /// Everything it offers, in its own order.
        commands: Vec<SlashCommand>,
    },
}

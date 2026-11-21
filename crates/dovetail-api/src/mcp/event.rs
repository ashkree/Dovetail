//! The `mcp` feature's events.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::mcp::{McpOutcome, McpRequest};

/// A change to the client's snapshot, or an answer, in the feature `mcp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpEvent {
    /// A session's answer to a [`McpCommand::Request`] (task #44). Changes nothing in the snapshot; the
    /// open MCP menu takes it in.
    Reply {
        /// The session asked.
        agent: AgentId,
        /// What was asked, so the menu knows which action this finishes.
        request: McpRequest,
        /// The answer.
        outcome: McpOutcome,
    },
}

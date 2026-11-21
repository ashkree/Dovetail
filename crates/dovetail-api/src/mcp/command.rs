//! The `mcp` feature's commands.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::mcp::McpRequest;

/// The MCP menu: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCommand {
    /// One request of the MCP menu to a session (task #44): its servers, or an action on one.
    /// The answer is [`McpEvent::Reply`]; a session that is not running answers
    /// [`crate::app::AppEvent::CommandFailed`].
    Request {
        /// The session; a subagent's is its parent's.
        agent: AgentId,
        /// What to ask.
        request: McpRequest,
    },
}

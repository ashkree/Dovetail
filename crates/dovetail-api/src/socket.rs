//! Messages on the app's unix socket (`$XDG_RUNTIME_DIR/dovetail/<project-id>.sock`).
//!
//! Clients are `dovetail add` and the per-session `dovetail mcp` bridges. Framing is NDJSON: one
//! [`SocketRequest`] per line from the client, one [`SocketResponse`] per line back, matched on
//! `id`. Responses may arrive out of order, since a held tool call (a blocking `ask_user`) must not
//! stop other requests on the same connection.

use serde::{Deserialize, Serialize};

use crate::tools::Tool;
use crate::{AgentId, NewTask, TaskId};

/// One request from a socket client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketRequest {
    /// Chosen by the client, unique per connection; echoed in the response.
    pub id: u64,
    /// What is asked.
    pub body: SocketRequestBody,
}

/// What a socket client asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SocketRequestBody {
    /// `dovetail add`: create a task from a parsed quick-capture line.
    AddTask {
        /// The task.
        task: NewTask,
    },
    /// An MCP `tools/call` forwarded by a session's bridge.
    ToolCall {
        /// The calling session, from the bridge's `DOVETAIL_AGENT_ID`.
        agent: AgentId,
        /// `params._meta["claudecode/toolUseId"]`, matched to the stream's `parent_tool_use_id`
        /// to attribute subagent calls (spike S3).
        tool_use_id: Option<String>,
        /// The tool and its arguments, parsed and validated by the bridge.
        call: Tool,
    },
}

/// One response to a [`SocketRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketResponse {
    /// The request's `id`.
    pub id: u64,
    /// The outcome.
    pub body: SocketResponseBody,
}

/// The outcome of a socket request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SocketResponseBody {
    /// The task was created.
    TaskAdded {
        /// Its id.
        id: TaskId,
    },
    /// A tool call finished; the bridge turns this into an MCP `CallToolResult`.
    ToolResult {
        /// Text content returned to the agent.
        text: String,
        /// Sets `isError` on the MCP result.
        is_error: bool,
    },
    /// The request could not be handled at all.
    Error {
        /// What went wrong.
        message: String,
    },
}

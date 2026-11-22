//! The `agent` feature's commands.

use crate::agent::AgentId;
use crate::task::TaskId;
use serde::{Deserialize, Serialize};

/// Agents: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCommand {
    /// Stop an agent's current turn.
    Interrupt {
        /// The agent.
        agent: AgentId,
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
        /// The repo it works in, by name (ADR 0021). May be left out when the project has one.
        #[serde(default)]
        repo: Option<String>,
    },
    /// End a slice session, or a side chat.
    CloseSlice {
        /// The slice or side chat.
        agent: AgentId,
    },
    /// Open a side chat with `question` as its first prompt (ADR 0011).
    NewSideChat {
        /// The first prompt.
        question: String,
    },
}

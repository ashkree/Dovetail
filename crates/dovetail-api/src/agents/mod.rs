//! The `agents` domain: agents, their conversations, models, MCP servers, tangents and the journal.
//!
//! A folder only: each feature below keeps its own `Command` and `Event`.

pub mod agent;
pub mod conversation;
pub mod journal;
pub mod mcp;
pub mod model;
pub mod tangent;

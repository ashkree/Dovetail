//! MCP infrastructure: Unix socket server (`socket`) and tool call dispatch (`tools`).

pub mod socket;
pub(crate) mod tools;

use std::path::PathBuf;

/// Unix sockets owned by the core for `dovetail add` and session MCP bridges.
pub(crate) struct McpState {
    /// The socket the bridges of this app's sessions use.
    pub(crate) socket: PathBuf,
    /// Sockets bound since start (after registration), removed on exit.
    pub(crate) extra_sockets: Vec<PathBuf>,
}

impl McpState {
    pub(crate) fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            extra_sockets: Vec::new(),
        }
    }
}

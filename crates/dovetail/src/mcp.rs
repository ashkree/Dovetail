//! `dovetail mcp`: the per-session stdio MCP bridge, forwarding tool calls to the app socket.

use anyhow::{Result, bail};

/// Run the subcommand with the arguments after its name.
pub fn run(_args: Vec<String>) -> Result<()> {
    bail!("`mcp` is not built yet")
}

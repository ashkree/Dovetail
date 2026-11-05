//! `dovetail hook worktree-create`: the `WorktreeCreate` hook (idempotent; symlinks and recipe setup; prints the path).

use anyhow::{Result, bail};

/// Run the subcommand with the arguments after its name.
pub fn run(_args: Vec<String>) -> Result<()> {
    bail!("`hook` is not built yet")
}

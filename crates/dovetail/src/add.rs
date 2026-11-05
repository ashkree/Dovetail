//! `dovetail add "title !p1 @now"`: quick capture from the shell (B19: socket if the app runs, else SQLite).

use anyhow::{Result, bail};

/// Run the subcommand with the arguments after its name.
pub fn run(_args: Vec<String>) -> Result<()> {
    bail!("`add` is not built yet")
}

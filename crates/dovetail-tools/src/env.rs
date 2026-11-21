//! Names of the environment variables Dovetail passes to the processes it launches.

/// The session's agent id; set for the MCP bridge and the worktree hook.
pub const AGENT_ID: &str = "DOVETAIL_AGENT_ID";
/// Path of the app's unix socket; set for the MCP bridge.
pub const SOCKET: &str = "DOVETAIL_SOCKET";
/// `coordinator`, `slice` or `side-chat` (see [`crate::Role`]); set for the MCP bridge.
pub const ROLE: &str = "DOVETAIL_ROLE";
/// The session's suites, comma-separated (see [`crate::Suite::join`]); set for the MCP
/// bridge. Without it the bridge uses the role's defaults.
pub const SUITES: &str = "DOVETAIL_SUITES";
/// The project id; set for the worktree hook.
pub const PROJECT: &str = "DOVETAIL_PROJECT";
/// A slice's repo name (ADR 0021); set for the worktree hook, which applies the recipe's
/// `[repos.<name>]` overrides.
pub const REPO: &str = "DOVETAIL_REPO";
/// Replaces `$XDG_DATA_HOME/dovetail` (databases, worktrees). With the three below, lets a
/// dev build in a slice run on its own state without touching Claude Code's environment.
pub const DATA_DIR: &str = "DOVETAIL_DATA_DIR";
/// Replaces `$XDG_CONFIG_HOME/dovetail` (config, role prompts, recipes).
pub const CONFIG_DIR: &str = "DOVETAIL_CONFIG_DIR";
/// Replaces `$XDG_STATE_HOME/dovetail` (logs, launch files, setup state).
pub const STATE_DIR: &str = "DOVETAIL_STATE_DIR";
/// Replaces `$XDG_RUNTIME_DIR/dovetail` (sockets; keep it short, ~108-byte limit).
pub const RUNTIME_DIR: &str = "DOVETAIL_RUNTIME_DIR";

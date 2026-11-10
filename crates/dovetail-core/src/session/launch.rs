//! Launch settings: everything a session gets is passed as flags and files outside the repo
//! (plan §4). Generated `settings.json` and `mcp.json` go in `Paths::run_dir`.
//!
//! - `--settings`: the `WorktreeCreate` hook (15-minute timeout, B5), a no-op `WorktreeRemove`
//!   hook (A9), `crossSessionInbound: accept`, and an ask rule for `gh pr merge` (B7).
//! - `--mcp-config`: `dovetail mcp` with the agent id, socket and role in its environment (B18).
//!   Never `--strict-mcp-config`, so the user's own MCP servers still load (B28).
//! - `--append-system-prompt-file`: the installed role prompt (Decision 8c).
//! - `--allowedTools`: Dovetail's tools for the role, plus `SendMessage` and `ListAgents` (S2).
//! - Environment: native to-do tools on, the to-do list id (A12), the hook's project id, and
//!   `CLAUDE_CONFIG_DIR` when the project picks a Claude profile (ADR 0009).
//!
//! Never bare mode: it disables the messaging socket.

use std::path::{Path, PathBuf};

use dovetail_api::tools::{Role, TOOLS, Tool, env};
use dovetail_driver::{PermissionMode as CliMode, SessionSpec};
use serde_json::json;

use crate::paths::Paths;

/// Records each session's stream in the fixture format next to its launch files, for capturing
/// new fixtures in live checks. Off unless set to `1`.
pub const RECORD_ENV: &str = "DOVETAIL_RECORD_STREAMS";

/// What a launch needs to know about the session.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Project id, or the app's key for an unregistered folder.
    pub project_key: String,
    /// Agent id, also the run directory's name.
    pub agent_id: String,
    /// Display name, `--name`, and the name other sessions message it by.
    pub name: String,
    /// Which tools it gets.
    pub role: Role,
    /// The Claude Code session id.
    pub session_id: String,
    /// Resume the session instead of creating it.
    pub resume: bool,
    /// Working directory.
    pub cwd: PathBuf,
    /// `--worktree <name>`, for a new slice in a git project.
    pub worktree: Option<String>,
    /// Starting permission mode.
    pub mode: CliMode,
    /// `CLAUDE_CODE_TASK_LIST_ID`.
    pub task_list_id: String,
    /// The app's socket, for the MCP bridge.
    pub socket: PathBuf,
    /// The `dovetail` binary, for the hook and the bridge.
    pub exe: PathBuf,
    /// `--model`.
    pub model: Option<String>,
    /// Extra environment (a slice's recipe `[env]`).
    pub env: Vec<(String, String)>,
    /// `CLAUDE_CONFIG_DIR`: the project's profile folder; `None` to inherit Dovetail's own.
    pub claude_config_dir: Option<PathBuf>,
}

/// Writes the launch files and returns the spec. The role prompt is passed only if installed.
pub fn spec(paths: &Paths, launch: &Launch) -> std::io::Result<SessionSpec> {
    let dir = paths.run_dir(&launch.project_key, &launch.agent_id);
    std::fs::create_dir_all(&dir)?;
    // The hook and the bridge are this app's binary pinned to this app's state: a slice's
    // session may carry `DOVETAIL_*_DIR` for its own dev builds (recipe `[env]`), and they
    // must not follow it.
    let pinned = paths.pinned_env();
    let pin: String = pinned
        .iter()
        .map(|(k, v)| format!("{k}={} ", shell_quote(&v.to_string_lossy())))
        .collect();
    let exe = shell_quote(&launch.exe.to_string_lossy());

    let settings_path = dir.join("settings.json");
    let settings = json!({
        "crossSessionInbound": "accept",
        "permissions": { "ask": ["Bash(gh pr merge:*)"] },
        "hooks": {
            "WorktreeCreate": [{ "hooks": [{
                "type": "command",
                "command": format!("env {pin}{exe} hook worktree-create"),
                "timeout": 900
            }]}],
            "WorktreeRemove": [{ "hooks": [{
                "type": "command",
                "command": format!("env {pin}{exe} hook worktree-remove"),
                "timeout": 30
            }]}]
        }
    });
    write_json(&settings_path, &settings)?;

    let mcp_path = dir.join("mcp.json");
    let mut bridge_env = json!({
        env::AGENT_ID: launch.agent_id,
        env::SOCKET: launch.socket,
        env::ROLE: launch.role.as_str(),
    });
    for (k, v) in &pinned {
        bridge_env[*k] = json!(v);
    }
    let mcp = json!({ "mcpServers": { "dovetail": {
        "command": launch.exe,
        "args": ["mcp"],
        "env": bridge_env,
    }}});
    write_json(&mcp_path, &mcp)?;

    let role_prompt = paths.role_prompt(launch.role.as_str());

    let mut spec = SessionSpec::new(launch.session_id.clone(), launch.cwd.clone());
    spec.resume = launch.resume;
    spec.name = Some(launch.name.clone());
    spec.model = launch.model.clone();
    spec.permission_mode = launch.mode;
    spec.settings_path = Some(settings_path);
    spec.mcp_config_path = Some(mcp_path);
    spec.append_system_prompt_file = role_prompt.exists().then_some(role_prompt);
    spec.allowed_tools = allowed_tools(launch.role);
    // A side chat reads and searches; it never edits (ADR 0011).
    if launch.role == Role::SideChat {
        spec.allowed_tools
            .extend(["WebSearch", "WebFetch"].map(String::from));
        spec.extra_args
            .extend(["--disallowedTools", "Edit,Write,NotebookEdit"].map(String::from));
    }
    spec.worktree = launch.worktree.clone();
    spec.env = vec![
        ("CLAUDE_CODE_ENABLE_TODO_TOOLS".into(), "1".into()),
        (
            "CLAUDE_CODE_TASK_LIST_ID".into(),
            launch.task_list_id.clone(),
        ),
        (env::AGENT_ID.into(), launch.agent_id.clone()),
    ];
    // The worktree hook names worktrees and finds the recipe by it (`new-<pid>` before
    // registration: no recipe then).
    spec.env
        .push((env::PROJECT.into(), launch.project_key.clone()));
    spec.env.extend(launch.env.iter().cloned());
    // After the recipe's `[env]`, so the profile always wins.
    if let Some(dir) = &launch.claude_config_dir {
        spec.env.retain(|(k, _)| k != "CLAUDE_CONFIG_DIR");
        spec.env.push((
            "CLAUDE_CONFIG_DIR".into(),
            dir.to_string_lossy().into_owned(),
        ));
    }
    if std::env::var(RECORD_ENV).is_ok_and(|v| v == "1") {
        spec.log_path = Some(dir.join(format!("stream-{}.ndjson", crate::db::now_ms())));
    }
    Ok(spec)
}

/// Dovetail's tools for the role as Claude Code names them, plus native messaging.
pub fn allowed_tools(role: Role) -> Vec<String> {
    let mut tools: Vec<String> = TOOLS
        .iter()
        .filter(|t| Tool::allowed(t.name, role))
        .map(|t| format!("mcp__dovetail__{}", t.name))
        .collect();
    tools.push("SendMessage".into());
    tools.push("ListAgents".into());
    tools
}

/// Where Claude Code files a session's transcript: `<claude home>/projects/<cwd with every
/// character but letters and digits turned into '-'>/<session id>.jsonl` (spike S4: under the
/// worktree for a slice). The home is the session's profile folder (ADR 0009), by default
/// `$CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn transcript_path(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    let encoded: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join("projects")
        .join(encoded)
        .join(format!("{session_id}.jsonl"))
}

/// Single-quotes a word for `sh`.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn write_json(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_are_filed_under_the_encoded_cwd() {
        let path = transcript_path(
            Path::new("/p/maveron"),
            Path::new("/tmp/dovetail-spikes/driver/repo"),
            "abc",
        );
        assert_eq!(
            path,
            Path::new("/p/maveron/projects/-tmp-dovetail-spikes-driver-repo/abc.jsonl")
        );
    }

    #[test]
    fn slices_get_no_coordinator_tools() {
        let slice = allowed_tools(Role::Slice);
        assert!(slice.contains(&"mcp__dovetail__ask_user".to_string()));
        assert!(!slice.iter().any(|t| t.ends_with("start_slice")));
        assert!(
            allowed_tools(Role::Coordinator).contains(&"mcp__dovetail__add_project".to_string())
        );
    }
}

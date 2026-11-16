//! Launch settings: everything a session gets is passed as flags and files outside the repo
//! (plan §4). Generated `settings.json` and `mcp.json` go in `Paths::run_dir`.
//!
//! - `--settings`: the `WorktreeCreate` hook (15-minute timeout, B5), a no-op `WorktreeRemove`
//!   hook (A9), `crossSessionInbound: accept`, and an ask rule for `gh pr merge` (B7).
//! - `--mcp-config`: `dovetail mcp` with the agent id, socket, role and suites in its environment
//!   (B18, ADR 0016). Never `--strict-mcp-config`, so the user's own MCP servers still load (B28).
//! - `--append-system-prompt-file`: the installed role prompt (Decision 8c).
//! - `--allowedTools`: Dovetail's tools in the session's suites and, for one release, their old
//!   names (ADR 0016), plus `SendMessage` and `ListAgents` (S2).
//! - A side chat (ADR 0011, read-only per the 2026-11-11 decision), and reading mode's Ask
//!   (ADR 0018), which has no suites: `--permission-mode dontAsk`,
//!   web tools and the read-only Bash commands of [`SIDE_CHAT_BASH`] allowed, and
//!   `--disallowedTools` for the editing tools and the options of [`SIDE_CHAT_BASH_DENY`].
//! - Environment: native to-do tools on, the to-do list id (A12), the hook's project id, and
//!   `CLAUDE_CONFIG_DIR` when the project picks a Claude profile (ADR 0009).
//!
//! Never bare mode: it disables the messaging socket.

use std::path::{Path, PathBuf};

use dovetail_api::tools::{Role, Suite, env, tools_in};
use dovetail_driver::{PermissionMode as CliMode, SessionSpec};
use serde_json::json;

use crate::paths::Paths;

/// Records each session's stream in the fixture format next to its launch files, for capturing
/// new fixtures in live checks. Off unless set to `1`.
pub const RECORD_ENV: &str = "DOVETAIL_RECORD_STREAMS";

/// The Bash commands a side chat may run, each with any arguments: they read and never write.
/// In `dontAsk` mode every other command is denied without asking (decided 2026-11-11).
/// `cargo tree` and `cargo metadata` are allowed only with `--locked` (they could otherwise
/// rewrite `Cargo.lock` or fetch), so a call that leaves it out is denied.
pub const SIDE_CHAT_BASH: &[&str] = &[
    "git log",
    "git show",
    "git diff",
    "git status",
    "git blame",
    "rg",
    "grep",
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "find",
    "cargo tree --locked",
    "cargo metadata --locked",
];

/// Options of [`SIDE_CHAT_BASH`] commands that write files or run other commands, denied even
/// though the command's prefix is allowed (deny rules win over allow rules). `*` matches any
/// text, spaces included.
pub const SIDE_CHAT_BASH_DENY: &[&str] = &[
    "find *-exec*",
    "find *-ok*",
    "find *-delete*",
    "find *-fprint*",
    "find *-fls*",
    "rg *--pre*",
    "git *--output*",
];

/// Tools a side chat never gets: it cannot change files.
pub const SIDE_CHAT_DENIED_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit"];

/// The `--allowedTools` a side chat adds to its role's: the web, and the read-only Bash
/// commands as `Bash(<command> *)`, which also matches the bare command.
pub fn side_chat_allowed() -> Vec<String> {
    ["WebSearch", "WebFetch"]
        .into_iter()
        .map(String::from)
        .chain(SIDE_CHAT_BASH.iter().map(|c| format!("Bash({c} *)")))
        .collect()
}

/// The `--disallowedTools` of a side chat.
pub fn side_chat_disallowed() -> Vec<String> {
    SIDE_CHAT_DENIED_TOOLS
        .iter()
        .map(|t| (*t).to_string())
        .chain(SIDE_CHAT_BASH_DENY.iter().map(|c| format!("Bash({c})")))
        .collect()
}

/// What a launch needs to know about the session.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Project id, or the app's key for an unregistered folder.
    pub project_key: String,
    /// Agent id, also the run directory's name.
    pub agent_id: String,
    /// Display name, `--name`, and the name other sessions message it by.
    pub name: String,
    /// Its role prompt and permission rules.
    pub role: Role,
    /// The installed role prompt to read instead of the role's own: a tangent has side chats'
    /// role and rules but its own prompt, `tangent` (ADR 0017).
    pub prompt: Option<&'static str>,
    /// The tool suites it gets (`config.toml` `[suites]`, ADR 0016).
    pub suites: Vec<Suite>,
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
    /// `--effort`.
    pub effort: Option<dovetail_api::ReasoningEffort>,
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
        env::SUITES: Suite::join(&launch.suites),
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

    let role_prompt = paths.role_prompt(launch.prompt.unwrap_or(launch.role.as_str()));

    let mut spec = SessionSpec::new(launch.session_id.clone(), launch.cwd.clone());
    spec.resume = launch.resume;
    spec.name = Some(launch.name.clone());
    spec.model = launch.model.clone();
    spec.effort = launch.effort;
    spec.permission_mode = launch.mode;
    spec.settings_path = Some(settings_path);
    spec.mcp_config_path = Some(mcp_path);
    spec.append_system_prompt_file = role_prompt.exists().then_some(role_prompt);
    spec.allowed_tools = allowed_tools(&launch.suites);
    // A side chat reads and searches; it never edits, and runs only read-only commands. In
    // `dontAsk` mode anything not allowed here is denied, never asked (decided 2026-11-11).
    if matches!(launch.role, Role::SideChat | Role::Ask) {
        spec.permission_mode = CliMode::DontAsk;
        spec.allowed_tools.extend(side_chat_allowed());
        spec.extra_args.extend([
            "--disallowedTools".to_string(),
            side_chat_disallowed().join(","),
        ]);
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

/// Dovetail's tools in `suites` as Claude Code names them, plus native messaging. Old names are
/// allowed too for one release, so a resumed conversation that calls one still works (ADR 0016).
pub fn allowed_tools(suites: &[Suite]) -> Vec<String> {
    let specs = || tools_in(suites);
    let mut tools: Vec<String> = specs()
        .map(|t| t.name)
        .chain(specs().filter_map(|t| t.old_name))
        .map(|name| format!("mcp__dovetail__{name}"))
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
    fn side_chats_allow_only_read_only_bash() {
        let allowed = side_chat_allowed();
        for rule in [
            "Bash(git log *)",
            "Bash(git show *)",
            "Bash(git diff *)",
            "Bash(git status *)",
            "Bash(git blame *)",
            "Bash(rg *)",
            "Bash(grep *)",
            "Bash(ls *)",
            "Bash(cat *)",
            "Bash(head *)",
            "Bash(tail *)",
            "Bash(wc *)",
            "Bash(find *)",
            "Bash(cargo tree --locked *)",
            "Bash(cargo metadata --locked *)",
            "WebSearch",
            "WebFetch",
        ] {
            assert!(allowed.contains(&rule.to_string()), "{rule}");
        }
        assert_eq!(allowed.len(), 17, "nothing else is allowed: {allowed:?}");
        // Without `--locked` they could rewrite the lockfile or fetch.
        for bare in ["Bash(cargo tree *)", "Bash(cargo metadata *)"] {
            assert!(!allowed.contains(&bare.to_string()), "{bare}");
        }
        let denied = side_chat_disallowed();
        for rule in [
            "Edit",
            "Write",
            "NotebookEdit",
            "Bash(find *-exec*)",
            "Bash(find *-delete*)",
            "Bash(rg *--pre*)",
            "Bash(git *--output*)",
        ] {
            assert!(denied.contains(&rule.to_string()), "{rule}");
        }
    }

    /// Every role gets exactly its suites' tools, new names first, then the old ones (M20).
    #[test]
    fn roles_get_exactly_their_suites() {
        let names = |role: Role| -> Vec<String> {
            allowed_tools(role.default_suites())
                .into_iter()
                .map(|t| t.trim_start_matches("mcp__dovetail__").to_owned())
                .collect()
        };
        let core = ["core_ask_user", "core_report_status", "core_log_decision"];
        let core_old = ["ask_user", "report_status", "log_decision"];
        let messaging = ["SendMessage", "ListAgents"];
        let side_chat: Vec<&str> = [&core[..], &core_old, &messaging].concat();
        assert_eq!(names(Role::SideChat), side_chat);
        let slice: Vec<&str> = [
            &core[..],
            &["maps_draw"],
            &core_old,
            &["draw_map"],
            &messaging,
        ]
        .concat();
        assert_eq!(names(Role::Slice), slice);
        let coordinator: Vec<&str> = [
            &core[..],
            &[
                "tasks_create",
                "tasks_list",
                "tasks_get",
                "tasks_update",
                "tasks_archive",
                "project_start_slice",
                "project_archive_slice",
                "project_add",
                "research_start_round",
                "research_write_report",
                "research_propose",
                "research_get",
                "maps_draw",
            ],
            &core_old,
            &[
                "create_task",
                "list_tasks",
                "get_task",
                "update_task",
                "archive_task",
                "start_slice",
                "archive_slice",
                "add_project",
                "draw_map",
            ],
            &messaging,
        ]
        .concat();
        assert_eq!(names(Role::Coordinator), coordinator);
    }
}

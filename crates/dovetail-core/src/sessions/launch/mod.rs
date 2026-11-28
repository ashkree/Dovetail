//! Launch settings: everything a session gets is passed as flags and files outside the repo
//! (plan §4). Generated driver files go in `Paths::run_dir`.
//!
//! - Claude Code (`claude`): `--settings` (`WorktreeCreate`/`WorktreeRemove` hooks,
//!   `crossSessionInbound: accept`, `gh pr merge` ask rule), `--mcp-config`, `--allowedTools`,
//!   `--append-system-prompt-file`, side-chat `--permission-mode dontAsk` + `--disallowedTools`,
//!   and `CLAUDE_*` / `CLAUDE_CONFIG_DIR` environment.
//! - agy (`agy`): `--cli_dir_name` holding `settings.json` (`{"toolPermission":"request-review"}`),
//!   inline `mcpServers` in `session/new`, `mcp_dovetail_*` pre-allowed in the driver,
//!   `_meta.antigravity.systemPrompt.append`, and `DontAsk` for side chats.
//!
//! Never bare mode: it disables the messaging socket.

mod agy;
mod claude;

use std::path::{Path, PathBuf};

use dovetail_api::agents::model::DriverKind;
use dovetail_driver::{PermissionMode as CliMode, SessionSpec};
use dovetail_tools::{Role, Suite, env, tools_in};

use crate::paths::{Paths, home_dir};

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
    /// The session id (Dovetail's `--session-id` for Claude Code, or agy's conversation id on resume).
    pub session_id: String,
    /// Resume the session instead of creating it.
    pub resume: bool,
    /// Working directory.
    pub cwd: PathBuf,
    /// `--worktree <name>`, for a new slice in a git project (Claude Code only).
    pub worktree: Option<String>,
    /// `--add-dir` / `additionalDirectories`: the repos of a session in the coordinator's folder (ADR 0021).
    pub add_dirs: Vec<PathBuf>,
    /// Starting permission mode.
    pub mode: CliMode,
    /// `CLAUDE_CODE_TASK_LIST_ID`.
    pub task_list_id: String,
    /// The app's socket, for the MCP bridge.
    pub socket: PathBuf,
    /// The `dovetail` binary, for the hook and the bridge.
    pub exe: PathBuf,
    /// `--model` / model family.
    pub model: Option<String>,
    /// `--effort` / thought level.
    pub effort: Option<dovetail_api::agents::model::ReasoningEffort>,
    /// Extra environment (a slice's recipe `[env]`).
    pub env: Vec<(String, String)>,
    /// `CLAUDE_CONFIG_DIR`: the project's profile folder; `None` to inherit Dovetail's own.
    pub claude_config_dir: Option<PathBuf>,
}

/// Writes the launch files for `driver` and returns the spec. The role prompt is passed only if
/// installed.
pub fn spec(paths: &Paths, launch: &Launch, driver: DriverKind) -> std::io::Result<SessionSpec> {
    let dir = paths.run_dir(&launch.project_key, &launch.agent_id);
    std::fs::create_dir_all(&dir)?;

    let role_prompt = paths.role_prompt(launch.prompt.unwrap_or(launch.role.as_str()));
    let mut spec = SessionSpec::new(launch.session_id.clone(), launch.cwd.clone());
    spec.resume = launch.resume;
    spec.name = Some(launch.name.clone());
    spec.model = launch.model.clone();
    spec.effort = launch.effort;
    // A side chat reads and searches; it never edits. In `dontAsk` mode anything not allowed is
    // denied without asking (decided 2026-11-11).
    spec.permission_mode = if matches!(launch.role, Role::SideChat | Role::Ask) {
        CliMode::DontAsk
    } else {
        launch.mode
    };
    spec.append_system_prompt_file = role_prompt.exists().then_some(role_prompt);
    spec.add_dirs = launch.add_dirs.clone();
    if std::env::var(RECORD_ENV).is_ok_and(|v| v == "1") {
        spec.log_path = Some(dir.join(format!("stream-{}.ndjson", crate::storage::now_ms())));
    }

    match driver {
        DriverKind::Claude => claude::configure(paths, launch, &dir, &mut spec)?,
        DriverKind::Agy => agy::configure(paths, launch, &dir, &mut spec)?,
    }
    Ok(spec)
}

/// Raw Dovetail MCP tool names in `suites`: new names first, then old aliases for one release
/// (ADR 0016).
pub(super) fn dovetail_tool_names(suites: &[Suite]) -> impl Iterator<Item = &'static str> {
    let specs = || tools_in(suites);
    specs()
        .map(|t| t.name)
        .chain(specs().filter_map(|t| t.old_name))
}

/// Dovetail's tools in `suites` as Claude Code names them, plus native messaging. Old names are
/// allowed too for one release, so a resumed conversation that calls one still works (ADR 0016).
pub fn allowed_tools(suites: &[Suite]) -> Vec<String> {
    let mut tools: Vec<String> = dovetail_tool_names(suites)
        .map(|name| format!("mcp__dovetail__{name}"))
        .collect();
    tools.push("SendMessage".into());
    tools.push("ListAgents".into());
    tools
}

/// Environment pairs passed to the `dovetail mcp` bridge process under any driver.
pub(super) fn bridge_env(paths: &Paths, launch: &Launch) -> Vec<(String, String)> {
    let mut pairs = vec![
        (env::AGENT_ID.into(), launch.agent_id.clone()),
        (
            env::SOCKET.into(),
            launch.socket.to_string_lossy().into_owned(),
        ),
        (env::ROLE.into(), launch.role.as_str().into()),
        (env::SUITES.into(), Suite::join(&launch.suites)),
    ];
    for (k, v) in paths.pinned_env() {
        pairs.push((k.into(), v.to_string_lossy().into_owned()));
    }
    pairs
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

/// Where agy keeps a conversation's store: `~/.gemini/antigravity-cli/conversations/<id>.db`.
/// Dovetail never reads it, but records it so `:fresh` and switch notices can name it.
pub fn agy_transcript_path(session_id: &str) -> PathBuf {
    home_dir()
        .join(".gemini/antigravity-cli/conversations")
        .join(format!("{session_id}.db"))
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
    fn agy_transcripts_are_under_gemini_conversations() {
        let path = agy_transcript_path("conv-123");
        assert!(path.ends_with(".gemini/antigravity-cli/conversations/conv-123.db"));
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
            &[
                "coordination_report",
                "coordination_claim_criterion",
                "maps_draw",
                "design_declare_variants",
            ],
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
                "coordination_report",
                "coordination_claim_criterion",
                "maps_draw",
                "canvas_add_block",
                "canvas_add_card",
                "canvas_link_cards",
                "design_declare_variants",
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

    #[test]
    fn agy_spec_writes_settings_folder_and_inline_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            data: dir.path().join("data"),
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            runtime: dir.path().join("run"),
        };
        std::fs::create_dir_all(paths.config.join("roles")).unwrap();
        let prompt = paths.role_prompt("coordinator");
        std::fs::write(&prompt, "lead").unwrap();

        let launch = Launch {
            project_key: "proj".into(),
            agent_id: "coordinator".into(),
            name: "proj.coordinator".into(),
            role: Role::Coordinator,
            prompt: None,
            suites: Role::Coordinator.default_suites().to_vec(),
            session_id: "s1".into(),
            resume: false,
            cwd: dir.path().join("home"),
            worktree: Some("ignored".into()),
            add_dirs: vec![dir.path().join("repo")],
            mode: CliMode::Auto,
            task_list_id: "tl".into(),
            socket: paths.socket("proj"),
            exe: PathBuf::from("/usr/bin/dovetail"),
            model: Some("gemini-3.8-flash".into()),
            effort: Some(dovetail_api::agents::model::ReasoningEffort::Low),
            env: vec![("CUSTOM".into(), "1".into())],
            claude_config_dir: Some(dir.path().join("claude-profile")),
        };

        let s = spec(&paths, &launch, DriverKind::Agy).unwrap();
        let settings_dir = s.agy.settings_dir.as_ref().expect("settings_dir");
        assert_eq!(
            settings_dir,
            &paths.run_dir("proj", "coordinator").join("agy")
        );
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(settings_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings,
            serde_json::json!({"toolPermission": "request-review"})
        );
        assert_eq!(s.agy.mcp_servers.len(), 1);
        assert_eq!(s.agy.mcp_servers[0].name, "dovetail");
        assert_eq!(
            s.agy.mcp_servers[0].command,
            PathBuf::from("/usr/bin/dovetail")
        );
        assert_eq!(s.agy.mcp_servers[0].args, vec!["mcp"]);
        assert!(
            s.agy.mcp_servers[0]
                .env
                .contains(&(env::AGENT_ID.into(), "coordinator".into()))
        );
        assert!(
            s.agy
                .allowed_tools
                .contains(&"mcp_dovetail_core_report_status".into())
        );
        assert!(!s.agy.allowed_tools.iter().any(|t| t.contains("__")));
        assert_eq!(s.append_system_prompt_file, Some(prompt));
        assert_eq!(s.claude.worktree, None);
        assert!(!s.env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR"));
        assert!(s.env.contains(&("CUSTOM".into(), "1".into())));

        // Side chats get DontAsk.
        let mut side = launch;
        side.role = Role::SideChat;
        let side_spec = spec(&paths, &side, DriverKind::Agy).unwrap();
        assert_eq!(side_spec.permission_mode, CliMode::DontAsk);
    }
}

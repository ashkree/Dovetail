//! Launch options and argv construction.

use std::path::{Path, PathBuf};

/// Permission mode of a session. Names follow the CLI; `Manual` is the CLI's `default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionMode {
    /// Classifier-approved tool use (`auto`).
    Auto,
    /// Every non-trivial tool use asks (`default`).
    Manual,
    /// File edits are allowed, the rest asks (`acceptEdits`).
    AcceptEdits,
    /// Plan mode (`plan`).
    Plan,
}

impl PermissionMode {
    /// The string the CLI uses, both for `--permission-mode` and `set_permission_mode`.
    pub fn as_cli(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
        }
    }

    /// Parse the CLI's name (as seen in `system/init` and `system/status`).
    pub fn from_cli(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => Self::Auto,
            "default" => Self::Manual,
            "acceptEdits" => Self::AcceptEdits,
            "plan" => Self::Plan,
            _ => return None,
        })
    }
}

/// Run the session inside `systemd-run --user --scope` (memory caps, M13).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemdScope {
    /// `-p` properties, e.g. `MemoryMax=4G`, `MemoryHigh=3G`.
    pub properties: Vec<String>,
}

/// Everything needed to launch one session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSpec {
    /// Session id Dovetail chose; passed as `--session-id` (or `--resume` when resuming).
    pub session_id: String,
    /// Resume an existing session instead of creating it.
    pub resume: bool,
    /// Display name (`--name`).
    pub name: Option<String>,
    /// Working directory of the process.
    pub cwd: PathBuf,
    /// Model alias or id (`--model`).
    pub model: Option<String>,
    /// Starting permission mode.
    pub permission_mode: PermissionMode,
    /// `--settings` file (hooks live here).
    pub settings_path: Option<PathBuf>,
    /// `--mcp-config` file.
    pub mcp_config_path: Option<PathBuf>,
    /// `--append-system-prompt-file` (role prompt).
    pub append_system_prompt_file: Option<PathBuf>,
    /// `--allowedTools` entries.
    pub allowed_tools: Vec<String>,
    /// `--worktree <name>` (slice sessions; the WorktreeCreate hook creates it).
    pub worktree: Option<String>,
    /// Pass `--replay-user-messages` (default on).
    pub replay_user_messages: bool,
    /// Extra environment variables (`CLAUDE_CODE_ENABLE_TODO_TOOLS`, `CLAUDE_CODE_TASK_LIST_ID`, ...).
    pub env: Vec<(String, String)>,
    /// Extra raw arguments appended last.
    pub extra_args: Vec<String>,
    /// Wrap in `systemd-run --user --scope`.
    pub systemd_scope: Option<SystemdScope>,
    /// If set, every line in both directions is appended here in the fixture format
    /// (`{"t","dir","msg"}`), so a session can be turned into a test fixture.
    pub log_path: Option<PathBuf>,
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl SessionSpec {
    /// A minimal spec: new session, auto mode, replay of user messages on.
    pub fn new(session_id: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            resume: false,
            name: None,
            cwd: cwd.into(),
            model: None,
            permission_mode: PermissionMode::Auto,
            settings_path: None,
            mcp_config_path: None,
            append_system_prompt_file: None,
            allowed_tools: Vec::new(),
            worktree: None,
            replay_user_messages: true,
            env: Vec::new(),
            extra_args: Vec::new(),
            systemd_scope: None,
            log_path: None,
        }
    }

    /// Arguments for `claude` itself (without the program name or any `systemd-run` prefix).
    pub fn claude_args(&self) -> Vec<String> {
        let mut a: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        a.push(self.permission_mode.as_cli().into());
        if self.replay_user_messages {
            a.push("--replay-user-messages".into());
        }
        // Guess: a resume names the session with --resume only (the spike resumed this way).
        a.push(
            if self.resume {
                "--resume"
            } else {
                "--session-id"
            }
            .into(),
        );
        a.push(self.session_id.clone());
        let optional = [
            ("--name", self.name.clone()),
            ("--model", self.model.clone()),
            ("--settings", self.settings_path.as_deref().map(lossy)),
            ("--mcp-config", self.mcp_config_path.as_deref().map(lossy)),
            (
                "--append-system-prompt-file",
                self.append_system_prompt_file.as_deref().map(lossy),
            ),
            ("--worktree", self.worktree.clone()),
        ];
        for (flag, value) in optional {
            if let Some(v) = value {
                a.push(flag.into());
                a.push(v);
            }
        }
        if !self.allowed_tools.is_empty() {
            a.push("--allowedTools".into());
            a.push(self.allowed_tools.join(","));
        }
        a.extend(self.extra_args.iter().cloned());
        a
    }
}

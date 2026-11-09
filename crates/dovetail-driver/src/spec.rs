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

/// Run the session inside `systemd-run --user --scope` (memory caps, M13; spike S4).
///
/// The scope is always started with `--collect` (an OOM-killed scope otherwise lingers) and a
/// fixed `--unit`, so the core knows its cgroup.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemdScope {
    /// The unit name without `.scope`, e.g. `dovetail-myproj-coordinator`.
    pub unit: String,
    /// `-p` properties, e.g. `MemoryMax=4294967296`.
    pub properties: Vec<String>,
    /// This `systemd-run` expands `$` in the command's arguments (S4: `$$` arrives as `$`):
    /// every `$` is written as `$$` so the session gets its arguments unchanged.
    pub escape_dollars: bool,
}

impl SystemdScope {
    /// A scope with a hard memory cap and no swap. `OOMPolicy=continue` keeps the session alive
    /// when the kernel kills only one of its tools (S4: the default policy stops the scope).
    pub fn capped(unit: impl Into<String>, max_bytes: u64, escape_dollars: bool) -> Self {
        Self {
            unit: unit.into(),
            properties: vec![
                format!("MemoryMax={max_bytes}"),
                "MemorySwapMax=0".into(),
                "OOMPolicy=continue".into(),
            ],
            escape_dollars,
        }
    }

    /// `systemd-run` arguments up to and including `--`.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = ["--user", "--scope", "--quiet", "--collect"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.push(format!("--unit={}", self.unit));
        for p in &self.properties {
            args.push("-p".into());
            args.push(p.clone());
        }
        args.push("--".into());
        args
    }

    /// One argument of the wrapped command as `systemd-run` must receive it.
    pub fn escape(&self, arg: &str) -> String {
        if self.escape_dollars {
            arg.replace('$', "$$")
        } else {
            arg.to_owned()
        }
    }
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

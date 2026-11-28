//! Launch options and argv construction.

use crate::agy::AgyOptions;
use crate::claude::ClaudeOptions;
use std::path::PathBuf;

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
    /// Only pre-approved tool use runs; everything else is denied without asking (`dontAsk`).
    /// Side chats run in it, so a command outside their allow-list is refused, not asked about.
    DontAsk,
}

impl PermissionMode {
    /// The string the CLI uses, both for `--permission-mode` and `set_permission_mode`.
    pub fn as_cli(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::DontAsk => "dontAsk",
        }
    }

    /// Parse the CLI's name (as seen in `system/init` and `system/status`).
    pub fn from_cli(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => Self::Auto,
            "default" => Self::Manual,
            "acceptEdits" => Self::AcceptEdits,
            "plan" => Self::Plan,
            "dontAsk" => Self::DontAsk,
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
    /// Reasoning effort (`--effort`); `None` leaves Claude Code's default.
    pub effort: Option<dovetail_api::agents::model::ReasoningEffort>,
    /// Starting permission mode.
    pub permission_mode: PermissionMode,
    /// `--append-system-prompt-file` (role prompt).
    pub append_system_prompt_file: Option<PathBuf>,
    /// One `--add-dir <path>` each: folders the session may read and edit besides its cwd (the
    /// coordinator's repos, ADR 0021).
    pub add_dirs: Vec<PathBuf>,
    /// Extra environment variables (`CLAUDE_CODE_ENABLE_TODO_TOOLS`, `CLAUDE_CODE_TASK_LIST_ID`, ...).
    pub env: Vec<(String, String)>,
    /// What only Claude Code takes.
    pub claude: ClaudeOptions,
    /// What only agy takes.
    pub agy: AgyOptions,
    /// Wrap in `systemd-run --user --scope`.
    pub systemd_scope: Option<SystemdScope>,
    /// If set, every line in both directions is appended here in the fixture format
    /// (`{"t","dir","msg"}`), so a session can be turned into a test fixture.
    pub log_path: Option<PathBuf>,
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
            effort: None,
            permission_mode: PermissionMode::Auto,
            append_system_prompt_file: None,
            add_dirs: Vec::new(),
            env: Vec::new(),
            claude: ClaudeOptions::default(),
            agy: AgyOptions::default(),
            systemd_scope: None,
            log_path: None,
        }
    }

    /// `program` with `claude_args` as one command line: wrapped in `systemd-run` when the spec
    /// has a scope, else run directly.
    pub fn wrap(
        &self,
        program: &std::ffi::OsStr,
        claude_args: Vec<String>,
    ) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
        use std::ffi::OsString;
        match &self.systemd_scope {
            None => (
                program.to_os_string(),
                claude_args.into_iter().map(OsString::from).collect(),
            ),
            Some(scope) => {
                let mut args: Vec<OsString> =
                    scope.args().into_iter().map(OsString::from).collect();
                args.push(scope.escape(&program.to_string_lossy()).into());
                args.extend(claude_args.iter().map(|a| OsString::from(scope.escape(a))));
                ("systemd-run".into(), args)
            }
        }
    }
}

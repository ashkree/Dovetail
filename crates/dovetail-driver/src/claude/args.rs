//! `claude`'s argv: the headless stream flags, and the flags a native take-over run shares.

use crate::SessionSpec;
use std::path::{Path, PathBuf};

/// What only Claude Code takes: its files and flags.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeOptions {
    /// `--settings` file (hooks live here).
    pub settings_path: Option<PathBuf>,
    /// `--mcp-config` file.
    pub mcp_config_path: Option<PathBuf>,
    /// `--allowedTools` entries.
    pub allowed_tools: Vec<String>,
    /// `--worktree <name>` (slice sessions; the WorktreeCreate hook creates it).
    pub worktree: Option<String>,
    /// Pass `--replay-user-messages` (default on).
    pub replay_user_messages: bool,
    /// Extra raw arguments appended last.
    pub extra_args: Vec<String>,
}

impl Default for ClaudeOptions {
    fn default() -> Self {
        Self {
            settings_path: None,
            mcp_config_path: None,
            allowed_tools: Vec::new(),
            worktree: None,
            replay_user_messages: true,
            extra_args: Vec::new(),
        }
    }
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl SessionSpec {
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
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if self.claude.replay_user_messages {
            a.push("--replay-user-messages".into());
        }
        a.extend(self.session_args());
        a
    }

    /// Arguments for native, interactive `claude` on the same session (take-over, M14): the
    /// same permission mode, settings, MCP config, role prompt, tools, name and model as
    /// [`SessionSpec::claude_args`], with none of the stream flags. It always resumes, and
    /// never passes `--worktree`: the session already runs in its worktree.
    pub fn native_args(&self) -> Vec<String> {
        let mut resuming = self.clone();
        resuming.resume = true;
        resuming.claude.worktree = None;
        resuming.session_args()
    }

    /// The arguments headless and native runs share.
    fn session_args(&self) -> Vec<String> {
        let claude = &self.claude;
        let mut a = vec![
            "--permission-mode".to_string(),
            self.permission_mode.as_cli().into(),
        ];
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
            ("--effort", self.effort.map(|e| e.as_str().to_owned())),
            ("--settings", claude.settings_path.as_deref().map(lossy)),
            ("--mcp-config", claude.mcp_config_path.as_deref().map(lossy)),
            (
                "--append-system-prompt-file",
                self.append_system_prompt_file.as_deref().map(lossy),
            ),
            ("--worktree", claude.worktree.clone()),
        ];
        for (flag, value) in optional {
            if let Some(v) = value {
                a.push(flag.into());
                a.push(v);
            }
        }
        for dir in &self.add_dirs {
            a.push("--add-dir".into());
            a.push(lossy(dir));
        }
        if !claude.allowed_tools.is_empty() {
            a.push("--allowedTools".into());
            a.push(claude.allowed_tools.join(","));
        }
        a.extend(claude.extra_args.iter().cloned());
        a
    }
}

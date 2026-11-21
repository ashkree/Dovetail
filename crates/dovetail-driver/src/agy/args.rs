//! What an agy session is launched with: the `agy acp` argv, and the parameters of the
//! `session/new` (or `session/resume`) request that opens the conversation.

use crate::{PermissionMode, SessionSpec};
use serde_json::{Value, json};
use std::path::PathBuf;

/// One MCP server for agy, passed inline in `session/new` (agy takes no config file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerSpec {
    /// The server's name; agy shows its tools as `mcp_<name>_<tool>`.
    pub name: String,
    /// The executable.
    pub command: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// Its environment.
    pub env: Vec<(String, String)>,
}

/// What only agy takes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgyOptions {
    /// `--cli_dir_name`: the folder holding the session's own `settings.json`, so its tool
    /// permissions are Dovetail's and not the user's (spike: absolute paths work).
    pub settings_dir: Option<PathBuf>,
    /// MCP servers for the session (Dovetail's bridge).
    pub mcp_servers: Vec<McpServerSpec>,
    /// Tool names (as agy names them, `mcp_dovetail_core_ask_user`) the driver allows without
    /// asking: agy's own allow-rules did not cover MCP tools in the spike.
    pub allowed_tools: Vec<String>,
}

impl SessionSpec {
    /// Arguments for `agy` itself (without the program name or any `systemd-run` prefix).
    pub fn agy_args(&self) -> Vec<String> {
        let mut a = vec!["acp".to_string()];
        if let Some(dir) = &self.agy.settings_dir {
            a.push("--cli_dir_name".into());
            a.push(dir.to_string_lossy().into_owned());
        }
        a
    }

    /// The params of `session/new`, or of `session/resume` when resuming (`sessionId` is then
    /// the agy conversation id the core recorded). `role_prompt` is the role prompt's text.
    pub(crate) fn agy_session_params(&self, role_prompt: Option<&str>) -> Value {
        let servers: Vec<Value> = self
            .agy
            .mcp_servers
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "command": s.command.to_string_lossy(),
                    "args": s.args,
                    "env": s.env.iter().map(|(k, v)| json!({"name": k, "value": v})).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mut params = json!({
            "cwd": self.cwd.to_string_lossy(),
            "mcpServers": servers,
        });
        if self.resume {
            params["sessionId"] = json!(self.session_id);
        }
        if !self.add_dirs.is_empty() {
            let dirs: Vec<_> = self.add_dirs.iter().map(|d| d.to_string_lossy()).collect();
            params["additionalDirectories"] = json!(dirs);
        }
        if let Some(prompt) = role_prompt {
            params["_meta"] = json!({"antigravity": {"systemPrompt": {"append": prompt}}});
        }
        params
    }
}

/// agy's mode for a permission mode. `DontAsk` runs in `default`: the driver itself denies
/// whatever the session's allow list does not name.
pub(crate) fn mode_id(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Auto => "auto",
        PermissionMode::Manual | PermissionMode::DontAsk => "default",
        PermissionMode::AcceptEdits => "accept-edits",
        PermissionMode::Plan => "plan",
    }
}

/// The permission mode agy reports, named as Claude Code names it (`DriverEvent::Status`
/// carries the CLI's word, which the core parses).
pub(crate) fn mode_as_cli(agy: &str) -> Option<&'static str> {
    Some(match agy {
        "auto" => PermissionMode::Auto.as_cli(),
        "default" => PermissionMode::Manual.as_cli(),
        "accept-edits" => PermissionMode::AcceptEdits.as_cli(),
        "plan" => PermissionMode::Plan.as_cli(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_carries_servers_folders_and_the_role_prompt() {
        let mut spec = SessionSpec::new("dovetail-id", "/work/repo");
        spec.add_dirs = vec!["/work/other".into()];
        spec.agy.settings_dir = Some("/run/dovetail/p/a/agy".into());
        spec.agy.mcp_servers = vec![McpServerSpec {
            name: "dovetail".into(),
            command: "/bin/dovetail".into(),
            args: vec!["mcp".into()],
            env: vec![("DOVETAIL_ROLE".into(), "slice".into())],
        }];
        assert_eq!(
            spec.agy_args(),
            ["acp", "--cli_dir_name", "/run/dovetail/p/a/agy"]
        );
        let params = spec.agy_session_params(Some("be brief"));
        assert_eq!(
            params,
            json!({
                "cwd": "/work/repo",
                "mcpServers": [{
                    "name": "dovetail",
                    "command": "/bin/dovetail",
                    "args": ["mcp"],
                    "env": [{"name": "DOVETAIL_ROLE", "value": "slice"}],
                }],
                "additionalDirectories": ["/work/other"],
                "_meta": {"antigravity": {"systemPrompt": {"append": "be brief"}}},
            })
        );
    }

    #[test]
    fn a_resume_names_the_agy_conversation() {
        let mut spec = SessionSpec::new("agy-conversation", "/w");
        spec.resume = true;
        let params = spec.agy_session_params(None);
        assert_eq!(params["sessionId"], "agy-conversation");
        assert!(params.get("_meta").is_none());
    }

    #[test]
    fn modes_map_both_ways() {
        for mode in [
            PermissionMode::Auto,
            PermissionMode::Manual,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
        ] {
            assert_eq!(mode_as_cli(mode_id(mode)), Some(mode.as_cli()));
        }
        assert_eq!(mode_id(PermissionMode::DontAsk), "default");
        assert_eq!(mode_as_cli("bogus"), None);
    }
}

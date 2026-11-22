//! Claude Code launch files (`settings.json`, `mcp.json`) and `ClaudeOptions`.

use std::path::Path;

use dovetail_driver::SessionSpec;
use dovetail_tools::{Role, env};
use serde_json::{Map, Value, json};

use crate::paths::Paths;

use super::{
    Launch, allowed_tools, bridge_env, shell_quote, side_chat_allowed, side_chat_disallowed,
    write_json,
};

pub(super) fn configure(
    paths: &Paths,
    launch: &Launch,
    dir: &Path,
    spec: &mut SessionSpec,
) -> std::io::Result<()> {
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
    let bridge_env: Map<String, Value> = bridge_env(paths, launch)
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    let mcp = json!({ "mcpServers": { "dovetail": {
        "command": launch.exe,
        "args": ["mcp"],
        "env": Value::Object(bridge_env),
    }}});
    write_json(&mcp_path, &mcp)?;

    spec.claude.settings_path = Some(settings_path);
    spec.claude.mcp_config_path = Some(mcp_path);
    spec.claude.allowed_tools = allowed_tools(&launch.suites);
    if matches!(launch.role, Role::SideChat | Role::Ask) {
        spec.claude.allowed_tools.extend(side_chat_allowed());
        spec.claude.extra_args.extend([
            "--disallowedTools".to_string(),
            side_chat_disallowed().join(","),
        ]);
    }
    spec.claude.worktree = launch.worktree.clone();
    spec.env = vec![
        ("CLAUDE_CODE_ENABLE_TODO_TOOLS".into(), "1".into()),
        (
            "CLAUDE_CODE_TASK_LIST_ID".into(),
            launch.task_list_id.clone(),
        ),
        (env::AGENT_ID.into(), launch.agent_id.clone()),
        // The worktree hook names worktrees and finds the recipe by it (`new-<pid>` before
        // registration: no recipe then).
        (env::PROJECT.into(), launch.project_key.clone()),
    ];
    spec.env.extend(launch.env.iter().cloned());
    // After the recipe's `[env]`, so the profile always wins.
    if let Some(dir) = &launch.claude_config_dir {
        spec.env.retain(|(k, _)| k != "CLAUDE_CONFIG_DIR");
        spec.env.push((
            "CLAUDE_CONFIG_DIR".into(),
            dir.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

//! agy launch files (`agy/settings.json`) and `AgyOptions`.

use std::path::Path;

use dovetail_api::tools::env;
use dovetail_driver::{McpServerSpec, SessionSpec};
use serde_json::json;

use crate::paths::Paths;

use super::{Launch, bridge_env, dovetail_tool_names, write_json};

pub(super) fn configure(
    paths: &Paths,
    launch: &Launch,
    dir: &Path,
    spec: &mut SessionSpec,
) -> std::io::Result<()> {
    let agy_dir = dir.join("agy");
    std::fs::create_dir_all(&agy_dir)?;
    write_json(
        &agy_dir.join("settings.json"),
        &json!({ "toolPermission": "request-review" }),
    )?;
    spec.agy.settings_dir = Some(agy_dir);
    spec.agy.mcp_servers = vec![McpServerSpec {
        name: "dovetail".into(),
        command: launch.exe.clone(),
        args: vec!["mcp".into()],
        env: bridge_env(paths, launch),
    }];
    spec.agy.allowed_tools = dovetail_tool_names(&launch.suites)
        .map(|name| format!("mcp_dovetail_{name}"))
        .collect();
    spec.env = vec![
        (env::AGENT_ID.into(), launch.agent_id.clone()),
        (env::PROJECT.into(), launch.project_key.clone()),
    ];
    spec.env.extend(launch.env.iter().cloned());
    Ok(())
}

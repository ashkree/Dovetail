//! `dovetail hook worktree-create` and `dovetail hook worktree-remove`: Claude Code hooks.
//!
//! # `worktree-create`
//!
//! Claude Code runs it on every `--worktree <name>` start (and for subagent worktrees), with the
//! hook JSON on stdin: `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `name`
//! (subagents: `agent-<id>`, plus `prompt_id`; `cwd` is then the parent's worktree).
//! Only `name` and `cwd` are used. On success it prints the worktree path on stdout (one line,
//! nothing else) and exits 0. It is idempotent: it creates the worktree on branch
//! `dovetail/<name>` from `cwd`'s `HEAD`, symlinks the recipe's paths, runs the recipe's setup
//! (slices only, once; interrupted or failed setup runs again) and returns. Either way it records
//! a `worktree::HookOutcome` in the setup state directory, which the core waits for before it
//! sends a slice its first prompt. On failure it prints a message on stderr and exits non-zero;
//! the core raises an inbox error and sends no first prompt.
//!
//! Environment, set by the core at launch:
//! - `DOVETAIL_PROJECT` (required): the project id (recipe: `~/.config/dovetail/projects/<id>.toml`).
//! - `DOVETAIL_REPO`: the slice's repo name, for the recipe's `[repos.<name>]` overrides.
//! - `DOVETAIL_SLICE`: the slice name (informational; the worktree name comes from the hook's `name`).
//! - `DOVETAIL_AGENT_ID`: the Dovetail agent id of the session (informational, used in messages).
//!
//! # `worktree-remove`
//!
//! A no-op that exits 0 (A9): Dovetail removes worktrees itself, and a native exit must never do it.
//!
//! # Launch settings the core must pass (`--settings`)
//!
//! The command is the `dovetail` binary's absolute path plus the subcommand. The timeout is in
//! seconds (15 minutes, B5), so a long setup finishes inside the hook:
//!
//! ```json
//! {
//!   "hooks": {
//!     "WorktreeCreate": [
//!       { "hooks": [ { "type": "command", "command": "/abs/path/dovetail hook worktree-create", "timeout": 900 } ] }
//!     ],
//!     "WorktreeRemove": [
//!       { "hooks": [ { "type": "command", "command": "/abs/path/dovetail hook worktree-remove", "timeout": 30 } ] }
//!     ]
//!   }
//! }
//! ```

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use dovetail_core::code::worktree::{self, CreateRequest, HookOutcome};
use dovetail_core::settings::paths::Paths;
use dovetail_core::settings::recipe::Recipe;
use dovetail_tools::env;
use serde::Deserialize;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub event: HookEvent,
}

#[derive(Debug, clap::Subcommand)]
pub enum HookEvent {
    /// Create (or reuse) a slice or subagent worktree from hook JSON on stdin.
    WorktreeCreate,
    /// No-op: Dovetail removes worktrees itself.
    WorktreeRemove,
}

/// The fields of the hook input that we use.
#[derive(Debug, Deserialize)]
struct HookInput {
    name: String,
    cwd: PathBuf,
    #[serde(default)]
    hook_event_name: Option<String>,
}

/// Run the subcommand with the parsed arguments.
pub fn run(args: Args) -> Result<()> {
    match args.event {
        HookEvent::WorktreeCreate => worktree_create(),
        HookEvent::WorktreeRemove => Ok(()),
    }
}

fn worktree_create() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("reading hook input from stdin")?;
    let input: HookInput = serde_json::from_str(&raw).context("parsing hook input JSON")?;
    if let Some(ev) = &input.hook_event_name
        && ev != "WorktreeCreate"
    {
        bail!("unexpected hook event {ev:?}");
    }
    let project = std::env::var(env::PROJECT)
        .ok()
        .filter(|p| !p.is_empty())
        .with_context(|| format!("{} is not set", env::PROJECT))?;
    let paths = Paths::from_env();
    let recipe = Recipe::load(&paths.recipe(&project)).context("loading the project recipe")?;
    // A slice of one of several repos gets that repo's `[repos.<name>]` overrides (ADR 0021).
    let recipe = match std::env::var(env::REPO).ok().filter(|r| !r.is_empty()) {
        Some(repo) => recipe.for_repo(&repo),
        None => recipe,
    };
    let req = CreateRequest {
        project,
        name: input.name,
        cwd: input.cwd,
        recipe,
    };
    let created = worktree::create(&paths, &req);
    // The core waits for this outcome before it sends a slice its first prompt.
    let outcome = match &created {
        Ok(c) => HookOutcome {
            path: Some(c.path.clone()),
            error: None,
        },
        Err(e) => HookOutcome {
            path: None,
            error: Some(e.to_string()),
        },
    };
    if let Err(e) = worktree::write_hook_outcome(&paths, &req.project, &req.name, &outcome) {
        eprintln!("recording the hook outcome: {e}");
    }
    let created = created.with_context(|| {
        let agent = std::env::var(env::AGENT_ID).unwrap_or_default();
        format!("creating worktree `{}` (agent {agent})", req.name)
    })?;
    println!("{}", created.path.display());
    Ok(())
}

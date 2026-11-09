# Profiles: a Claude account per project

New beyond the v1 design (task #27). The user builds Dovetail on their own Claude account and does work projects on another, so a project picks the Claude Code account its sessions run under. No v1 decision is superseded. The hard constraint "nothing in user repos, everything passed to Claude Code as launch flags" still holds: a profile is one more environment variable at launch.

## Decision

- A **profile** is a name for a Claude Code config directory, listed in `~/.config/dovetail/config.toml`:

  ```toml
  [profiles]
  walid = "~/.claude"
  maveron = "~/.local/share/dovetail/profiles/maveron"
  ```

  Paths must be absolute or start with `~/`. New profiles go under `~/.local/share/dovetail/profiles/<name>` (`Paths::profiles`).
- A project picks one with `profile = "<name>"` in its recipe (`~/.config/dovetail/projects/<id>.toml`). No profile means the default home (`$CLAUDE_CONFIG_DIR` as Dovetail was started with, else `~/.claude`) and no change to the launch: today's behaviour.
- Every session of the project (coordinator, slices, and their subagents, which run inside them) starts with `CLAUDE_CONFIG_DIR=<profile dir>`. Hooks, MCP config and settings are still passed as launch flags.
- Each agent row records the config directory its session ran under (`agents.claude_home`, state migration 0010). Everything that reads Claude Code's files uses that directory: transcript paths and the native to-do lists (`<dir>/tasks`).
- **A session is never resumed under a different profile** than the one it was recorded with. It starts fresh instead, with a notice in its conversation. When a project's profile changes, one Needs-you item says how many sessions will start fresh. Tasks, decisions and worktrees are kept.
- An unknown profile name (or an unreadable recipe) is an error item in Needs you, and no session starts. Dovetail never falls back to another account.
- Usage limits are per account: the core keeps the latest `rate_limit_event` figures per config directory and shows the project's. The header leads with the profile name: `maveron · opus-5-5 · 5h 34% · week 18%`. The name drops with the model as the row narrows. With no profile set, the name shown is the profile that lists the default home, if any.
- `dovetail login <profile>` (outside the TUI) creates the profile's folder (mode 0700), adds it to `config.toml` if missing (an inserted line, the user's lines untouched), and runs `claude` with that `CLAUDE_CONFIG_DIR` so the user can `/login`. `dovetail login --list` shows each profile, its folder and whether it holds a login. It only checks that `.credentials.json` exists and never reads it.

## Why

The spike (`docs/spikes/profiles.md`) showed that `CLAUDE_CONFIG_DIR` moves Claude Code 2.1.319's credentials, global config (`.claude.json`), transcripts and to-do lists into the given folder. With an empty folder, Claude Code reports "not logged in" even while `~/.claude` holds a login. So one environment variable per launch gives full isolation, with no wrapper scripts and nothing in the repo.

Resuming across profiles is refused because the transcript lives in the old profile's folder, and a session must not move to another account behind the user's back.

## Alternatives considered

- **A global account switch** (one profile for all of Dovetail): the user works on two accounts in parallel windows, so it has to be per project.
- **Profile per session or per slice:** no use case, and it would mix accounts inside one feature.
- **Copying or symlinking transcripts on a profile change, to resume:** this moves conversations between accounts, so it was rejected.
- **`toml_edit` to rewrite `config.toml`:** a new dependency just to add one line. A text insert, checked by parsing the result, is enough. When the table is written some other way, Dovetail asks the user to add the line by hand.

## Consequences

- A fresh profile has no user `settings.json`, skills or plugins of its own. Dovetail does not depend on them.
- `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_SECURESTORAGE_CONFIG_DIR` in Dovetail's environment override every profile's login; leave them unset.
- Profiles are read when the app opens a project; changing one takes a restart.

Task #27, approved by the user 2026-11-09.

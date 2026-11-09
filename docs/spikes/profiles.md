# Profiles: does `CLAUDE_CONFIG_DIR` isolate a Claude Code account?

**Status:** run 2026-11-09 against Claude Code 2.1.319 on Linux. **Isolation holds** — profiles can be built on it (task #27). No login, no tokens spent.

## Question

Profiles (task #27) give each project its own Claude Code account by launching every session with `CLAUDE_CONFIG_DIR=<profile dir>`. That only works if Claude Code then reads credentials and keeps all account state (global config, transcripts, to-do lists) under that dir, and never falls back to `~/.claude` or `~/.claude.json`.

## Method

Two parts, neither of them logs in or calls the API:

1. **Empirical.** Point `CLAUDE_CONFIG_DIR` at a fresh empty dir, while `~/.claude/.credentials.json` holds a valid login:

   ```
   mkdir -p /tmp/dt-profile-spike
   env CLAUDE_CONFIG_DIR=/tmp/dt-profile-spike claude --version
   env CLAUDE_CONFIG_DIR=/tmp/dt-profile-spike claude auth status
   find /tmp/dt-profile-spike
   ```

2. **Static.** Read the path code in the 2.1.319 binary (`strings` on `~/.local/share/claude/versions/2.1.319`, grep for `CLAUDE_CONFIG_DIR`, `.credentials.json`, `.claude.json`, `"tasks"`).

## Result

`claude --version` writes nothing. `claude auth status` reports, exit 1:

```json
{ "loggedIn": false, "authMethod": "none",
  "projectsDirectory": "/tmp/dt-profile-spike/projects",
  "configDirectory": "/tmp/dt-profile-spike", ... }
```

and creates only `/tmp/dt-profile-spike/.claude.json` (+ its lock and a backup). The real login in `~/.claude` is not seen.

What the binary resolves from the config home (`$CLAUDE_CONFIG_DIR`, else `~/.claude`):

| State | Path with `CLAUDE_CONFIG_DIR` set |
| --- | --- |
| Credentials (Linux, plain file) | `$CLAUDE_CONFIG_DIR/.credentials.json` — unless `CLAUDE_SECURESTORAGE_CONFIG_DIR` is set, which then wins |
| Global config (`~/.claude.json` normally) | `$CLAUDE_CONFIG_DIR/.claude.json` |
| Transcripts | `$CLAUDE_CONFIG_DIR/projects/<slug>/<session>.jsonl` |
| Native to-do lists | `$CLAUDE_CONFIG_DIR/tasks/<list>/<id>.json` |
| User settings, skills, plugins, history, file-history | under `$CLAUDE_CONFIG_DIR` |

Caveats found:

- **The path must be absolute.** An empty or relative value makes Claude Code resolve it against several folders. Dovetail expands `~` and rejects relative profile paths.
- **`CLAUDE_SECURESTORAGE_CONFIG_DIR`** overrides where credentials live (an empty value means `~/.claude`). Sessions inherit Dovetail's environment, so if it is set there it wins for every profile: leave it unset. Noted in `docs/self-hosting.md`.
- **`ANTHROPIC_API_KEY` / `CLAUDE_CODE_OAUTH_TOKEN`** in the environment win over any stored login, for every profile. Not Dovetail's to change; noted in `docs/self-hosting.md`.
- `~/.claude/.device-keys.json` stays under the home dir for all profiles. It is a device identity, not account credentials; harmless here.
- A fresh profile dir has no user `settings.json`, skills or plugins: those are per profile too. Dovetail passes its own hooks, MCP config and settings as launch flags, so Dovetail itself does not depend on them.

## Decision

Build profiles on `CLAUDE_CONFIG_DIR` (ADR 0009). Everything in Dovetail that reads Claude Code's files must use the agent's profile dir.

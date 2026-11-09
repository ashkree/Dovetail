# Building Dovetail inside Dovetail

From M10 on, the rest of the build runs inside Dovetail: the installed release drives the coordinator and the slices, and each slice works in its own git worktree. This page covers the daily flow, upgrading, recovering when something breaks, and dev builds inside a slice.

## Once

1. `scripts/bootstrap.sh` creates Dovetail's directories, installs the role prompts and this repo's recipe (`~/.config/dovetail/projects/dovetail.toml`), and checks Claude Code, `git` and the memory caps. Run it again with `--force` after the role prompts or the recipe change in the repo.
2. `scripts/install.sh` installs the stable build to `~/.local/bin/dovetail`. Put `~/.local/bin` on your `PATH`.
3. In the repo, run `dovetail`, then `:project add`.

## Daily flow

- Open `dovetail` in the repo. The coordinator's session resumes at once; slices resume when you first open or message them.
- Plan with the coordinator. It files tasks (`create_task`) and starts slices (`start_slice`). Outside auto mode each start asks you first, in Needs you.
- Slices push a branch and open a PR. On your yes, the coordinator merges it and pulls `main`.
- Archive a slice with `dd` on its row, or archive its task. Dovetail asks before removing a worktree with uncommitted or unpushed work.
- Quit with `:q`, or `Ctrl-C` twice. Either asks first if a turn is running. Every session ends with the app, and nothing runs in the background.

## After a restart

Whatever was running when the app quit, was killed or crashed is reported on the next open:

- **A cut-off turn** raises "*agent*'s turn was cut off by a restart" in Needs you, with the last prompt. The agent is never continued for you. Message it to carry on.
- **A permission request** that was waiting is gone with its session. It shows in Activity as "*tool target*: request expired at restart". The agent asks again if it still needs to.
- **An `ask_user` question** stays in Needs you. Your answer reaches the agent as a message.
- **A slice whose worktree is gone** is archived, with a note in Activity.
- **A worktree no slice owns** raises one "Remove worktree" item. `y` removes it, and `d` keeps it for good. Nothing is deleted without your `y`.

## Upgrading

1. Quit Dovetail (`:q`). Running sessions use the installed binary for their hooks and MCP bridge, so `scripts/install.sh` refuses while an app runs ("close Dovetail first").
2. Pull and run `scripts/install.sh`.
3. If the role prompts or the recipe changed, run `scripts/bootstrap.sh --force`.
4. Open `dovetail` again. The conversations resume.

Hook and MCP paths always come from the running binary, so the installed app never points at `target/`, and rebuilding `target/` cannot break running sessions.

## Recovery

Everything Dovetail does is ordinary git and ordinary Claude Code, so if Dovetail itself breaks:

- `claude` in the repo still works. To pick up a slice's conversation, run `claude --resume <session id>` in its worktree, under `~/.local/share/dovetail/projects/<id>/worktrees/<name>`. The ids are in the project's database: `sqlite3 ~/.local/share/dovetail/projects/<id>/state.db "select name, session_id from agents"`.
- Slice branches are plain `dovetail/<name>` branches, and their worktrees show in `git worktree list`.
- Dovetail's own state is in `~/.local/share/dovetail/` (databases), `~/.config/dovetail/` (config, roles, recipes) and `~/.local/state/dovetail/` (logs, launch files). Nothing is written into the repo except branches.
- Still broken after a restart? Install the last good commit with `scripts/install.sh`.

## Dev builds inside a slice

A slice working on Dovetail builds and runs its own `dovetail` from its worktree. That dev build must never touch your live state. The recipe's `[env]` gives every slice its own directories:

```toml
DOVETAIL_DATA_DIR = "{worktree}/target/dovetail-dev/data"
DOVETAIL_CONFIG_DIR = "{worktree}/target/dovetail-dev/config"
DOVETAIL_STATE_DIR = "{worktree}/target/dovetail-dev/state"
DOVETAIL_RUNTIME_DIR = "/tmp/dvt-{worktree_hash}"
```

- `{worktree}` is the slice's worktree, and `{worktree_hash}` is eight hex digits from its path.
- The runtime directory holds unix sockets, whose paths are limited to about 108 bytes, so it stays short under `/tmp`.
- These are Dovetail's own variables, not `XDG_*`. The slice's Claude Code session inherits its environment, and moving its `XDG_*` would also move Claude Code's files and the messaging sockets that link it to the coordinator.
- The installed hook and MCP bridge are pinned to your live directories in their launch files, so the slice's dev variables never redirect them.

A dev build's state lives in the worktree's `target/`, which git ignores, and goes away with the worktree. Its socket directory under `/tmp` is cleared at reboot.

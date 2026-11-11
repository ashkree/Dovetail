# Building Dovetail inside Dovetail

From M10 on, the rest of the build runs inside Dovetail: the installed release drives the coordinator and the slices, and each slice works in its own git worktree. This page covers the daily flow, upgrading, recovering when something breaks, and dev builds inside a slice.

## Once

1. `scripts/bootstrap.sh` creates Dovetail's directories, installs the role prompts and this repo's recipe (`~/.config/dovetail/projects/dovetail.toml`), and checks Claude Code, `git` and the memory caps. Run it again with `--force` after the role prompts or the recipe change in the repo.
2. `scripts/install.sh` installs the stable build to `~/.local/bin/dovetail`. Put `~/.local/bin` on your `PATH`.
3. In the repo, run `dovetail`, then `:project add`.
4. Optional: pick the Claude account this project runs on (see "Claude profiles" below).

## Daily flow

- Open `dovetail` in the repo. The coordinator's session resumes at once; slices resume when you first open or message them.
- Plan with the coordinator. It files tasks (`tasks_create`) and starts slices (`project_start_slice`). Outside auto mode each start asks you first, in Needs you.
- Slices push a branch and open a PR. On your yes, the coordinator merges it and pulls `main`.
- Archive a slice with `dd` on its row, or archive its task. Dovetail asks before removing a worktree with uncommitted or unpushed work.
- Quit with `:q`, or `Ctrl-C` twice. Either asks first if a turn is running. Every session ends with the app, and nothing runs in the background.
- **End of day (ADR 0012).** On the first quit of a day Dovetail asks "Close the day?". `y` asks the coordinator for its handoff and writes `~/.local/share/dovetail/projects/<id>/journal/YYYY-MM-DD.md` (its Handoff plus Dovetail's Facts), then quits; `n` quits now; `Esc` stays. `:eod` does the same without quitting. A handoff file the coordinator wrote there by hand works too.
- **Next morning** the coordinator starts as a new session, seeded with the newest journal file and your open tasks. Yesterday's session is kept: `y` on "Keep yesterday's coordinator" in Needs you goes back to it, `d` keeps the fresh one.

## After a restart

Whatever was running when the app quit, was killed or crashed is reported on the next open:

- **A cut-off turn** raises "*agent*'s turn was cut off by a restart" in Needs you, with the last prompt. The agent is never continued for you. Message it to carry on.
- **A permission request** that was waiting is gone with its session. It shows in Activity as "*tool target*: request expired at restart". The agent asks again if it still needs to.
- **A `core_ask_user` question** stays in Needs you. Your answer reaches the agent as a message.
- **A slice whose worktree is gone** is archived, with a note in Activity.
- **A worktree no slice owns** raises one "Remove worktree" item. `y` removes it, and `d` keeps it for good. Nothing is deleted without your `y`.

## Upgrading

From inside the app (ADR 0014):

1. Type `:upgrade`. Dovetail fetches `origin/main` into a throwaway worktree and builds it in the background into `~/.local/share/dovetail/upgrade/`. It never builds over `~/.local/bin/dovetail`, and never in your checkout. `:upgrade --local` builds the checkout instead.
2. When the build is done, **Upgrade ready** appears in Needs you. A failed build is an Error item with the end of cargo's output; nothing is replaced.
3. `y` restarts once every agent is idle (no turn is cut off; the item lists the agents it waits for). `d` leaves the build staged: it is installed when you next quit with `:q`.
4. The restart is the normal quit, then the staged binary is renamed over the installed one, then Dovetail starts again in the same terminal, folder and arguments. Sessions resume as after any restart.
5. If the item says role prompts or the recipe differ, they are not overwritten; run `scripts/bootstrap.sh --force` to take the new ones.

`dovetail upgrade [--local]` does the same from the shell: with no app running it builds and installs at once; with an app running in the project's folder it hands the build to that app, which offers **Upgrade ready**. If another project's app runs, it refuses ("close Dovetail first"). Only the installed `dovetail` upgrades; a dev build refuses. Needs `cargo` and `git`.

The installed binary is only replaced while no app and no helper of it runs. If one still does at the swap (another project's app), the build stays staged and Dovetail says so after it shuts down.

The old way still works: quit Dovetail (`:q`), pull, run `scripts/install.sh` (it refuses while an app runs, "close Dovetail first"), run `scripts/bootstrap.sh --force` if the role prompts or the recipe changed, and open `dovetail` again.

Hook and MCP paths always come from the running binary, so the installed app never points at `target/`, and rebuilding `target/` cannot break running sessions.

## Recovery

Everything Dovetail does is ordinary git and ordinary Claude Code, so if Dovetail itself breaks:

- `claude` in the repo still works. To pick up a slice's conversation, run `claude --resume <session id>` in its worktree, under `~/.local/share/dovetail/projects/<id>/worktrees/<name>`. The ids are in the project's database: `sqlite3 ~/.local/share/dovetail/projects/<id>/state.db "select name, session_id from agents"`.
- Slice branches are plain `dovetail/<name>` branches, and their worktrees show in `git worktree list`.
- Dovetail's own state is in `~/.local/share/dovetail/` (databases), `~/.config/dovetail/` (config, roles, recipes) and `~/.local/state/dovetail/` (logs, launch files). Nothing is written into the repo except branches.
- Still broken after a restart? Install the last good commit with `scripts/install.sh`.

## Claude profiles

A profile is a Claude Code account, kept in its own config folder (ADR 0009). Each project picks one, and every session of the project runs under it. A project without one uses your default `~/.claude`, as before.

1. `dovetail login maveron` creates the profile (under `~/.local/share/dovetail/profiles/maveron`), adds it to `~/.config/dovetail/config.toml`, and opens Claude Code in it. Type `/login`, sign in with that account, then `/exit`.
2. Add `profile = "maveron"` to the project's recipe, `~/.config/dovetail/projects/<id>.toml`.
3. Restart Dovetail in that project. The header now starts with `maveron`, and the usage figures are that account's.

To name your existing login too, add it by hand under `[profiles]`, for example `walid = "~/.claude"`. `dovetail login --list` shows every profile, its folder, and whether it is logged in.

- When a project's profile changes, sessions recorded under the old one start fresh: one Needs-you item says how many. Tasks, decisions and worktrees are kept. A session never resumes under another account.
- An unknown profile name shows as an error in Needs you, and no session starts until you fix it. So does a recipe `[env]` that sets `CLAUDE_CONFIG_DIR`: use `profile = …` instead.
- `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN` or `CLAUDE_SECURESTORAGE_CONFIG_DIR` in your environment override every profile's login, so leave them unset.
- A new profile starts without your `~/.claude` settings, skills and plugins. Copy over what you want; Dovetail itself needs none of them.

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

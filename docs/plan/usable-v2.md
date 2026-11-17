# Usable v2: close the gaps to daily use (2026-11-17)

Scope approved by the user through the coordinating session: agent conversations, interrupt /
archive / reorder, a launcher, a live pass, and a dry run on a copy of the user's real data.
Branch `claude/usable-v2-245d55`. Anything that needs a user decision is listed at the end, not
decided.

## Build

1. **Agent conversations.** A new main view `agent` (tab `Agent` while one is open, like `Inbox
   item`). It shows the agent's conversation from `Snapshot.conversations[agent]` (already in the
   core, streamed by `conversation_appended` / `assistant_delta`), in the chat's message style; tool
   summaries are one dim line; its open inbox items (permission requests, questions) sit above
   its own chat bar and answer with the inbox keys `y A d D r x`. `i` types to it (`prompt` with
   the agent and its task's effort). `gg` at the top pages in older entries (`load_conversation`).
   Opened from: the inbox item (`Enter`), the tree (`Enter` on a subagent; `c` on any agent row),
   `Space s s` / the picker, a chat report line (`Enter`), a canvas card (`ga`: the agent that
   added it). Files: `core/model.ts`, `core/view.ts`, `core/protocol.ts`, `core/translate.ts`,
   `state/ui.ts`, `keys/actions.ts`, `keys/keymap.ts`, `components/main/AgentView.tsx`,
   `components/chat/ChatBar.tsx`, `components/main/MainView.tsx`, `components/shell/TopBar.tsx`.
2. **Interrupt, archive, reorder.** `Space s x` interrupts the agent in focus (the agent view's,
   else the tree cursor's, else the coordinator; a subagent's slice, since it shares that
   session) with the core's `interrupt`. Tree `dd` archives with the keep/remove picker as the
   confirm: a task → `archive_task` (its slices end), an effort → `update_effort` status closed,
   a slice → `close_slice`. Tree `J`/`K` on a task → `reorder_task`, the delta counted so the task
   passes one task of its own effort (the core orders a whole bucket). Search stays a toast.
3. **Launcher.** `scripts/install.sh` also builds the window (`npx tauri build --no-bundle`) and
   installs it as `~/.local/bin/dovetail-gui`: one binary, assets embedded, no dev server, the
   core in-process as today. Its sessions' hooks and MCP bridge keep using `~/.local/bin/dovetail`,
   installed by the same run.

## Verify

4. Live pass (`scripts/dev-gui.sh --reset`, scratch repo under `/tmp/dvt-usable`, tiny prompts):
   PR #86's changes, then the agent view, typing to a slice, interrupt, archive.
5. Dry run on a copy of real data: `~/.local/share/dovetail` and `~/.config/dovetail` copied (read
   only) into `target/dovetail-dev/`, `scripts/dev-gui.sh` without `--reset`; migrations run,
   Home lists the projects, a project opens with its history. No session against a real repo.

## Status

| # | Step | Result | Evidence |
| --- | --- | --- | --- |
| 1 | Agent conversations | not reached | |
| 2 | Interrupt, archive, reorder | not reached | |
| 3 | Launcher | not reached | |
| 4 | Live pass | not reached | |
| 5 | Dry run on real data copy | not reached | |

## Open for the user

(filled in as the build goes)

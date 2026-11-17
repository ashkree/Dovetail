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
| 1 | Agent conversations | pass | live: tree `c` opened slice `hi`'s view (brief, reply, `⚙ Bash` tool lines); typed to it from its own bar; a story streamed in; a `Write` permission came up in the view and in the inbox (`⚠ 1 need you`), `y` allowed it and `note.txt` held `ok`; `Space s s` reopened it from the picker |
| 2 | Interrupt, archive, reorder | fixed | interrupt: `Space s x` mid-story cut it off, `Dovetail · Interrupted`, slice idle. Archive: `dd` on `say-hi` → keep/archive picker → archived, slice ended, the core asked to remove the worktree (uncommitted `note.txt`), `d` kept it. Reorder: fixed (finding 3) |
| 3 | Launcher | pass | `npx tauri build --no-bundle` (2 min) → `target/release/dovetail-gui`, 15 MB, opens Home with no dev server; `scripts/dev-gui.sh --release` runs it on dev state. `scripts/install.sh` installs it as `~/.local/bin/dovetail-gui` (the install itself was not run here: see below) |
| 4 | Live pass | fixed | PR #86: new project selected after `n`, `efforts 0 / 4`, finished turn dim and uncounted, `Shift-Tab` per agent (`hi · Manual`, fixed: finding 1), `b 1` made a question-tree block. Spend: coordinator $0.20, slice $0.28 |
| 5 | Dry run on real data copy | pass | registry 3 → 4, `dovetail` state 15 → 21; Home lists Dovetail (repo `dovetail`) and Tangents; Dovetail opens with its chat from 6 Nov on and its 4 open tasks; originals unchanged (nothing newer than a marker, versions still 3 / 15) |

## Findings and fixes

1. **Fixed — `Shift-Tab` did nothing in the window.** WebKitGTK sends it as key `Unidentified`, code `Tab`; the key was dropped and the browser moved focus into a chat bar, which entered Insert mode (a stray `i` landed in the next message). `tokenOf` falls back on the code. `gui/src/keys/resolve.ts`, test in `resolve.test.ts`.
2. **Fixed — the agent view's header shrank** under a tall permission box; header, items and bar no longer shrink.
3. **Fixed (partly) — reorder did nothing between `say-hi` and a new task.** The coordinator puts a task it starts a slice for into bucket `now`; the core orders tasks only within a bucket. The tree now lists tasks in the core's order (bucket, then position), and `J`/`K` say why a task cannot pass one in another bucket. Ordering across buckets needs a core change (open question 2).
4. **Fixed — error items were dim and uncounted.** ADR 0028 counts errors as needing you; they were mapped to finished turns. They are now their own kind (`✕`, counted, dismiss / open the agent).
5. Review findings (code-review, two axes): cursor not following a reordered task, success toasts before the core answered, a subagent's bar sending prompts the core refuses, sessions the tree does not list shown as "ended", selectors in `actions.ts`, script option parsing. All fixed.

## What an old project looks like in v2 (dry run)

- **Efforts:** one, `unsorted`, holding the open (to do) tasks; archived tasks and their 17 closed slices are not shown. Its "where was I" is empty and it has no criteria.
- **Chat:** the whole v1 coordinator conversation, untagged (no effort dots), then two Dovetail notices: the coordinator moved to `projects/dovetail/home`, and "a fresh session started, seeded with the open tasks" with the seed text.
- **First open starts a coordinator at once** (`resume=false`, multi-repo note): on the real data this is one paid turn that reads the open tasks and asks what to pick up.
- **Inbox:** two error rows: "The coordinator moved…" (names the old session to reopen by hand) and, in the dry run only, "session ended unexpectedly" (the stub refused to run).
- **Home** reads counts from the state databases before they are migrated: `no such table: efforts` warnings in the log and `0 running / 0` until the project has been opened once. Harmless.
- How it was run: copies of `~/.local/share/dovetail` (without `upgrade/`), `~/.config/dovetail` and `~/.local/state/dovetail` in `target/dovetail-dev/`; worktree paths in the copies' `agents` rewritten to the copy; `config.toml` with `claude =` a stub that answers `--version` and refuses anything else, so no session could start.

## Not done here

- `scripts/install.sh` was not run: replacing `~/.local/bin/dovetail` was refused by this session's permission check. No Rust changed on this branch, and the installed binary was built after PR #86 merged, so the live pass used a current core.
- Not exercised live: a subagent's view, `ga` on a card, `Enter` on a report line, paging older messages with `gg` (each is covered by the unit tests or is a thin call into the same `showAgent`).

## Open for the user

1. **Keys** (proposals like the rest of the keymap): tree `c` (an agent's conversation), chat `Enter` on a report line, canvas `ga`, the Agent scope's keys.
2. **Reorder across buckets:** should the core order tasks per effort in v2 (buckets are a v1 idea the GUI does not show), or should `J`/`K` move a task into its neighbour's bucket?
3. **What `dd` on a slice row means:** now "end the slice" (worktree and branch stay). Alternatives: archive its task, or nothing.
4. **Agent view answers only the oldest open item** of that agent; a picker among several was not built.
5. **A desktop entry / app-menu launcher** for `dovetail-gui` (not added; one `.desktop` file would do it).
6. **Coordinator chat paging:** `gg` in the agent view loads older messages; the coordinator chat does not yet.

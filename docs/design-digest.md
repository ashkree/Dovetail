# Dovetail — design digest

The implementation-relevant parts of the Dovetail design doc, condensed for coding agents. The full design doc (decision log, rationale, spike write-ups) should sit next to this file as `docs/design.md`; where they differ, `docs/design.md` wins. All decisions are **locked** — do not reopen them; flag contradictions instead.

Spike results were measured on Claude Code **2.1.312** (Oct 29, 2026). Re-check behaviour that depends on them if the installed version differs.

---

## 1. Decision log (condensed)

| # | Decision | Choice |
| --- | --- | --- |
| 1 | What Dovetail tracks | Every agent in one tree: full headless sessions (coordinator, long-lived slices) with take-over; subagents spawned by Claude for quick work |
| 2a / 2b | Leader / chat input | Space, Normal mode only. Plain input; `Esc` → Normal; interrupt is a Normal-mode key |
| 3 | Orchestration | Native cross-session messaging first, then a wrapper MCP server for enforced state |
| 4a | Driver | Community Rust SDK crate behind a `SessionDriver` trait; fallbacks: own protocol layer, or official SDK as a sidecar |
| 4b | Process ownership | v1 single app with project persistence; split into a daemon later for remote |
| 5a–c | State | Global `registry.db` + one folder per project, nothing in repos; app state + search index (no tool output); transcripts referenced not copied; `rusqlite` (bundled) on a dedicated thread, WAL |
| 6a / 6b | Panes / projects | Buffers from day one; fixed layout in v1, splits later. One active project |
| 7a–c | Inbox | Two tiers (Needs you / Activity); every finished turn lands in Needs you, `ask_user` adds a blocked flag; approvals inline with a context preview |
| 8 | Wrapper MCP | 8a stdio MCP server = same binary run as `dovetail mcp`, connecting back to the app's unix socket; 8b tools in order (see §6); 8c short role prompts in v1 via `--append-system-prompt-file`, plugin skills later |
| 9 | Lifecycle | Coordinator always live with a higher memory cap; quick work as subagents; per-session cgroup caps + memory display in v1; idle reaping later |
| 10a–c | Keymap | Mnemonic leader groups + single-key hot actions, which-key popup; hardcoded → TOML → Lua; global movement, buffer-local actions |
| 11 | Tasks | Two layers: user tasks in Dovetail; under each task its agents (background subagents as one node each, from `task_*` events) and native to-do items from sessions and foreground subagents |
| 12, 12a–c | Worktrees | Strictly one worktree per slice session; coordinator in the main checkout; subagents share the parent's unless Claude isolates them. Created by a `WorktreeCreate` hook passed in `--settings`, under Dovetail's project folder; sessions still start with `--worktree`. Per-project recipe: files to **symlink** (never copy) + optional setup command; slice starts only after setup succeeds |
| 13 | Editor / file tree | Deferred |
| 14a / 14b | Switcher | Telescope-style `<leader>ss`, Dovetail's agents only; `<leader><Tab>` alternate agent |
| **15** | **Default layout** | **Layout B: task tree over the inbox on the left; conversation full height on the right** |
| 15 (perm.) | Permission mode | Every session starts with `--permission-mode auto`; `<leader>sm` cycles auto → acceptEdits → manual via the control protocol (no restart); badge on non-auto agents; if init reports non-auto, run manual and warn in the status bar |
| 16 | Extra views | Board (kanban by status) `<leader>b`, worktree view `<leader>g`, focus mode `<leader>z` (coordinator only) — buffers in the main region |
| 16a–c | Readability | Blank rows between blocks, 120-col reading column; bold/italic/code each keep style + get a colour, off-white body; transparent by default, solid-background toggle, auto-on in focus mode |
| 17, 17a–e | Inbox keys / daily cycle / priority | `y A d D r x Enter`; end-of-day record (facts + coordinator narrative + user notes) as `journal/YYYY-MM-DD.md`; "Close the day?" on quit, morning review on first open of a day, `:eod` / `:morning`; review buffer with `y`/`n`/`A`; P0–P3 + Now/Next/Later buckets, mismatch flagged |
| 18 | Side-chat helper | Separate lightweight headless session on a faster model, read-only + exploration tools, one per grill, auto-starts on a `grill*` Skill call, `<leader>?` swaps it into the inbox region, `<leader>y` copies its answer into the coordinator input |
| 19 | Notes / decisions | User notes + `log_decision` tool; one markdown file per task (`tasks/<id>.md`), edited by handing off to nvim, indexed; task detail view |
| 20 | Search | FTS5 over notes, decisions, journal, messages, inbox, conversation text (no tool output); deep transcript search on demand; `<leader>/` picker |
| 21 | Quick capture | `<leader>t` and `dovetail add "…"`; `!p0–3 @now/@next/@later #1231` tokens; Unsorted group; switchable link to the focused task (`Ctrl-l`); coordinator `create_task` |
| 22 | Context fill | Coordinator % in status bar, slices' in tree; amber 60 %, red 80 %; estimated from per-turn token usage ÷ model context window. Above 80 %: coordinator offered a fresh start with handoff; slices restart automatically with handoff; fresh coordinator every morning |
| 23, 23a–i | Visual maps | One structured map model; v1 = image canvas (kitty graphics) with an outline beside it and as fallback; agents draw via `draw_map`; merman behind a `MapRenderer` trait (fallback elkjs in QuickJS); nine node kinds; structure maps (merman) + swimlane flow maps (laid out by Dovetail on a grid); MAP mode with mouse capture only there; guided flows `n`/`p`; ids + versions |
| 24 | Keymap confirmations | Inbox always-allow is `A`; `<leader>a` activity; tree `Enter o a e s b p J K dd`; map mode `f u c s` |
| 25 | Agent tools | v1 `ask_user`, `report_status`, `draw_map`; v1.1 `log_decision`, `create_task`, `claim`/`release`/`list_claims`; v2 `when`; later `notify` |

---

## 2. Architecture

```
TUI (buffers, modes, keymap)
   │ actions / snapshots + events
Core ── SQLite (registry.db + state.db per project)
   │ NDJSON over stdio            ▲ unix socket
Claude Code sessions ──MCP tools──► MCP bridge (dovetail mcp, one per session)
   └── native messaging between sessions (per-session unix sockets)
```

- **Headless sessions.** Each session is a `claude` process speaking NDJSON over stdin/stdout (the Agent SDK protocol: `-p --input-format stream-json --output-format stream-json`). No terminal emulator is embedded.
- **`SessionDriver` trait**: start, send, interrupt, set permission mode, answer control requests, event stream. The rest of the app never sees the crate behind it.
- **Take-over** (`<leader>st`): stop the headless process, open native `claude --resume <id>` in a terminal, resume headless on exit. A lock prevents headless and native running the same session.
- **Launch-time injection, never repo files**: `--settings` (hooks incl. `WorktreeCreate`/`WorktreeRemove`, `crossSessionInbound: "accept"`), `--mcp-config` pointing at `dovetail mcp`, `--append-system-prompt-file` with the role prompt, `--permission-mode auto`, env `CLAUDE_CODE_ENABLE_TODO_TOOLS=1` and `CLAUDE_CODE_TASK_LIST_ID=<task id>`. **Never bare mode** (it disables the messaging socket).
- **Core / UI split**: v1 is one process; later the core becomes a daemon and the TUI a client over a unix socket (and remote over SSH). The boundary is message-passing from day one.

## 3. Stream handling rules (Spike 1)

- Background subagents run **inside their parent's process** (~20–25 MB PSS each vs ~210 MB per extra session).
- Subagent events carry `parent_tool_use_id` (the Agent call that launched them) — route them to the right tree node with it.
- `system` events with subtypes `task_started`, `task_progress`, `task_updated`, `task_notification`, `background_tasks_changed` carry `task_id`, `subagent_type`, `spawn_depth`, per-subagent usage (tokens, tool uses, duration) and an output file. They drive subagent status and cost. They are **not** the to-do list.
- A session emits one `result` per parent turn, and the parent wakes for each finished subagent (an extra `system/init` each). **Never treat the first `result` as done**; a session is done when its process exits or all background tasks have reported. With a background subagent running there are two `result` events.
- The final `result` undercounts turns/duration; sum usage from the task events.
- Which subagent type Claude picks varies between runs.

## 4. Native to-do lists (Spike 2)

- Task tools (`TaskCreate`, `TaskGet`, `TaskList`, `TaskUpdate`) are off in `-p` unless `CLAUDE_CODE_ENABLE_TODO_TOOLS=1` — set it on every session.
- Foreground subagents share the parent's list; **background subagents can't use the task tools at all**; lists persist after exit and are shared by list id.
- Layout: `~/.claude/tasks/<list-id>/<task-id>.json`, one pretty-printed JSON file per item; list id sanitised to `[a-zA-Z0-9_-]`, defaulting to the session id.
- Fields: `id` (integer as string, never reused), `subject`, `description`, `status` (`pending`/`in_progress`/`completed`), `blocks`, `blockedBy` (stored on both sides).
- Files are rewritten in place under a lock dir; reads take no lock → reread on JSON parse error, or act only on close-write events. Deletes remove the file, strip the id from others, and update `.highwatermark`.
- **No attribution on disk**: map item → agent from the stream (`TaskCreate` results carry the id; subagent calls carry `parent_tool_use_id`) and store the mapping in `state.db`.
- Dovetail deletes the lists it created when their task is archived.

## 5. Permissions (Spike 3)

- `--permission-mode auto` works headless and cleared all ten ordinary coding actions in testing (manual: 8 of 10 needed approval; acceptEdits: 5). Without the flag, headless runs start in manual; `defaultMode` in project settings is ignored. Read `permissionMode` from the init event and warn if it isn't `auto`.
- Approvals go through the **SDK control protocol** (permission callbacks over stdio), not an MCP permission tool: requests carry more context (subagent id, reason, rule suggestions for "always allow").
- **Answer requests concurrently** — a held request must not block others. Requests simply wait for the answer (150 s held fine), including from background subagents.
- Tool results carry permission-decision metadata; denials appear in the `result`.
- Untested: classifier denials and what auto mode still escalates. Expect them in real use.

## 6. Worktrees (Spike 4 — passed)

- `WorktreeCreate` hook (in `--settings`) creates each worktree under `~/.local/share/dovetail/projects/<id>/worktrees/<slice>` (must be outside the repo), symlinks the recipe's files, runs the setup command, prints the path. It runs on **every** `--worktree <name>` start, so it must be **idempotent and fast**.
- Isolation holds for the session and its subagents (Edit/Write and `git -C <main>` into the main checkout blocked; plain shell writes are not). Symlinked `.env` is readable but not writable through the link. Headless resume returns to the worktree. Nothing written into the repo except branches.
- Hook input: `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `name`; subagents get `name = agent-<id>`, `cwd` = parent's worktree, plus `prompt_id`.
- **Transcripts are filed under the worktree path** (`~/.claude/projects/<encoded worktree path>/`), and the hook's `transcript_path` is never written — Dovetail records each session's id and transcript location itself.
- `WorktreeRemove` doesn't fire in `-p` or when subagents finish — **Dovetail cleans up itself**: unchanged subagent worktrees as soon as the subagent finishes; slice worktrees on task archive, after checking for uncommitted/unpushed work and asking. Remove its own symlinks/setup files first (plain `git worktree remove` fails otherwise), then `git worktree remove`, then delete the `dovetail/*` branch.

## 7. Data and persistence

```
~/.local/share/dovetail/
├── registry.db          # projects: id, name, repo path, git common dir, last opened
└── projects/<project-id>/
    ├── state.db         # tasks, agents, claims, inbox, message index, maps, FTS
    ├── worktrees/       # one per slice session
    ├── tasks/<id>.md    # notes + decision log (v1.1)
    ├── journal/         # YYYY-MM-DD.md (v1.1)
    └── logs/
~/.config/dovetail/
├── roles/               # coordinator.md, slice.md (8c), < ~20 lines each
└── projects/<id>.toml   # worktree recipe: files to symlink, setup command
```

- Project identity = `git rev-parse --git-common-dir`, so every worktree of a repo maps to one project.
- `state.db`: tasks (status, P-level, bucket, order, deps, PR), agents (kind, session id, parent, task, worktree, branch, status line, transcript path), item→agent mapping, claims, inbox items + answers, message index, maps.
- `rusqlite` (bundled) on a dedicated DB thread, WAL mode, small migration layer.
- **Restore on open**: coordinator resumed immediately; other sessions resumed by session id when first touched.

## 8. Coordination

- **Native messaging** (Claude Code ≥ 2.1.224): per-session unix sockets, `ListAgents` / `SendMessage`. Workers launch with `crossSessionInbound: "accept"`. Messages carry no authority. The message timeline is built from `SendMessage` tool calls and received messages in each stream (how inbound messages appear is an **open spike**). Warn at startup if `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, `DISABLE_TELEMETRY`, `DO_NOT_TRACK` or `DISABLE_GROWTHBOOK` is set (any can silently disable messaging).
- **Agent teams ruled out** (can't spawn teammates from non-interactive sessions).
- **Wrapper tools** served by `dovetail mcp`:

| Tool | Purpose | Coordinator | Slices | Helper | Ships |
| --- | --- | --- | --- | --- | --- |
| `ask_user` | Question to the inbox, optionally blocking | ✓ | ✓ | – | v1 |
| `report_status` | One-line status for the tree | ✓ | ✓ | – | v1 |
| `draw_map` | Create/update a map | ✓ | ✓ | ✓ | v1 |
| `log_decision` | Add to a task's decision log | ✓ | ✓ | – | v1.1 |
| `create_task` | Create a task (lands in Unsorted) | ✓ | – | – | v1.1 |
| `claim` / `release` / `list_claims` | PR and task ownership | ✓ | ✓ | – | v1.1 |
| `when` | Rules like "rebase #1231 after #1230 merges" | ✓ | – | – | v2 |
| `notify` | Wake a stopped slice | ✓ | – | – | later |

## 9. Lifecycle and memory

- Budget: ~210 MB PSS per headless session at rest, +20–25 MB per subagent; some versions have leaked to 12+ GB, so caps matter. The real pressure is the tools agents run (typechecks, test suites).
- Each session runs in its own cgroup with a hard cap: `systemd-run --user --scope -p MemoryMax=…` (works without root — **open spike** on the target machine). Coordinator gets a higher cap.
- A cap kill or crash kills one session and its subagents; Dovetail restarts it from its transcript and lists in-flight subagent work (task ids, output files) offering to restart each piece.
- Per-session memory in the tree (from `/proc`), total in the status bar.
- Coordinator never stopped; slices run until closed. Idle reaping is **later**.

## 10. UI essentials

- **Layout B** (Decision 15 — supersedes the UI brief's sketch):

```
┌ tasks ──────────────────┬ coordinator ──────────────┐
│ ◐ ◆ coordinator         │ you · 10:14               │
│ ▾ #1231 business surface│ What's blocking slice-7?  │
│   ◐ ◆ slice-4           │                           │
│     ⎇ dovetail/slice-4  │ coordinator · 10:14       │
│     └ ✓ ◇ rebase        │ It needs FS_EVAL_VIRTUAL… │
│ ▾ ⚠ get_decision 500    │                           │
├ inbox (2) ──────────────┤                           │
│ ▸ ⚠ slice-7  Bash       │                           │
│   ? slice-4  Question   │ › i to chat               │
├─────────────────────────┴───────────────────────────┤
│ NORMAL  business · 3 tasks · 2 agents · 2 ⚠ · 2.1G  │
└─────────────────────────────────────────────────────┘
```

- Buffers: conversation, task tree, inbox, activity, message timeline, task detail, board, worktree view, map, review, side helper. Any region can show any buffer.
- Modes: Normal, Insert, Command; Visual later; MAP mode for maps.
- Everything else (keys, switcher, readability, quick capture, inbox keys) is in `docs/ui-session-brief.md` and the README.

## 11. Visual maps (v1 stage 1)

- Model: map (id, title, type `structure`|`swimlane`), nodes (id, kind, label, description, optional file link, optional pseudocode), groups, connections (from, to, label), flows (name, ordered steps with notes), lanes. Stored per project, linked to a task, versioned by id.
- Node kinds (fixed nine): Part, File, Endpoint, Data, External, Actor, Outcome, Logic, Concept.
- `draw_map` validates input and returns specific errors (unknown kind, connection to a missing node, step in an unknown lane).
- Structure maps → Mermaid flowchart → merman → PNG → kitty graphics (e.g. `ratatui-image`); selection re-renders with a highlight class. Swimlanes laid out by Dovetail on a lane × step grid. Map code talks only to `MapRenderer`.
- v1 view: image canvas with `hjkl` node selection, `Enter` drill-in, flows `n`/`p`, keyboard pan/zoom; outline beside it and as the fallback in terminals without images; a map shelf. The text box-drawing renderer was **dropped**.
- merman caveats: pre-1.0, node positions undocumented (may need reading from the SVG) — **open spike**.

## 12. Roadmap

**v1** — SessionDriver on a community crate · headless launch with injected settings/hooks/role prompts · registry + `state.db`, resume after restart · buffers in the fixed layout · Normal/Insert/Command, hardcoded keymap via the action registry, `nucleo` picker · two-tier inbox with inline approvals · maps stage 1 · context fill display · quick capture + `dovetail add` + `create_task` · readability · auto mode + `<leader>sm` + concurrent approvals · one worktree per slice via hooks · task tracker · agent tree with subagents · background subagent nodes + native to-do items · take-over · cgroup caps + memory display · `ask_user`, `report_status` · role prompts.

**v1.1** — claims · TOML keymap + which-key · maps stage 2 · fresh starts with handoffs · task notes + decision log · search · side-chat helper · daily review cycle · syntax highlighting, theme config, focus mode · Dovetail plugin with skills.

**v2** — `when` rules · splits · maps stage 3 · Visual mode. **Later** — daemon + remote attach, Lua, idle reaping, editor/file tree, other local sessions in the switcher.

## 13. Open spikes (gate the work that depends on them)

| Spike | Gates | If it fails |
| --- | --- | --- |
| Driver crate: test two community crates (e.g. `claude-agent-sdk-rs`, `rusty_claw`) in a two-session test — multi-session, interrupt, in-process permission callbacks, permission-mode switching | Session driver | Hand-written NDJSON layer, or official SDK as a sidecar |
| Inbound `SendMessage` in a headless stream | Message timeline | Timeline from sender side only |
| `systemd-run --user --scope -p MemoryMax=…` without root on the target machine | Memory caps | Another capping method, or none |
| merman: 30-node map, two nesting levels, one style per kind → PNG; time, memory, node positions, look in kitty | Map canvas | elkjs in QuickJS behind `MapRenderer` |
| MCP tools in background subagents | Subagents using `ask_user`/`draw_map` | Route through the parent session |
| `ask_user` role-prompt test (run during the build) | Role prompt wording | Stronger prompting or a skill |
| Terms of use for running Claude Code headless this way | Everything | Rethink (user's call, not an agent task) |

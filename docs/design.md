# Dovetail — Design

Oct 26, 2026 · @Maveron Tyriel V. Aguares

## Overview

**Dovetail** is a keyboard-driven Rust TUI for running a coordinator Claude Code session plus several worker slices on one feature, replacing the current tmux + fzf setup. The coordinator is the main collaborator; slices work in their own worktrees and come to you through an inbox.

**Goals**

- See every session's real state (working, idle, blocked on you) without visiting panes.
- Answer questions and approvals from one place.
- Keep track of projects and tasks, so nothing in flight is forgotten at the speed sessions move.
- Enforce coordination facts (PR ownership, merge order) instead of trusting prose.
- Neovim-style modal control: Space leader, `hjkl` between panes, `i` to chat.

**Constraints**

- Target machine: 16 GB RAM; the GPU now works and kitty is the default terminal. Memory is a first-class requirement.
- Nothing written into project repos. Hooks and settings are passed as launch flags.
- One project per work session.
- Remote use (daemon on a dev box) must be possible later without a rewrite.

## Decision log

All decisions are locked.

| # | Decision | Choice | Status |
| --- | --- | --- | --- |
| 1 | What Dovetail tracks | Every agent in one tree: full sessions (headless, with take-over) for the coordinator and long-lived slices; subagents, spawned by Claude, for quick work. Revised Oct 29 | Locked |
| 2a | Leader key | Space, Normal mode only | Locked |
| 2b | Chat input | Plain input; `Esc` returns to Normal; interrupt is a Normal-mode key | Locked |
| 3 | Orchestration | Native cross-session messaging first, then a wrapper MCP server for enforced state | Locked |
| 4a | Rust ↔ Claude Code driver | Community Rust SDK crate behind a `SessionDriver` trait; own layer or official SDK sidecar as fallbacks | Locked |
| 4b | Process ownership | v1 single app with project persistence; split into a daemon later for remote | Locked |
| 5a | State location | Global `registry.db` + one data folder per project; nothing in repos | Locked |
| 5b | What is stored | App state + search index of messages, inbox, notes, journal and conversation text (no tool output); full transcripts referenced, not copied. Revised by Decision 20 | Locked |
| 5c | SQLite crate | `rusqlite` on a dedicated thread | Locked |
| 6a | Pane model | Buffers from day one; fixed layout in v1, splits later | Locked |
| 6b | Project focus | One active project (badge for other projects later) | Locked |
| 7a | Inbox scope | Two tiers: Needs you / Activity | Locked |
| 7b | Question detection | Every finished turn lands in Needs you; `ask_user` adds a blocked flag later | Locked |
| 7c | Approvals | Inline in the inbox with a context preview | Locked |
| 8 | Wrapper MCP tools | 8a stdio MCP server (`dovetail mcp`); 8b tools in the proposed order; 8c role prompts in v1, plugin skills later | Locked |
| 9 | Session lifecycle | Coordinator always live with a higher memory cap; quick work runs as subagents; per-session cgroup caps and memory display in v1; idle reaping deferred to later. Revised Oct 29 | Locked |
| 10a | Leader layout | Mnemonic groups + single-key hot actions, which-key popup | Locked |
| 10b | Keymap config | Hardcoded → TOML → Lua | Locked |
| 10c | Key scope | Global movement, buffer-local actions | Locked |
| 11 | Task tracking | Two layers: your tasks in Dovetail; under each task, its agents (background subagents as one node each, from `task_*` events) and native to-do items from sessions and foreground subagents. Revised Oct 29 (11b) | Locked |
| 12 | Sessions and worktrees | Strictly one worktree per slice session; the coordinator runs in the main checkout; subagents share their parent's worktree unless Claude isolates them | Locked |
| 12a | Worktree location | Dovetail's project folder, created by a WorktreeCreate hook passed in --settings; sessions still start with --worktree so Claude Code enforces isolation | Locked |
| 12b | Worktree setup | Per-project recipe in Dovetail's config: files to symlink in (e.g. .env) plus an optional setup command; the slice starts only after setup finishes | Locked |
| 12c | Link or copy setup files | Link only. Agents can read linked files but can't write through them (Spike 4), so the main .env stays the single, protected source of truth; slices can't have their own values | Locked |
| 13 | Editor and file tree | Deferred to Later. Options kept: hand off to nvim in the agent's worktree; built-in changed-files tree and diff view; embedded nvim via nvim --embed; a neovim plugin over the socket API. Limit to one editor-opened worktree at a time because of language-server memory | Deferred |
| 14a | Session switcher scope | Telescope-style switcher (\<leader>ss) listing Dovetail's agents only: sessions and their subagents. Other local Claude sessions later | Locked |
| 14b | Jump-back key | \<leader>\<Tab> toggles between the current agent and the previous one | Locked |
| 15 | Default layout | Layout B: task tree over the inbox on the left, the conversation full height on the right. Chosen Oct 29 from three mocks | Locked |
| 16 | Extra views | Board (kanban by task status), worktree view and focus mode (coordinator only, needs-you count kept), as buffers in the main region | Locked |
| 17 | Inbox deny keys | d / D deny (with reason), r reply, x dismiss; n / N stay global search | Locked |
| 15 | Default permission mode | Every session starts in auto mode (--permission-mode auto). \<leader>sm cycles the focused agent between auto, acceptEdits and manual; non-auto agents get a badge in the tree. If auto is unavailable, fall back to manual and warn in the status bar | Locked |
| 16a | Readability: spacing | Blank row between paragraphs, list items and around code blocks; reading column capped at 120 columns. Line height itself is left to the terminal | Locked |
| 16b | Readability: emphasis | Keep bold, italic and code styling, and also give each its own color in a soft reading theme (off-white body, no pure white) | Locked |
| 16c | Readability: background | Transparent by default; a toggle gives the reading column a solid dark background, switched on automatically in focus mode | Locked |
| 17a | End-of-day record contents | Facts from Dovetail's data, a short narrative from the coordinator, and your own notes | Locked |
| 17b | Record location | Dated markdown files in Dovetail's project folder (journal/), indexed for search; a custom folder is a setting | Locked |
| 17c | Triggers | "Close the day?" when quitting with no record yet; morning review opens on the first open of a new day (one key to skip); :eod and :morning also available | Locked |
| 17d | Morning review | The coordinator proposes concrete task changes in a review buffer; y / n per change, A to accept all; Dovetail applies what you accept | Locked |
| 17e | Task priority model | Both: a P-level (P0–P3) for importance and a Now / Next / Later bucket for timing. The tree groups by bucket and sorts by P-level; a mismatch (e.g. a P0 in Later) is flagged for the morning review | Locked |
| 18 | Side-chat helper for grilling | A separate lightweight session (18a), started with your current focus (18b), read-only plus exploration tools (18c), one per grill (18d), shown by swapping the bottom-right region (18e) | Locked |
| 19 | Task notes and decision log | You write notes; agents log decisions with a log\_decision tool, and the end-of-day narrative proposes unlogged ones (19a). Entries: what, why, rejected alternatives, who, when, link to the turn (19b). One markdown file per task, edited in nvim, indexed (19c). Shown in a task detail view and fed to briefs, the helper and the morning review (19d) | Locked |
| 20 | Search | Index Dovetail's own content plus conversation text (no tool output), with on-demand transcript search for the rest (20a). Current project by default, toggle for all (20b). \<leader>/ picker, and plain-language questions via the side helper (20c). Revises 5b | Locked |
| 21 | Quick capture | One line with optional !p / @bucket / #PR tokens; \<leader>t in Dovetail and dovetail add in a shell; coordinator create\_task tool; untagged tasks go to an Unsorted group cleared by the morning review; visible, switchable link to the focused task | Locked |
| 22 | Context fill | Coordinator % in the status bar, slices' in the tree (amber 60%, red 80%). Above 80%: the coordinator is offered a fresh start with a handoff; slices restart automatically in the same worktree with a handoff. A fresh coordinator every morning, seeded with the previous one's handoff | Locked |
| 23 | Visual maps | One structured map model with two views: an interactive text navigator and a view-only image canvas (kitty graphics), plus export (23a). Agents draw maps on request with a draw\_map tool; automatic code maps eventually (23b). Revised Nov 1: the image canvas is the v1 target, with the outline beside it and as fallback; the text drawing is dropped | Locked |
| 23c | Map layout engine | merman (headless Rust Mermaid with compiled-in ELK layout, SVG and PNG output), behind a MapRenderer trait; Dovetail's map model is converted to a Mermaid flowchart for drawing. Fallback: elkjs in an embedded QuickJS engine. Revised Nov 1, pending a spike | Locked |
| 23d | Map node kinds | A fixed set of nine: Part, File, Endpoint, Data, External, Actor, Outcome, Logic (pseudocode card), Concept. Any node can also carry an optional pseudocode snippet shown in the side panel | Locked |
| 23e | Map types | Structure maps (flowchart style, drawn with merman) and flow maps as horizontal swimlanes (a lane per participant, time left to right), laid out by Dovetail itself on a grid, in the same visual style | Locked |
| 23f | Map interaction | A MAP mode (Enter on a map, \<leader>v, or the expand button) with the map full-screen, mouse capture on only in this mode, and keys alongside; Esc or ✕ leaves | Locked |
| 23g | Guided flows | Step-by-step walkthroughs created by agents with draw\_map; you move through them manually (n / p or click), no auto-play; a summary of all step notes at the end | Locked |
| 23h | Map updates | Maps have ids; redrawing with the same id updates in place and keeps the previous version; you can ask the coordinator to update a map (right-click or u); comments stay on nodes that still exist; new subjects get new maps | Locked |
| 23i | draw\_map format | Dovetail's own structured input (map, nodes, groups, connections, flows, lanes), validated with clear errors; converted to Mermaid for structure maps or laid out by Dovetail for swimlanes. Mermaid import as an extra in v1.1 | Locked |
| 24 | Keymap confirmations | Inbox always-allow is A (Shift-a). Confirmed for now: \<leader>a activity; tree Enter, o, a, e, s, b, p, J/K, dd; map mode f, u, c, s. Revisit when keymaps move to TOML in v1.1 | Locked |
| 25 | Agent tool list | ask\_user, report\_status, draw\_map (v1); log\_decision, create\_task, claim / release / list\_claims (v1.1); when (v2); notify (later). Split by role as in the Coordination section; the side helper gets draw\_map only | Locked |

## Architecture

One Rust binary drives every Claude Code session headless and owns all state; the TUI is a view over that core.

**Sessions run headless.** Each session is a `claude` process speaking NDJSON over stdin/stdout (the protocol the Agent SDKs use). The app receives structured events (messages, tool calls, permission requests) and sends prompts, interrupts and permission answers. No terminal emulator is embedded.

**`SessionDriver` boundary.** All Claude Code interaction goes through one trait: start, send, interrupt, event stream. A community Rust SDK crate sits behind it (4a). If the crate lags or fights multi-session use, a hand-written protocol layer or the official SDK as a sidecar can replace it without touching the rest of the app.

**Take-over escape hatch.** `<leader>st` stops a session's headless process, opens native `claude --resume <id>` in a terminal, and resumes it headless on exit. The app holds a lock so headless and native never run the same session at once.

**Launch-time injection.** Every session is started with flags, never repo files: `--settings` (hooks, `crossSessionInbound: "accept"`), an MCP config pointing at the wrapper's tools, and an appended role prompt. Bare mode is never used, since it disables the messaging socket.

**Core / UI split.** Core logic sits behind an internal API (the same action registry the keymap and `:` commands call). v1 runs core and TUI in one process; later the core becomes a daemon and the TUI a client over a unix socket, which also enables remote attach over SSH.

&#91;embedded content: architecture · core, TUI, sessions and channels\]

The TUI only calls the core; the core drives sessions over stdio and receives their tool calls through a per-session MCP bridge, while sessions talk to each other over native sockets. Dashed boxes are sometimes absent: a reaped slice, or a session taken over natively.

## Agents and tasks

Your tasks are the main thing Dovetail shows; agents of any kind are workers attached to them (Decisions 1 and 11).

**Agent tree.** Dovetail tracks every agent, not only the sessions it starts.

| Kind | Started by | You can | Dovetail sees it through |
| --- | --- | --- | --- |
| Session (coordinator, long-lived slice) | Dovetail | Chat, approve, take over | Its own headless event stream |
| Subagent | A session, at Claude's choice | Read its transcript; reach it through its parent | `SubagentStart` / `SubagentStop` hooks and transcripts under `~/.claude/projects/…/subagents/` |

Rule of thumb: quick, self-contained work runs as subagents, chosen by Claude. A slice that owns a PR across days is a full session, since each subagent invocation starts fresh.

**Two-layer tasks.**

- **Your tasks** live in `state.db`: title, status, dependencies, notes, linked PR and linked agents. A task can exist with no agent yet (an idea, a follow-up).
- **Under each task (Decision 11b):** what actually exists, nothing guessed.
  - **Background subagents** appear as one node each: description, live progress, summary when done, and cost, from the stream's `system` `task_*` events. They have no checklist, since they can't use the task tools.
  - **Native to-do items** from sessions and foreground subagents appear under the agent that created them. A session started for a task gets `CLAUDE_CODE_TASK_LIST_ID` set to that task's id and `CLAUDE_CODE_ENABLE_TODO_TOOLS=1`; the coordinator gets a project-level list. Dovetail reads `~/.claude/tasks/<list-id>/` and attributes each item using the stream, storing the item-to-agent mapping in `state.db`.
  - Dovetail deletes the native lists it created when you archive their task.
- **Your layer wins.** A task is done when it's marked done in Dovetail, not when its subtasks are all ticked.

**Worktrees (Decision 12).** Each slice session has exactly one worktree, and no worktree has more than one session. This prevents two agents editing the same files.

- **Coordinator:** runs in the main checkout and owns no branch. Its role prompt tells it not to edit there.
- **Slices:** started with `claude -p --worktree <slice>`, so Claude Code's isolation checks block edits, commands and git redirects into the main checkout, for the slice and every subagent it spawns. Resume returns a slice to its worktree.
- **Subagents:** share their parent's worktree by default. When Claude gives one its own worktree, it also goes through Dovetail's hook (named `agent-<id>`, branched from the parent worktree's HEAD), so Dovetail owns its cleanup too.
- **Location (12a):** a `WorktreeCreate` hook, passed in `--settings`, creates each worktree under `~/.local/share/dovetail/projects/<id>/worktrees/<slice>` and prints its path. The folder is outside any repository, as Claude Code requires for hook-created worktrees. A `WorktreeRemove` hook handles removal. Because a hook replaces the default logic, Claude Code's `.worktreeinclude` is not used.
- **Setup (12b):** the hook symlinks the project's listed files (e.g. `apps/helix-backend/.env`) into the new worktree, then runs the optional setup command. The slice's first prompt is sent only after setup succeeds.
- **Cleanup:** Claude Code never removes these worktrees in headless mode, and the `WorktreeRemove` hook almost never fires. Dovetail removes them itself:
  - a subagent worktree with no changes, as soon as the subagent finishes;
  - a slice worktree, when you archive its task. Before removing, Dovetail checks for uncommitted or unpushed work and asks first. It deletes its own symlinks and setup files, then runs `git worktree remove` and deletes the `dovetail/*` branch.
- **Picker and commands:** a way to open a slice's worktree in your editor, since the paths are long.

**Spike 4 results (Oct 29, Claude Code 2.1.312).** Hook-created worktrees outside the repo get the same isolation as default ones, and nothing is written into the repo. Dovetail keeps this design; no fallback needed.

| Test | Result |
| --- | --- |
| Creation via hook | Pass: the session starts in the hook's path, with the symlink and setup in place |
| Isolation (session and subagents) | Edit/Write into the main checkout, `git -C <main>` and `cd <main> && git ...` are blocked. Reads and plain shell writes (`echo > file`, `touch`) into the main checkout get through, as the docs warn |
| Symlinked `.env` | Readable; writes through the link are blocked, so the main `.env` is safe |
| Headless resume | Pass: returns to the worktree with no notice |
| Reuse of a name | The hook runs on every `--worktree <name>` start; it must be idempotent and fast |
| Subagent worktrees | Go through the hook as `agent-<id>` |
| `WorktreeRemove` | Doesn't fire at the end of a `-p` session or when a subagent finishes |
| Repo untouched | Pass: no `.claude/`, `.git/info/exclude` and `.git/config` unchanged; only branches added |

- **Hook input:** `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `name`. For a subagent: `name` is `agent-<id>`, `cwd` is the parent's worktree, and a `prompt_id` is added.
- **Transcripts** are filed under the worktree's path (`~/.claude/projects/<encoded worktree path>/`), not the launch directory, contrary to the docs. The `transcript_path` the hook receives is never written. Dovetail must record each session's id and transcript location itself.
- **Leftovers:** without cleanup, worktrees and `dovetail/agent-*` branches pile up.
- **Plain `git worktree remove` fails** because of the untracked symlink and setup files; Dovetail removes those first.
- **Not covered by isolation:** shell commands that write into the main checkout. Claude Code's sandbox could restrict this later.

**Effects on other decisions.** Quick work no longer needs its own process, so less reaping is needed (Decision 9). Agent view's per-user supervisor (`claude --bg`, `claude agents`) may later replace Dovetail's planned daemon (4b); revisit when it leaves research preview.

**Spike 1 results (Oct 29, Claude Code 2.1.312).** Background subagents run inside their parent's process. Three read-only tasks run as subagents in one session peaked at about 270 MB PSS, against about 617 MB as three separate sessions: 56% lower.

| Measure | One session + 3 subagents | 3 separate sessions |
| --- | --- | --- |
| Peak PSS (mean of 2 runs) | 270 MB | 617 MB |
| Peak RSS | 362 MB | 912 MB |
| OS processes | 1 | 3 |
| Wall clock | 150 s | 107–140 s |
| Cost (2 runs) | $1.59, $1.30 | $1.54, $1.47 |

- **Marginal cost:** each concurrent subagent adds about 20–25 MB PSS; each extra session adds about 205–220 MB PSS (about 307 MB RSS).
- **Caveats:** a small repo (xsv, \~7k lines), read-only tools, no Bash. Test runs and typechecks that agents start are separate child processes and cost memory either way.
- **Trade-off:** the parent is a single point of failure. If it crashes or hits its memory cap, every subagent inside it stops.

**Stream handling rules learned from Spike 1.**

- Subagent events carry `parent_tool_use_id` (the Agent call that launched them); route them to the right node in the agent tree with it.
- `system` events with subtypes `task_started`, `task_progress`, `task_updated`, `task_notification` and `background_tasks_changed` carry `task_id`, `subagent_type`, `spawn_depth`, per-subagent usage (tokens, tool uses, duration) and an output file. These drive subagent status and per-agent cost in the tree.
- A session emits one `result` event per parent turn, and the parent wakes for each finished subagent (one extra `system/init` each). Never treat the first `result` as done; a session is done when its process exits or all its background tasks have reported.
- The final `result` undercounts the run (turns, duration); sum usage from the task events instead.
- Which subagent type Claude picks varies between runs (Explore vs general-purpose).

**Spike 2 results (Oct 29, Claude Code 2.1.312).** Native task lists work as a shared, persistent store, but background subagents can't write to them.

| Who | Can use the task list? |
| --- | --- |
| Headless session with `CLAUDE_CODE_ENABLE_TODO_TOOLS=1` | Yes |
| Foreground subagent | Yes, same list as its parent; sees the parent's tasks |
| Background subagent | No: the task tools are never available to it |
| A second session with the same list id | Yes: lists persist after exit and are shared |

- **Opt-in:** in `-p` mode the task tools (`TaskCreate`, `TaskGet`, `TaskList`, `TaskUpdate`) are off unless `CLAUDE_CODE_ENABLE_TODO_TOOLS=1` is set. Dovetail must set it on every session.
- **Layout:** `~/.claude/tasks/<list-id>/<task-id>.json`, one pretty-printed JSON file per task. The list id is sanitized to `[a-zA-Z0-9_-]`; with no id, the session id is used.
- **Fields:** `id` (an integer as a string, never reused), `subject`, `description`, `status` (`pending`, `in_progress`, `completed`), `blocks` and `blockedBy` (stored on both sides). Optional `owner`, `activeForm` and `metadata` were never written.
- **Writes:** files are rewritten in place under a lock directory; reads take no lock. Dovetail must reread on a JSON parse error or act only on close-write events.
- **Deletes:** the file is removed, its id is stripped from other tasks' dependencies, and `.highwatermark` records it. No tombstone.
- **No attribution on disk.** Nothing records which agent created a task. The stream is the only link: `TaskCreate` results carry the task id, and a subagent's calls carry `parent_tool_use_id`. Dovetail must store that mapping itself.
- **Not to be confused:** `system` `task_*` events describe a subagent's run (lifecycle, progress, usage), not the task list.
- An untested `TaskCreated` hook could replace file watching.

## Data and persistence

State lives outside every repo: one small registry database plus a folder per project, so closing tonight and reopening tomorrow restores the whole feature.

```
~/.local/share/dovetail/
├── registry.db          # projects: id, name, repo path, git common dir, last opened
└── projects/
    └── <project-id>/
        ├── state.db     # tasks, agents, claims, inbox, message index
        ├── worktrees/   # one per slice session (Decision 12a)
        └── logs/

~/.config/dovetail/
├── roles/               # coordinator.md, slice.md (Decision 8c)
└── projects/<id>.toml   # worktree setup: files to symlink, setup command (12b)
```

- **Project identity** = the repo's git common directory (`git rev-parse --git-common-dir`), so every worktree of a repo maps to the same project.
- **Stored in `state.db`:** your tasks (status, dependencies, notes, linked PR), agents (kind, session id, parent, task, worktree, branch, status line), claims, inbox items and your answers, the inter-session message index, rules (later).
- **Referenced, not copied:** full conversations stay in Claude Code's transcripts; the app stores the session id and transcript path and reads them on demand.
- **Full-text search** over messages and inbox items via SQLite FTS5.
- **`rusqlite`** with bundled SQLite on a dedicated database thread; WAL mode so scripts can read while the app writes; a small migration crate for schema changes.
- **Restore on open:** sessions are resumed by session id when you first interact with them (or immediately for the coordinator), not all at once.

## UI and interaction

Every view is a buffer shown in a fixed v1 layout, driven by vim-style modes; the coordinator owns the main pane.

**Buffers.** Session conversation, inbox, activity feed, message timeline, project tree, PR view. Any region can show any buffer (swapped via the picker). Vim-style splits (`<leader>w…`) come later on the same abstraction.

**Default layout.**

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

**Modes.**

| Mode | Purpose | Enter / leave |
| --- | --- | --- |
| Normal | Navigate panes and lists, run leader commands | Default; `Esc` returns here |
| Insert | Type to the focused session | `i` |
| Visual | Select transcript text to yank or quote | `v`, `V` |
| Command | Ex-style commands (`:spawn`, `:merge 1231`) | `:` |

**Chat input.** Plain multiline input. `Ctrl-j` / `Alt-Enter` insert a newline (Shift+Enter only where the terminal supports the kitty keyboard protocol); `Ctrl-w` and `Ctrl-u` for basic line editing. `i` from the tree or inbox targets the coordinator; from an open slice, that slice.

**Keymap.** Space leader with mnemonic groups, single keys for the hottest actions, and a which-key popup. Movement is global; action keys are buffer-local. v1 hardcodes the map through one action registry; a TOML override file follows, then Lua.

| Keys | Action |
| --- | --- |
| `h` `j` `k` `l` | Move between panes |
| `i` / `Esc` | Chat with the focused session / back to Normal |
| `:` | Command mode |
| `/` `n` `N` | Search in the current buffer |
| `<leader><leader>` | Picker over sessions, inbox and messages |
| `<leader>c` | Jump to the coordinator |
| `<leader>n` | Jump to the inbox |
| `<leader>m` | Message timeline |
| `<leader>ss` `sn` `st` `sx` | Pick session, new slice, take over, interrupt |
| `<leader>pp` `pr` | Switch project, register project |
| `<leader>w…` | Window splits (later) |
| Inbox: `y` `A` `d` `D` `r` `x` `Enter` | Allow once, always allow, deny, deny with reason, reply to a question, dismiss, open session |
| `<leader><Tab>` | Toggle between the current agent and the previous one |
| `<leader>b` | Board: tasks as a kanban in the main region |
| `<leader>g` | Worktree view: one row per session's worktree |
| `<leader>z` | Focus mode: coordinator only |
| `<leader>sm` | Cycle the focused agent's permission mode: auto, acceptEdits, manual |

**Session switcher (Decision 14).** `<leader>ss` opens a Telescope-style overlay: a search prompt and results list on the left, a preview on the right.

- **Rows:** status glyph, agent name or task title, worktree branch, time since last activity. Subagents are indented under their parent.
- **Order:** with an empty search, agents that need you first, then most recently active; typing switches to fuzzy ranking (`nucleo`).
- **Preview:** the agent's last few turns, rendered like the main pane. For a background subagent: its description, progress and summary.
- **Keys:** type to filter; `Ctrl-j` / `Ctrl-k` or arrows to move; `Enter` opens the agent in the main pane; `Esc` closes. Later: `Ctrl-x` interrupt and `Ctrl-t` take over from the list.
- **Scope:** Dovetail's agents only. Other local Claude sessions, found through Claude Code's session records, come later.
- **Getting back:** `<leader>c` jumps to the coordinator; `<leader><Tab>` toggles between the current agent and the previous one, like vim's alternate buffer.

**Readability (Decision 16).** Dovetail renders all conversation text itself, so it is tuned for low eyesight on a transparent terminal.

- **Width:** the reading column is capped at 120 columns (configurable) and centered in wider panes. Code blocks and tables may use the full pane.
- **Spacing (16a):** a blank row between paragraphs, between list items, and around code blocks and headings. Never a dense block. Line height itself is the terminal's setting (for example foot's `line-height`).
- **Emphasis (16b):** markdown styling is kept (bold is bold, italic is italic, code is code), and each also gets its own color, so emphasis is told apart by color as well as weight:
  - body text: soft off-white, never pure white;
  - bold: a warm accent color;
  - italic: a second, distinct color;
  - inline code and file paths: a third color;
  - headings and speaker labels: the accent color;
  - tool summaries and older turns: dimmed.
- **Background (16c):** transparent by default. A toggle gives the reading column a solid dark background; focus mode turns it on automatically. Other regions stay transparent.
- **Less text:** tool activity folds into one-line summaries ("Edited 3 files · ran tests ✓ · 42 s"), expandable; thinking is hidden; long output is truncated with a "show more".
- **Navigation:** in a conversation, `j`/`k` scroll, `gg`/`G` top and bottom, `]]`/`[[` next and previous turn, `zc`/`zo` fold and unfold a turn.
- **Source text:** role prompts ask agents to lead with the outcome, keep paragraphs short and put paths and commands in code formatting.

## Inbox

Sessions come to you through a two-tier inbox: "Needs you" holds anything waiting on your answer, and a separate Activity feed holds everything else.

| Event | Tier | Source |
| --- | --- | --- |
| Permission request | Needs you (blocking) | Permission callback from the session |
| Question via `ask_user` | Needs you (blocked flag) | Wrapper MCP tool (once Decision 8 ships it) |
| Finished turn | Needs you | End-of-turn event; preview of the last message |
| Error, crash, memory kill | Needs you | Driver error or cgroup OOM |
| PR pushed, CI status | Activity | Tool events, hooks |
| Inter-session message | Activity | `SendMessage` tool calls in the sender's stream |

**Question detection.** Until `ask_user` exists, every finished turn lands in Needs you with a preview, and you skim for blockers. No heuristics. Once `ask_user` ships, explicit questions carry a blocked flag and sort first.

**Approvals.** Answered inline: `y` allow once, `A` always allow for this session (Shift-a, so a slip of focus can't grant it), `d` deny, `D` deny with a reason; `n` / `N` stay global search. `r` replies to a question and `x` dismisses a finished turn or error. The selected item shows the tool, its full input and the session's last message; `Enter` opens the session for more context.

**Answering a question** opens Insert mode on that item; the answer is sent to the session and stored with the item.

**Spike 3 results (Oct 29, Claude Code 2.1.312).** Auto mode works headless with `--permission-mode auto` and cleared every ordinary coding action, so approvals should be rare.

| Mode | Of 10 actions, sent to the inbox |
| --- | --- |
| `manual` (`default`) | 8: everything except reads and `git status` |
| `acceptEdits` | 5: tests, `pip`, network, writes outside the repo, commit and push |
| `auto` | 0: the classifier approved all of them |

- **Switching it on:** pass `--permission-mode auto`. Headless runs start in `manual` otherwise, and `defaultMode: "auto"` in project settings is ignored. It needs a supported model, and an admin can disable it (`disableAutoMode`); Dovetail should read `permissionMode` from the init event and warn if it isn't `auto`.
- **Use the SDK control protocol for approvals,** not an MCP permission tool. Over `--input-format stream-json --permission-prompt-tool stdio`, each `can_use_tool` request carries `agent_id` for subagent calls, a reason, a blocked path and the "always allow" rule suggestions. The MCP route carries only the tool, input and `tool_use_id`.
- **Requests wait for the answer.** A request held for 150 s simply waited, including from a background subagent, and the session finished normally. Dovetail must answer requests concurrently, since one held request blocked another behind it in a one-at-a-time handler.
- **Background subagents are safe headless:** their requests reach the handler like the main agent's.
- **In the stream:** each tool result carries `tool_result_meta[].permission_decision` (decision, source, reason such as `classifier`); `system` events with subtype `permission_check_status` show a check in progress (about 0.9 s); denials appear in `result.permission_denials`.
- **Untested:** a classifier denial, and what auto mode still escalates (protected paths, dangerous `rm`, ask rules, fallback after repeated blocks). Watch for these in real use.
- **Again:** with a background subagent running there are two `result` events; never stop at the first.

**Permission mode (Decision 15).** Every session Dovetail starts runs in `auto`. `<leader>sm` cycles the focused agent between `auto`, `acceptEdits` and `manual` through the control protocol, without restarting it; the tree shows a badge on any agent not in `auto`. If the init event reports a mode other than `auto` (unsupported model, or disabled by an admin), Dovetail runs in `manual` and warns in the status bar. Inbox `A` (always allow) offers the rule suggestions Claude sends with each request.

## Coordination

Conversation between sessions rides Claude Code's native messaging; facts that must be true (ownership, merge order) move into wrapper tools step by step.

**Native messaging (v1).** Since v2.1.224, each session binds a per-session Unix socket; sessions find each other with `ListAgents` and talk with `SendMessage`. Headless `-p` sessions bind sockets too, but can't show the hold dialog, so the app launches every worker with `crossSessionInbound: "accept"` in its own `--settings`. Messages are plain text with no authority: they can't approve prompts or change config. The app builds the message timeline from `SendMessage` tool calls and received messages in each session's event stream. At startup the app warns if `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, `DISABLE_TELEMETRY`, `DO_NOT_TRACK` or `DISABLE_GROWTHBOOK` is set, since any of them can silently disable messaging.

**Wrapper tools (Decision 8).** All three parts are locked:

| Part | Choice | Why |
| --- | --- | --- |
| 8a Serving | Stdio MCP server, same binary run as `dovetail mcp`, connecting back to the app's socket | Works for headless and taken-over native sessions alike |
| 8b Order | v1 `ask_user`, `report_status`; v1.1 `claim`, `release`, `list_claims`; v2 `when(event, then)` rules; later `notify` (with reaping) | Inbox first; ownership fixes double-work; `notify` only matters once slices are reaped |
| 8c Prompting | v1: short role prompts (`coordinator.md`, `slice.md` in `~/.config/dovetail/roles/`) via `--append-system-prompt-file`. Later: a Dovetail plugin with skills via `--plugin-dir` | Rules must always apply and survive compaction, so they live in the prompt (kept under \~20 lines). Long procedures cost context only when used, and Dovetail can name the skill when it sends the triggering event |

**Agent teams ruled out.** Teammates can't be spawned from non-interactive sessions, which conflicts with Decision 1.

**Agent tools (Decision 25).** Every tool Dovetail serves to agents, and who gets it:

| Tool | What it does | Coordinator | Slices | Side helper | Ships |
| --- | --- | --- | --- | --- | --- |
| `ask_user` | A question to your inbox, optionally blocking | ✓ | ✓ | – | v1 |
| `report_status` | A one-line status for the tree | ✓ | ✓ | – | v1 |
| `draw_map` | Create or update a map | ✓ | ✓ | ✓ | v1 |
| `log_decision` | Add a decision to a task's log | ✓ | ✓ | – | v1.1 |
| `create_task` | Create a task (lands in Unsorted) | ✓ | – | – | v1.1 |
| `claim` / `release` / `list_claims` | PR and task ownership | ✓ | ✓ | – | v1.1 |
| `when` | Rules like "rebase #1231 after #1230 merges" | ✓ | – | – | v2 |
| `notify` | Wake a stopped slice | ✓ | – | – | later |

Subagents inherit their parent's tools only if Claude Code passes MCP tools to them; background subagents were seen to lose the native task tools, so this is unconfirmed (see open questions).

## Daily review cycle

Dovetail closes each day with a record and opens the next with a review that cleans up and reprioritizes your tasks (Decision 17).

**End of day (17a–c).**

- Triggered when you quit Dovetail on a day with no record yet ("Close the day?": yes, no, later), or with `:eod`.
- Dovetail fills in the facts: tasks finished, started and moved; PRs opened and merged; what's blocked; decisions logged; cost per task.
- The coordinator adds a short narrative: what was learned, what's unresolved.
- You add your own notes, then save.
- Saved as markdown at `~/.local/share/dovetail/projects/<id>/journal/YYYY-MM-DD.md` (or a folder you set), and indexed for full-text search.

**Morning review (17c–d).**

- Opens automatically on the first open of a new day (one key to skip), or with `:morning`.
- The coordinator reads yesterday's record and your open tasks, then proposes concrete changes: close stale tasks, merge duplicates, split large ones, and a new priority order.
- A review buffer shows the changes like a diff: `y` / `n` per change, `A` to accept all. Dovetail applies what you accept.
- Running it in the coordinator means it starts the day knowing your priorities, at the cost of some of its context.

**Priority (17e).** Each task has two fields:

- **P-level** (P0–P3): how important it is. Changes rarely.
- **Bucket** (Now, Next, Later): when you'll do it. The morning review mostly moves tasks between buckets and orders them within Now.

The task tree groups by bucket (Now at the top, Later collapsed) and sorts by P-level within each. A mismatch, such as a P0 in Later or a P3 in Now, is flagged in the tree and raised by the morning review. Unfinished Now tasks stay in Now and are offered for review the next morning.

## Side-chat helper

During a grilling session, a second agent answers your quick questions without leaving the grill or adding to the coordinator's context (Decision 18).

- **What runs it (18a):** a separate headless session on a faster model (configurable). About 210 MB while running. Not a fork of the coordinator, and not a subagent, since subagents can't be chatted with headless and their answers would land in the coordinator's context.
- **Context (18b):** each time you open it, Dovetail passes the coordinator's latest message (usually the current grill question), the current task with its notes, and the project's paths. Quoting selected text comes with Visual mode in v2.
- **Tools (18c):** read-only plus exploration: read files, search the code, web search and fetch, read Dovetail's journal and conversation index, and spawn read-only exploration subagents. No edits, no shell writes. Runs from the main checkout. To be widened later.
- **Lifetime (18d):** one helper per grill. It starts automatically when the coordinator invokes a grilling skill (detected from the `Skill` tool call in the stream; the skill-name pattern is configurable, e.g. `grill*`) and is stopped when you close it or the next grill starts. `<leader>?` also starts one manually at any time.
- **Layout and keys (18e):** `<leader>?` toggles it into the bottom-right region in place of the inbox, so the grill question stays visible in the main pane; the inbox count stays in the status bar. `i` types to it. `<leader>y` copies the helper's last answer into the coordinator's input for you to edit and send.

## Task notes and decisions

Every task keeps your notes and a log of the decisions made on it, so nothing settled has to be remembered (Decision 19).

- **Who writes (19a):** you write notes. Agents log decisions with a `log_decision` MCP tool, prompted by the role prompts. The end-of-day narrative proposes decisions that weren't logged, for you to accept or reject.
- **Entry shape (19b):** what was decided and why (required), alternatives rejected (optional), who decided (an agent or you), when, and a link to the conversation turn it came from.
- **Storage (19c):** one markdown file per task in `~/.local/share/dovetail/projects/<id>/tasks/<task-id>.md`: your notes at the top, the decision log below. Indexed in `state.db`. Editing hands the file to nvim (a simple hand-off, not the deferred editor feature); Dovetail rereads it on save.
- **Where it shows (19d):** `Enter` on a task opens a task detail view (title, P-level, bucket, notes, decisions, linked PR, agents); `Enter` on an agent still opens its conversation. A new slice's brief includes its task's notes and decisions, and the side helper and morning review read them. The end-of-day record lists the day's decisions per task.

**Quick capture (Decision 21).** Getting a thought into the task list takes one line and no form.

- **Syntax:** a single line; anything that isn't a token is the title. `!p0`–`!p3` sets the P-level, `@now` / `@next` / `@later` the bucket, `#1231` links a PR. Example: `fix eval key leaking into logs !p1 @now #1231`.
- **Where:** `<leader>t` anywhere in Dovetail (Normal mode); `dovetail add "…"` from a shell, using the current folder's project or else the last one used. Not from inside conversations.
- **Keys:** `Enter` saves and returns you where you were; `Tab` expands into the full task detail view.
- **Unsorted:** a task with no bucket token goes to an **Unsorted** group at the top of the tree (shown with a count, hidden when empty), with no P-level unless one was typed. The morning review proposes a bucket and P-level for each, emptying the group.
- **Linking:** when you capture while focused on a task (or an agent working on one), the capture line shows "linked to: \<task>", on by default; `Ctrl-l` unlinks before saving. No link elsewhere. From the shell, `--to <task>` links. A linked task sits in Unsorted with a "follow-up of \<task>" marker and appears in the parent's detail view.
- **The coordinator** can create tasks with a `create_task` MCP tool (same fields). They go to Unsorted unless given a bucket, never straight to Now, and are marked coordinator-created until you've seen them. Slices and subagents can't create tasks; they tell the coordinator.

## Search

One search covers everything Dovetail knows, so you can find what was said without remembering where (Decision 20).

- **Indexed (20a):** notes, decisions, journal, messages between agents, inbox items, and conversation text (your messages and agents' replies, not tool output), in SQLite full-text search. History outlives Claude Code's 30-day transcript cleanup. This revises Decision 5b, which only referenced conversations.
- **Deep search (20a):** for anything not indexed, such as tool output, an on-demand search runs over Claude Code's transcript files (within their retention window).
- **Scope (20b):** the current project by default, with a toggle for all projects.
- **How (20c):** `<leader>/` opens the Telescope-style picker (the same component as the switcher), results grouped by type with a preview; `Enter` jumps to the exact turn, note or journal entry. Plain-language questions ("what did we decide about X?") go to the side helper, which can read the index.

## Visual maps

Agents explain code, structure, outcomes and architecture as maps you can navigate in the terminal, modeled on Floorplan's zoomable map but native (Decision 23).

- **One model (23a):** a map is structured data: nodes (parts, files, endpoints, concepts) with plain-English descriptions and file links, groups that nodes drill into, labeled connections ("calls", "reads from"), and guided flows (ordered paths through the map with a note per step). Stored per project, linked to a task, searchable, versioned.
- **Views:** an interactive text navigator everywhere, and a view-only image canvas where the terminal supports images (kitty's graphics protocol; kitty is the default terminal). Annotation happens in the side panel, or by exporting to a live Excalidraw or Floorplan.
- **Source (23b):** agents draw maps on request with a `draw_map` MCP tool; a Mermaid subset is accepted as import. Automatic code maps from scanning imports, routes and files come later.
- **Layout and rendering (23c):** [merman](https://docs.rs/crate/merman/0.9.0), a headless Rust implementation of Mermaid with compiled-in ELK layout and SVG, PNG, JPEG and PDF output, with no JavaScript runtime. Dovetail's map stays the source of truth and is converted to a Mermaid flowchart for drawing: groups become subgraphs; each node kind gets a shape and class (e.g. a cylinder for data, dashed borders for external, a thick border for outcomes); the selected node or flow step gets a highlight class and the image is re-rendered. The PNG is shown through kitty's graphics protocol (e.g. `ratatui-image`). Map code only talks to a `MapRenderer` trait, so elkjs in an embedded QuickJS engine can replace merman if needed.
- **Caveats:** merman is pre-1.0 with thin documentation; programmatic node positions (needed for `hjkl` between nodes) are undocumented and may have to be read from the SVG. Licenses: Apache-2.0 or MIT, plus EPL-2.0 notices for ELK.

**Node kinds (23d).** A fixed set, so every map reads the same way; anything else is a Concept. `?` shows a legend.

| Kind | For | Style |
| --- | --- | --- |
| Part | a service, package or module; drills in | double border, clay |
| File | a source file; opens it | thin border, off-white |
| Endpoint | a route, screen or CLI command | rounded, cyan |
| Data | a table, cache, queue, config | lidded / cylinder, amber |
| External | Stripe, SendGrid, other systems | dashed, grey |
| Actor | a user, admin or other role | person glyph ☺, rounded, warm pink |
| Outcome | a result: pass, fail or a value | heavy border, green or red |
| Logic | a few lines of plain pseudocode | code card, ƒ glyph, monospace, periwinkle; up to \~6 lines shown, the rest in the side panel |
| Concept | a free-form idea | no box, italic lavender |

Any node can also carry an optional pseudocode snippet, shown in the side panel when it's selected.

**Map types (23e).**

- **Structure maps:** what exists and how it connects. Flowchart style with nested groups, drawn with merman.
- **Flow maps:** what happens in order, including the trip back to the user (e.g. user → page → rules → database → back). Drawn as **horizontal swimlanes**: one lane per participant (tinted bands, labels on the left), time running left to right, steps as numbered boxes in the same styles, arrows in order. Lanes are nodes from the structure map, so you can jump between the two views. Dovetail lays swimlanes out itself on a grid (lane × step), so positions are known exactly; guided flows step through them with `n`/`p`. Long flows pan sideways.

**Map mode (23f).** Outside it, a map in a pane is a picture to look at. Inside it, the map fills the screen with its side panel, the status bar shows `MAP`, and the mouse works. Mouse capture is on only in map mode, so the terminal's normal text selection keeps working elsewhere.

- **Enter:** `Enter` on a map in the shelf or picker, `<leader>v` with a map in the main pane, or the expand ⤢ button in the map pane's header. **Leave:** `Esc` or the ✕ button.
- **Mouse:** click selects a node (side panel shows description, connections, pseudocode); double-click drills into a group or opens a file; hover highlights the node and its connections (redrawn only when the hovered node changes); scroll wheel zooms around the pointer; drag on empty space pans; right-click opens a menu (comment, ask the helper, send to the coordinator, open file); clicking a step number in a flow jumps to it.
- **Keys:** `hjkl` nearest node in that direction; `Tab` / `Shift-Tab` follow connections; `Enter` drill in or open; `Backspace` up a level; `n` / `p` flow steps; `+` / `-` / `0` zoom in, out, fit; `Ctrl-h/j/k/l` pan; `v` outline; `?` legend.
- **Hit-testing** uses node positions: exact for swimlanes; for structure maps, from merman (part of its spike).

**Guided flows (23g).** A walkthrough of a map, one step at a time.

- **Created by agents** as part of `draw_map`: a named, ordered path through the map's nodes with a short note per step. A swimlane map is itself a flow.
- **Starting:** the side panel lists a map's flows; click one or press `f`. On a swimlane, `n` starts at step 1.
- **Moving:** manual only, with `n` / `p` or by clicking a step number. The current step's node and incoming arrow light up, everything else dims, and the step's note shows in the side panel.
- **Ending:** after the last step, the side panel shows a summary of all the step notes. `Esc` leaves a flow early; the map stays open.
- **Asking:** `<leader>?` sends the current step to the side helper.

**Map updates (23h).** Every map has an id. When an agent calls `draw_map` with an existing id, the map is updated in place and the previous version is kept (for before-and-after comparison in v2). In map mode, right-click or `u` asks the coordinator to redraw the map from the current code. Comments stay on the nodes they were pinned to, as long as those nodes still exist. A new subject gets a new map.

**The draw\_map tool (23i).** Agents send structured data, not Mermaid, so nothing is lost:

- **map:** id (optional; reuse to update), title, type (`structure` or `swimlane`);
- **nodes:** id, kind (one of the nine), label, description, optional file link, optional pseudocode;
- **groups:** which nodes sit inside which (structure maps);
- **connections:** from, to, label;
- **flows:** name, ordered steps (node, plus the connection into it), a note per step;
- **lanes** (swimlanes): ordered participants; each step names its lane.

Dovetail validates the input (unknown kind, a connection to a missing node, a step in an unknown lane) and returns specific errors for the agent to fix. Structure maps are converted to Mermaid for merman; swimlanes are laid out by Dovetail. Mermaid import (v1.1) brings an existing diagram in as a structure map without descriptions.

| Version | What ships |
| --- | --- |
| v1 | `draw_map` tool; **image canvas** as the main map view (drawn as an image on the CPU, shown through kitty's graphics protocol): `hjkl` selects nodes, `Enter` drills into a group, guided flows highlight each step, keyboard pan and zoom; **outline** beside it for descriptions and connections, and as the fallback where images aren't supported; the map shelf |
| v1.1 | Side-panel comments pinned to nodes, sendable to the coordinator; "ask the map" via the side helper; Mermaid import; recent-work highlighting |
| v2 | Map versions compared before and after a PR; export to HTML and Excalidraw |
| Dropped | The box-and-arrow drawing in text characters (Nov 1): the rendering quality wasn't good enough. The outline is the fallback instead |
| Later | Automatic code maps (possibly using Floorplan's MIT-licensed scanner); opening a live Excalidraw or Floorplan session from a map |

## Context fill and fresh starts

Dovetail watches how full each long-lived agent's context is and replaces it with a fresh one, carrying a written handoff, before it degrades (Decision 22).

- **Display:** the coordinator's context fill in the status bar at all times (e.g. `ctx 62%`); each slice session's next to it in the tree. Subagents aren't shown. Normal up to 60%, amber 60–80%, red above 80%. Estimated from the token usage each turn reports in the stream, divided by the model's context window.
- **Coordinator above 80%:** a "Needs you" item offers a fresh start. If accepted, the coordinator writes a handoff note (current tasks, open threads, decisions in flight, next steps) into the journal; Dovetail starts a new coordinator seeded with it plus your tasks, notes and decisions. The old conversation stays searchable.
- **Every morning:** a fresh coordinator by default. At the end of the day, the outgoing coordinator writes a handoff section in the end-of-day record (open threads, what it was about to do, anything you asked it to remember). On the first open of a new day, the new coordinator is seeded with that section, your tasks, notes and decisions, and the morning review runs in it. "Keep yesterday's coordinator" is a one-key option on the review screen.
- **Slices above 80%:** automatic. After its current turn, the slice writes a handoff into its task's notes (done, next, gotchas); Dovetail closes it and starts a new session in the same worktree, seeded with the handoff and the task's notes and decisions. The old session is closed first, so the one-worktree-per-session rule holds. The coordinator is told and it shows in Activity; only a failed restart lands in "Needs you".
- **Subagents** need none of this: they're short-lived and start fresh.

## Session lifecycle and memory

The Rust app is the smallest process on the machine; Claude Code processes and the tools they run are the real memory cost, so the app runs quick work as subagents and caps each session.

**Measured budget (16 GB, Spike 1).** A headless `claude` session costs about 210 MB PSS (about 307 MB RSS) at rest; each background subagent inside it adds about 20–25 MB. Some versions have leaked to 12+ GB, so caps still matter. OS, desktop and browser take about 4–6 GB. The real constraint is now the tools agents run (typechecks, test suites), not the agents themselves.

**Lifecycle rules.**

- The coordinator is never stopped. Its memory cap is higher than other sessions', since its subagents run inside its process.
- If the coordinator crashes or hits its cap, Dovetail restarts it from its transcript and lists the subagent work that was in flight (task ids and output files from the stream), offering to restart each piece.
- Long-lived slice sessions run until you close them or the project.
- **Later:** idle reaping. A slice session idle past a timeout is stopped and resumed by session id, never while it holds a claim or has a blocking inbox item; `notify` wakes it; a live-session cap queues extra work.

**Guardrails (v1).**

- Each session runs in its own cgroup with a hard memory cap (a transient systemd scope with `MemoryMax`). The coordinator gets a higher cap than slice sessions. A leak kills one session and its subagents, which the app restarts from its transcript, not the machine.
- Per-session memory in the tree (read from `/proc`) and a total in the status bar.

**Known costs of reaping (later).** A reaped slice is unreachable by native `SendMessage` until woken; resuming adds latency and may miss the prompt cache; background jobs a slice started die with it.

**GPU and terminal.** Irrelevant to Claude Code and the TUI. Update (Nov 1): the GPU now works and kitty is the default terminal; its graphics protocol enables the map image view (Decision 23).

## Roadmap

v1 replaces the tmux setup for one project with real status, an inbox and memory guardrails; each later phase adds one capability on the same core.

**v1 — usable daily driver**

- [ ] `SessionDriver` with one implementation backed by a community SDK crate
- [ ] Launch sessions headless with injected settings, hooks and role prompts
- [ ] Registry + per-project `state.db`; resume a project after restart
- [ ] Buffers in the fixed layout: tree, coordinator, slice view, inbox, activity, message timeline
- [ ] Normal / Insert / Command modes; hardcoded keymap through the action registry; picker (`nucleo`)
- [ ] Two-tier inbox with inline approvals
- [ ] Visual maps, stage 1: `draw_map` tool, image canvas via kitty with node selection, flows and pan/zoom, outline beside it and as fallback, map shelf (Decision 23)
- [ ] Context fill per agent: coordinator in the status bar, slices in the tree, amber and red thresholds (Decision 22)
- [ ] Quick capture: `<leader>t`, `dovetail add`, inline tokens, Unsorted group, switchable link to the focused task; coordinator `create_task` (Decision 21)
- [ ] Readability: 120-column reading column, spacing rules, reading theme with colored emphasis, folded tool activity, turn navigation, solid-background toggle
- [ ] Sessions start in auto mode; per-agent mode switch (`<leader>sm`) and badge; approvals over the SDK control protocol, answered concurrently
- [ ] One worktree per slice via the `WorktreeCreate` / `WorktreeRemove` hooks, with symlinked files and a setup command
- [ ] Task tracker: your tasks with status, dependencies, notes and linked PR, in the left pane
- [ ] Agent tree under each task: sessions plus their subagents, via hooks and transcripts
- [ ] Under each task: background subagents as progress nodes, plus native to-do items from sessions and foreground subagents (one list id per task)
- [ ] Take-over escape hatch
- [ ] cgroup memory cap per session; memory in tree and status bar
- [ ] `ask_user` and `report_status` via `dovetail mcp` (Decision 8b)
- [ ] Role prompts for coordinator and slices, appended at launch (Decision 8c)

**v1.1 — enforced coordination and leaner memory**

- [ ] `claim`, `release`, `list_claims`
- [ ] TOML keymap overrides; which-key popup
- [ ] Visual maps, stage 2: node comments, ask the map, Mermaid import, recent-work highlighting (Decision 23)
- [ ] Fresh starts with handoffs: coordinator on request above 80% and every morning; slices automatically above 80% (Decision 22)
- [ ] Task notes and decision log: per-task markdown files, `log_decision` tool, task detail view (Decision 19)
- [ ] Search: full-text index including conversation text, `<leader>/` picker, deep transcript search (Decision 20)
- [ ] Side-chat helper for grilling: auto-start on a grilling skill, `<leader>?` region swap, `<leader>y` to send its answer to the coordinator (Decision 18)
- [ ] Daily review cycle: end-of-day record (facts, coordinator narrative, your notes), morning review buffer, journal search (Decision 17)
- [ ] Readability extras: syntax highlighting in code blocks, configurable theme colors (including a high-contrast variant), focus mode
- [ ] Dovetail plugin with skills for procedures (rebase after merge, PR handoff), triggered by name from Dovetail's events (Decision 8c)

**v2 — automation and layout**

- [ ] `when(event, then)` rules (e.g. rebase #1231 after #1230 merges)
- [ ] Vim-style window splits
- [ ] Visual maps, stage 3: map versions, HTML and Excalidraw export (Decision 23)
- [ ] Visual mode

**Later**

- [ ] Split core into a daemon; TUI as a client; remote attach over SSH
- [ ] Background-project attention badge
- [ ] Lua configuration
- [ ] Session switcher also lists your other local Claude sessions, read-only, messaged over the native socket (Decision 14a)
- [ ] Editor and file tree (Decision 13): start with a hand-off to nvim in the focused agent's worktree, then a changed-files and diff view, then embedded nvim if needed
- [ ] Idle reaping of slice sessions with wake-on-message, `notify` and a live-session cap (deferred after Spike 1)
- [ ] Dovetail subtask tool, so background subagents get checklists too (first: spike whether background subagents can call MCP tools)

## Open questions and spikes

Five short spikes and two decisions stand between this design and the first real code.

- [x] **Auto mode headless:** done Oct 29. Works with `--permission-mode auto`; 0 of 10 ordinary actions needed approval. See Inbox.
- [ ] **Inbound messages in the stream:** how a received `SendMessage` appears in a headless session's event stream. Needed for the message timeline.
- [ ] **Driver crate (4a):** pick which community Rust SDK crate to use by testing two against a two-session test; check multi-session support, interrupt and in-process permission callbacks.
- [ ] **cgroup launch:** confirm `systemd-run --user --scope -p MemoryMax=…` works on the target machine without root.
- [ ] **Terms of use:** confirm running Claude Code headless this way fits your plan and auth setup.
- [ ] **Decision 8c test:** run one slice with only tool descriptions and one with the role prompt; check which calls `ask_user` when stuck.
- [ ] **MCP tools in background subagents:** check whether a background subagent can call Dovetail's MCP tools (e.g. `ask_user`, `draw_map`). Can ride along with another spike.
- [ ] **merman (23c):** render a 30-node map with two levels of nested subgraphs and one style per node kind to PNG; measure time and memory; find how to get each node's position (API or SVG node groups); check how it looks in kitty. Fallback if it fails: elkjs in QuickJS.
- [x] **Hook-created worktrees (done Oct 29, passed):** with a `WorktreeCreate` hook passed in `--settings`, confirm that `claude -p --worktree` uses the hook's path, that isolation checks still block edits to the main checkout (including from subagents), that a headless resume returns to the worktree, and whether subagent worktrees also go through the hook.
- [x] **Subagent memory:** done Oct 29. Subagents run in the parent's process; 56% less peak memory than separate sessions. See Agents and tasks.
- [x] **Subagent events headless:** done Oct 29. Fully visible via `parent_tool_use_id` and `system` task events.
- [x] **Subagents and the native task list:** done Oct 29. Foreground subagents share the parent's list; background subagents can't use task tools at all.
- [x] **Native task files:** done Oct 29. Format recorded under Agents and tasks.
- [x] **Name:** Dovetail.

**Sources**

- [Message your other Claude Code sessions](https://code.claude.com/docs/en/cross-session-messaging) (official docs)
- [Cross-Session Messaging in Claude Code](https://blakecrosley.com/blog/claude-code-cross-session-messaging), Blake Crosley, updated Aug 25, 2026
- [Claude Code agent teams](https://scalably.io/blog/claude-code-agent-teams), Scalably
- [claude-agent-sdk-rs](https://docs.rs/crate/claude-agent-sdk-rs/latest) and [rusty\_claw](https://docs.rs/crate/rusty_claw/latest) on docs.rs
- [deltachat-claude-code](https://pypi.org/project/deltachat-claude-code) (per-session memory and idle reaping)
- [\~700 MB RSS per CLI instance](https://claudeissues.com/issue/61425-bug-excessive-memory-usage-700-mb-rss-per-cli-instance) and [14.6 GiB RSS OOM kill](https://claudeissues.com/issue/86712-claude-code-cli-reached-14-6-gib-rss-and-triggered-a-host-wide-oom-kill) issue reports

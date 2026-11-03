# Dovetail

> Status: **design phase**. No code yet.

A keyboard-driven terminal UI, written in Rust, for running many Claude Code agents on one feature at the same time: a **coordinator** you work with directly, plus the sessions and subagents doing the rest of the work.

It replaces a hand-built setup of tmux, fzf and window juggling with one app that tracks your tasks, knows what every agent is doing, and brings anything that needs you into a single inbox.

## Why

Running a coordinator and several worker sessions in tmux works, but:

- You can't tell which session is working, idle, or blocked on you without visiting each pane.
- Questions and approvals get buried at the end of long turns in panes you aren't looking at.
- Coordination lives in chat. Two sessions can both believe they own the same PR, and nothing stops them.
- Opening, naming and tracking sessions is manual.
- At that speed it's hard to keep in mind what's in flight, what's next, and what you've already finished.
- Subagents that sessions spawn on their own are invisible.

Dovetail keeps the parts that work (Claude Code itself, its native cross-session messaging, a neovim-style workflow) and adds the missing layer: task tracking, real status, one inbox, and enforced coordination.

## What it does

- **Tracks your tasks.** Each project has a task list: status, dependencies, notes and a linked PR. Tasks can exist before any agent touches them (ideas, follow-ups). This is the main view, also available as a kanban board.
- **Shows every agent under its task.** Full sessions and the subagents they spawn appear in one tree. Background subagents show their progress, summary and cost; sessions and foreground subagents also show their to-do items from Claude Code's native task list.
- **Uses the right kind of agent.** Quick, self-contained work runs as subagents, chosen by Claude. Slices that own a PR across days run as full sessions.
- **Runs sessions headless.** The app drives each session over Claude Code's structured JSON protocol, so it sees messages, tool calls and permission requests as events instead of screen text.
- **Coordinator first.** The coordinator gets the main pane and is always running, and a focus mode shows nothing else. Other agents come to you when they need something.
- **Two-tier inbox.** "Needs you" holds permission requests, questions, finished turns and errors. A separate Activity feed holds pushes, CI and messages between sessions.
- **Quiet by default.** Sessions run in Claude Code's auto mode, where a classifier approves routine actions, so few requests reach you. Any agent can be switched to a stricter mode with one key.
- **Inline approvals.** Answer the requests that do come up from the inbox with a single key, with the request's context shown alongside.
- **Worktree view.** Every session's worktree in one list: branch, sync, changes, PR, setup and cleanup.
- **Quick capture.** `<leader>t` (or `dovetail add` from a shell) turns one line like `fix eval key leak !p1 @now #1231` into a task. Anything without a bucket lands in Unsorted, which the morning review sorts.
- **Visual maps.** Ask an agent to explain code, structure, an outcome or an architecture, and it draws a map: parts, connections and plain-English descriptions, as a structure diagram or a swimlane of a business flow, with step-by-step guided flows. Maps show as a proper diagram right in kitty, navigable by keyboard and mouse in a dedicated map mode, with an outline beside it that also serves as the fallback in terminals without image support. Each map is saved with its task.
- **Built for reading.** Conversations render in a 120-column column with real spacing, markdown styling plus a color for each kind of emphasis on soft off-white text, folded tool chatter, turn-by-turn navigation, and an optional solid background behind the text for transparent terminals.
- **Fresh context, never lost work.** Dovetail shows how full each agent's context is. A full coordinator or slice is replaced by a fresh session seeded with a written handoff, and the coordinator starts fresh every morning from the previous day's handoff.
- **Daily review cycle.** At the end of the day Dovetail writes a record of what happened (facts, a short narrative from the coordinator, and your notes). Next morning the coordinator reviews it with your open tasks and proposes cleanup and new priorities, which you accept or reject one by one.
- **Remembers for you.** Each task keeps your notes and a log of decisions, written by you and by the agents as they decide things. One search (`<leader>/`) covers notes, decisions, the journal, inbox, messages and conversation text, and keeps history after Claude Code clears old transcripts.
- **A helper for grilling sessions.** When the coordinator starts grilling you, a read-only side chat opens next to it for quick lookups, so the grill keeps going and the coordinator's context stays clean. One key sends the helper's answer into your reply.
- **Message timeline.** See what sessions say to each other over Claude Code's native messaging.
- **Enforced coordination.** A small set of tools the sessions call, served by the app over MCP: `ask_user`, `report_status`, then `claim` / `release` for PR ownership and rules like "rebase #1231 after #1230 merges."
- **Take over any session.** Drop into native `claude --resume` for hands-on work, then hand it back.
- **Project persistence.** Close everything at the end of the day; reopen tomorrow and your tasks, agents, ownership and conversations are all back.

## Interaction model

Modal, neovim-style, no mouse required (the mouse works in map mode).

| Keys | Action |
| --- | --- |
| `h` `j` `k` `l` | Move between panes |
| `i` / `Esc` | Chat with the focused session (coordinator by default) / back to Normal |
| `:` | Command mode |
| `Space` | Leader. Mnemonic groups with a which-key popup |
| `<leader><leader>` | Picker over sessions, inbox and messages |
| `<leader>c` / `<leader>n` | Jump to coordinator / inbox |
| `<leader>ss` | Telescope-style agent switcher with a live conversation preview |
| `<leader><Tab>` | Toggle between the current agent and the previous one |
| `<leader>s…` | Sessions: new slice, take over, interrupt, cycle permission mode |
| `<leader>b` / `<leader>g` / `<leader>z` | Board / worktree view / focus mode |
| `<leader>t` | Quick capture a task |
| `<leader>/` | Search everything (v1.1) |
| `<leader>v` | Enter map mode (full-screen map, mouse on) |
| `<leader>?` / `<leader>y` | Toggle the grilling side chat / send its last answer into your reply (v1.1) |
| Inbox: `y` `A` `d` `D` `r` `x` | Allow once, always allow (Shift-a), deny, deny with reason, reply, dismiss |

## Design principles

- **Low memory.** Built for a 16 GB machine. The app itself is small; the real cost is Claude Code processes, so quick work runs as subagents inside their parent's process (measured at about 20–25 MB each, against about 210 MB per separate session), each session runs under a hard memory cap, and memory use is shown per session.
- **One worktree per slice.** Each slice session gets its own git worktree, so no two agents edit the same files; Claude Code's isolation checks block edits back into the main checkout. The coordinator works from the main checkout. Files like `.env` are symlinked into each new worktree, and an optional setup command runs before the slice starts.
- **Nothing in your repos.** All state, including slice worktrees, lives under `~/.local/share/dovetail/`, and role prompts and per-project setup under `~/.config/dovetail/`. Hooks, settings and role prompts are passed to Claude Code at launch, never written into the project.
- **One project at a time.** Focus on the feature in front of you.
- **Built to grow.** The core sits behind an internal API, so it can later run as a daemon on a remote dev box with the TUI attached over SSH.

## Architecture at a glance

```
TUI (buffers, modes, keymap)
   │ actions
Core ── SQLite (registry.db + state.db per project)
   │ NDJSON over stdio            ▲ unix socket
Claude Code sessions ──MCP tools──► MCP bridge (dovetail mcp, one per session)
   └── native messaging between sessions (per-session unix sockets)
```

- **Language:** Rust (ratatui, tokio, rusqlite, nucleo)
- **Storage:** SQLite; conversations stay in Claude Code's own transcripts and are referenced, not copied
- **Session driver:** a community Rust Claude Agent SDK crate behind a `SessionDriver` trait, so it can be swapped for a hand-written layer or the official SDK later
- **Maps:** [merman](https://docs.rs/crate/merman) (headless Rust Mermaid with ELK layout) renders structure maps to images, Dovetail lays out swimlanes itself, and both show through kitty's graphics protocol behind a `MapRenderer` trait

## Requirements (planned)

- Linux (cgroup memory caps use `systemd-run --user`)
- Claude Code v2.1.224 or later (native cross-session messaging)
- A terminal emulator; kitty (or Ghostty or WezTerm) for map images. Other terminals work, with maps falling back to an outline

## Roadmap

- **v1:** task tracker with a kanban board, P-levels, Now / Next / Later buckets and quick capture; agent tree with subagent progress; to-do items from Claude's native task list; headless sessions in auto mode with a per-agent mode switch; one worktree per slice with a worktree view; fixed layout and focus mode; Telescope-style session switcher; inbox with inline approvals; take-over; project persistence; memory caps and context fill per agent; readability theme; visual maps (`draw_map`, image canvas in kitty, outline fallback, map mode, guided flows); `ask_user` and `report_status`; role prompts for coordinator and slices
- **v1.1:** daily review cycle, fresh starts with handoffs, side-chat helper for grilling, task notes and decision log, search across everything, map comments and Mermaid import, PR ownership tools, configurable keymap and theme, Dovetail plugin with skills for procedures
- **v2:** automation rules, map versions and export, window splits, Visual mode
- **Later:** daemon and remote attach, Lua configuration, idle reaping of slice sessions, checklists for background subagents, a neovim-based editor and file tree for reviewing agent work, automatic code maps
- **Backlog (after v1):** cost and budget caps, long-running jobs view, PR panel, task templates, summary cards, desktop notifications, read aloud, PR stack view, quiet hours, broadcast to slices

## Status and open items

All design decisions are made; the full design and decision log live in the design doc. The order of work is in `docs/implementation-plan.md`.

- Done: spikes S0–S4 (terms of use, driver, messaging, MCP, cgroups); results in `docs/spikes/`
- Done: M1. The workspace, the core/UI protocol and a seeded mock core. Try `cargo run -p dovetail -- --mock --dump`; the same `--seed` prints the same stream
- Next: M2, the TUI shell running on the mock
- Later spikes: merman map rendering (S5, before maps); whether a short role prompt makes slices use `ask_user` reliably (S6, during M9)

## Repository layout

- `crates/dovetail-api/` — plain-data protocol between core and UI: `Snapshot`, `Event`, `Command` and the `apply()` reducer
- `crates/dovetail-mock/` — `MockCore`, a seeded stand-in for the core that speaks the same protocol
- `crates/dovetail-core/` — sessions, storage, worktrees and MCP tool handlers (from M5)
- `crates/dovetail-driver/` — runs headless Claude Code sessions (from M6); recorded stream fixtures in `tests/fixtures/`
- `crates/dovetail-tui/` — the terminal UI (from M2); depends only on `dovetail-api`
- `crates/dovetail/` — the `dovetail` binary
- `spikes/` — throwaway spike programs, outside the workspace
- `docs/implementation-plan.md` — milestones and the order of work
- `docs/ui-session-brief.md` — detailed UI spec
- `docs/design.md` — snapshot of the design doc (decision log, architecture, spike results); the live version is the Dovetail design doc

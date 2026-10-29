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
- **Inline approvals.** Answer permission requests from the inbox with a single key, with the request's context shown alongside.
- **Worktree view.** Every session's worktree in one list: branch, sync, changes, PR, setup and cleanup.
- **Message timeline.** See what sessions say to each other over Claude Code's native messaging.
- **Enforced coordination.** A small set of tools the sessions call, served by the app over MCP: `ask_user`, `report_status`, then `claim` / `release` for PR ownership and rules like "rebase #1231 after #1230 merges."
- **Take over any session.** Drop into native `claude --resume` for hands-on work, then hand it back.
- **Project persistence.** Close everything at the end of the day; reopen tomorrow and your tasks, agents, ownership and conversations are all back.

## Interaction model

Modal, neovim-style, no mouse required.

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
| `<leader>s…` | Sessions: new slice, take over, interrupt |
| `<leader>b` / `<leader>g` / `<leader>z` | Board / worktree view / focus mode |
| Inbox: `y` `a` `d` `D` `r` `x` | Allow once, always allow, deny, deny with reason, reply, dismiss |

## Design principles

- **Low memory.** Built for a 16 GB machine with no working GPU. The app itself is small; the real cost is Claude Code processes, so quick work runs as subagents inside their parent's process (measured at about 20–25 MB each, against about 210 MB per separate session), each session runs under a hard memory cap, and memory use is shown per session.
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

## Requirements (planned)

- Linux (cgroup memory caps use `systemd-run --user`)
- Claude Code v2.1.224 or later (native cross-session messaging)
- A terminal emulator; a CPU-rendered one is recommended on machines without a working GPU

## Roadmap

- **v1:** task tracker with a kanban board, agent tree with subagent progress, to-do items from Claude's native task list, headless sessions, one worktree per slice with a worktree view, fixed layout, focus mode, inbox with inline approvals, take-over, project persistence, memory caps, `ask_user` and `report_status`, role prompts for coordinator and slices
- **v1.1:** PR ownership tools, configurable keymap, Dovetail plugin with skills for procedures
- **v2:** automation rules, window splits, Visual mode
- **Later:** daemon and remote attach, Lua configuration, idle reaping of slice sessions, checklists for background subagents, a neovim-based editor and file tree for reviewing agent work

## Status and open items

All design decisions are made; the full design and decision log live in the design doc. Before the first real code:

- Spikes: auto mode in headless sessions, how inbound messages appear in the event stream, picking which community SDK crate to use, cgroup launch on the target machine, terms of use
- Done: agent and task spikes. Subagents run inside their parent's process (56% less memory than separate sessions) and are fully visible in the headless stream; native task lists are readable and shared, but background subagents can't write to them
- Done: worktrees created by a `WorktreeCreate` hook outside the repo get Claude Code's isolation checks and headless resume, with nothing written into the repo; Dovetail handles their cleanup itself
- Test: whether a short role prompt makes slices use `ask_user` reliably

## Repository layout

- `crates/dovetail/` — the `dovetail` binary (placeholder until the UI shell lands)
- `docs/ui-session-brief.md` — brief for the UI-shell-against-mock-data session

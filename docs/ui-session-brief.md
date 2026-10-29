We're starting the UI for **Dovetail**, a keyboard-driven Rust TUI I'm building to run many Claude Code agents on one feature: a coordinator session I work with directly, plus long-lived "slice" sessions and the subagents they spawn. The design phase is finished. This session builds the **UI shell against mock data**. No Claude Code integration yet.

Before writing any code, read this brief, then propose the crate layout, module plan and milestone order, and wait for my OK.

## What Dovetail is for

I run several Claude Code agents in parallel and lose track of them. Dovetail's job, in order of importance:

1. **Track my tasks** for the current project: what's in flight, what's next, what's waiting on me, what's done. Tasks are the main thing on screen.
2. **Show every agent under the task it works on**: full sessions, and the subagents they spawn.
3. **Bring anything that needs me into one inbox**: permission requests, questions, finished turns, errors.
4. **Let me talk to any agent**, mainly the coordinator, without the mouse.

## Hard constraints

- **Rust**, `ratatui` + `crossterm`. Fuzzy picker with `nucleo`.
- **Low memory.** The target machine has 16 GB RAM and **no working GPU**. Keep the app lean: no heavy dependencies, and redraw only when something changed.
- **Any terminal, including basic CPU-rendered ones.** Don't rely on the kitty keyboard protocol. Shift+Enter is a nice extra only, never required.
- **Nothing written into user repos.** Irrelevant this session, but don't add anything that assumes it.
- **Linux** is the primary target.

## Architecture rules for the UI

- **Core / UI split.** The UI talks only to a core API: it reads state snapshots and receives events over a channel. The real core will drive Claude Code later, and may later move into a separate daemon process, so the UI must never reach past that boundary. For this session, write a `MockCore` behind the same interface that generates believable activity on a timer: agents changing status, streaming replies, new inbox items, subagents starting and finishing.
- **One action registry.** Every keybinding and every `:` command resolves to a single `Action` enum. The keymap is hardcoded in **one table** for now; it becomes a TOML file later, and Lua after that, so keep it data-like.
- **Buffers.** Every view is a buffer type (task tree, conversation, inbox, activity feed, message timeline, board, worktrees). Regions of the layout display buffers, and which buffer a region shows can be swapped (via the picker). v1 uses a **fixed layout**; vim-style splits come later on top of the same abstraction, so don't hardcode a buffer to a region.
- **Movement keys are global; action keys are buffer-local.** For example, `y` approves in the inbox but can mean something else elsewhere.

## Data model (mock it, keep it close to this)

- **Project**: name, tasks.
- **Task**: id, title, status (`todo`, `in_progress`, `needs_you`, `done`, `archived`), dependencies, notes, linked PR (e.g. `#1231`), and the agents attached to it. A task can have no agents yet (an idea or follow-up).
- **Agent**:
  - id, kind (`Session` or `Subagent { background: bool }`), role (`Coordinator`, `Slice`, `Subagent`), parent agent, task.
  - **Worktree:** each slice session has exactly one worktree (path and branch), and no worktree has two sessions. The coordinator works in the main checkout. Subagents share their parent's worktree. Show the branch next to slice sessions in the tree.
  - status (`working`, `idle`, `needs_you`, `done`, `error`), a one-line status text, memory in MB (sessions only), cost so far.
  - **Background subagents** have a description, progress and a final summary, but **no to-do items**.
  - **Sessions and foreground subagents** can have to-do items.
- **TodoItem**: id, subject, status (`pending`, `in_progress`, `completed`), blocked-by ids, created-by agent.
- **InboxItem**:
  - kind: `Permission { tool, input }`, `Question { text, blocking }`, `FinishedTurn { preview }`, `Error { text }`.
  - The agent it came from, and a timestamp.
  - "Needs you" items only. Activity (PR pushed, CI status, messages between agents) is a separate feed.
- **Conversation entries** per agent: my message, assistant text (streams in), and a compact tool-call line (e.g. `Edit src/routes.rs`).

## Layout (fixed, v1)

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

- **Left, top:** the task tree. Project, then tasks, then each task's agents (with the branch under each slice), then each agent's to-do items. A background subagent shows as one node with a progress indicator.
- **Left, bottom:** the inbox ("Needs you").
- **Right:** a conversation buffer, full height. It shows the **coordinator** by default and switches when I open another agent.
- **Status bar:** mode, project, task count, live agent count, "needs you" count, total memory.
- Glyphs (adjust if something reads better): `○` todo, `◐` working, `⚠` needs you, `✓` done, `✗` error. `◆` session, `◇` subagent.

The mocks for every screen are on the layout canvas: https://claude.ai/artifact/D6yQ2Vejy8VEbQeEJWYtQL

## Modes and keys

| Mode | Purpose | Enter / leave |
| --- | --- | --- |
| Normal | Navigate and act | Default; `Esc` returns here |
| Insert | Type to the focused agent | `i` |
| Command | Ex-style commands | `:` |

Visual mode comes later; leave room for it.

**Global (Normal mode):**

| Keys | Action |
| --- | --- |
| `h` `j` `k` `l` | Move focus between regions (`j`/`k` move within lists when a list is focused, so decide and tell me how you'll resolve this) |
| `i` | Insert mode for the focused agent. From the tree or inbox it targets the **coordinator**, unless an agent is selected in the tree |
| `Esc` | Back to Normal |
| `:` | Command mode (`:q` quits; add a few mock commands, e.g. `:spawn <name>`) |
| `/` `n` `N` | Search in the current buffer |
| `Space` | Leader key (Normal mode only) |
| `<leader><leader>` | Picker over tasks, agents, inbox items |
| `<leader>c` | Jump to the coordinator |
| `<leader><Tab>` | Toggle between the current agent and the previous one (vim's alternate buffer) |
| `<leader>n` | Jump to the inbox |
| `<leader>m` | Message timeline buffer |
| `<leader>a` | Activity feed buffer |
| `<leader>b` | Board: tasks as a kanban in the main region |
| `<leader>g` | Worktree view |
| `<leader>z` | Focus mode: coordinator only |
| `<leader>ss` / `sn` / `st` / `sx` | Pick agent / new slice / take over (stub) / interrupt |
| `<leader>pp` / `pr` | Switch project / register project (stubs) |

**Session switcher (`<leader>ss`), Telescope-style.** I mostly chat with the coordinator and visit other agents briefly, so this needs to be fast.

```
┌ Switch ────────────────────────────┐┌ Preview ─────────────────────────────┐
│ › eval                        3/7  ││ ◐ Helix Business release eval run     │
├ Agents ────────────────────────────┤│   slice · dovetail/eval · 412 MB      │
│ ⚠ Business home raise judge   2m   ││                                       │
│▌◐ Helix Business release eval  now ││ Claude                                │
│ ◐ └ rebase onto #1229  sub     1m  ││ The second full eval (business@4) is  │
│ ✓ Triage group A           18m     ││ running on 723f41738 with the same    │
│ ○ plan_product triage       1h     ││ settings as before...                 │
└────────────────────────────────────┘└───────────────────────────────────────┘
```

- A floating overlay: search prompt and results list on the left, preview on the right.
- Rows: status glyph, agent name or task title, worktree branch, time since last activity. Subagents indented under their parent.
- Empty search: agents that need me first, then most recently active. Typing switches to fuzzy ranking with `nucleo`.
- Preview: the agent's last few turns, rendered like the main pane. For a background subagent, its description, progress and summary.
- Keys: type to filter; `Ctrl-j` / `Ctrl-k` or arrows to move; `Enter` opens the agent in the main pane; `Esc` closes.
- Lists Dovetail's agents only.
- `<leader><leader>` is the same overlay over everything (tasks, agents, inbox items), so build one picker component and reuse it.

**Other views.**

- **Board (`<leader>b`):** tasks as a kanban in the main region, columns Todo, In progress, Needs you, Done. Cards show agents, to-do progress, cost and memory. `j`/`k` card, `H`/`L` column (plain `h`/`l` stay focus movement), `<` `>` move a card to the previous or next status.
- **Worktrees (`<leader>g`):** one row per session's worktree: branch, ahead/behind, changes, PR, setup state; the coordinator on the main checkout; subagents Claude isolated, under their parent; archived worktrees waiting for removal. The selected row shows path, setup, base, head, uncommitted files and commits ahead. `Enter` open the session, `e` open in nvim, `c` copy path, `X` remove (asks first if work is unpushed).
- **Focus mode (`<leader>z`):** only the coordinator conversation and the input; the status bar keeps the "needs you" count.

**Buffer-local:**
- **Inbox:** `y` allow once, `a` always allow, `d` deny, `D` deny with a reason (opens input), `r` reply to a question, `x` dismiss a finished turn or error, `Enter` open the agent. `n`/`N` stay global search. The selected item shows a context preview: the tool, its full input, and the agent's last message.
- **Task tree:** `Enter` open the agent's conversation, `o` expand or collapse, `a` add task, `e` edit title, `s` cycle status, `dd` archive.

**Chat input (Insert mode):** plain multiline input. `Enter` sends. `Ctrl-j` and `Alt-Enter` insert a newline. `Ctrl-w` deletes a word, `Ctrl-u` the line. No vim editing inside the input.

**Interrupt** is a Normal-mode action (`<leader>sx`), because `Esc` belongs to mode switching.

A which-key popup (press Space, wait, see the bindings) is planned for later. Build it only if it falls out naturally.

## Milestones for this session (adjust order if you see a better one)

1. Workspace skeleton, event loop, mode state machine, status bar.
2. Fixed layout (tree over inbox, conversation on the right) and focus movement.
3. Task tree buffer on mock data, with expand/collapse, glyphs, branch lines and live updates from `MockCore`.
4. Inbox buffer, with inline approval keys and the preview.
5. Conversation buffer, with Insert mode sending to the mock and a streamed mock reply.
6. Command mode, the session switcher and the picker.
7. Board, worktree view and focus mode.

**Out of scope:** real Claude Code, SQLite, MCP, settings files, splits, Visual mode, TOML keymaps.

## Testing

- Unit tests for the mode state machine and keymap resolution.
- Rendering snapshot tests with ratatui's `TestBackend` (`insta` is fine).
- Run `cargo clippy` and `cargo test` before telling me a milestone is done.

## Reference

- The full design (decisions, architecture, spike results) is in the Dovetail design doc: https://claude.ai/artifact/Lbsz6jYHREqaPmdWpstK8N
- The implementation plan for this session: https://claude.ai/artifact/WYWjqZJsuADXjYPMepwoho
- The project README is in the Dovetail project as `claude/README.md`.

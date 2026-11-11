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
- **Low memory.** The target machine has 16 GB RAM. Keep the app lean: no heavy dependencies, and redraw only when something changed.
- **Terminal:** kitty is my default, but Dovetail must work in any terminal. Don't rely on the kitty keyboard protocol; Shift+Enter is a nice extra only, never required. (kitty's graphics protocol will be used later for map images, not now.)
- **Nothing written into user repos.** Irrelevant this session, but don't add anything that assumes it.
- **Linux** is the primary target.

## Architecture rules for the UI

- **Core / UI split.** The UI talks only to a core API: it reads state snapshots and receives events over a channel. The real core will drive Claude Code later, and may later move into a separate daemon process, so the UI must never reach past that boundary. For this session, write a `MockCore` behind the same interface that generates believable activity on a timer: agents changing status, streaming replies, new inbox items, subagents starting and finishing.
- **One action registry.** Every keybinding and every `:` command resolves to a single `Action` enum. The keymap is hardcoded in **one table** for now; it becomes a TOML file later, and Lua after that, so keep it data-like.
- **Buffers.** Every view is a buffer type (task tree, conversation, inbox, activity feed, message timeline, board, worktrees). Regions of the layout display buffers, and which buffer a region shows can be swapped (via the picker). v1 uses a **fixed layout**; vim-style splits come later on top of the same abstraction, so don't hardcode a buffer to a region.
- **Movement keys are global; action keys are buffer-local.** For example, `y` approves in the inbox but can mean something else elsewhere.

## Data model (mock it, keep it close to this)

- **Project**: name, tasks.
- **Task**: id, title, status (`todo`, `in_progress`, `done`, `archived`; "needs you" is derived from its agents' inbox items, never stored, per plan B4), dependencies, notes, linked PR (e.g. `#1231`), and the agents attached to it. A task can have no agents yet (an idea or follow-up).
  - **Priority, two fields:** a P-level (`P0`–`P3`, importance) and a bucket (`Now`, `Next`, `Later`, timing), with an order within `Now`.
  - **Tree grouping:** group tasks by bucket (`Now` at the top and expanded, `Later` collapsed with a count), sorted by P-level within each bucket. Show the P-level next to each task. Flag a mismatch (a `P0` in `Later`, a `P3` in `Now`) with a subtle marker.
  - **Unsorted group:** tasks with no bucket yet sit in an `Unsorted` group shown above `Now` (with a count, hidden when empty). They may have no P-level. A task can carry a "follow-up of <task>" marker and a "coordinator-created" marker.
- **Agent**:
  - id, kind (`Coordinator`, `Slice` or `Subagent { background: bool }`; coordinators and slices are sessions), parent agent, task.
  - **Worktree:** each slice session has exactly one worktree (path and branch), and no worktree has two sessions. The coordinator works in the main checkout. Subagents share their parent's worktree. Show the branch next to slice sessions in the tree.
  - status (`working`, `idle`, `needs_you`, `done`, `error`), a one-line status text, memory in MB (sessions only), cost so far.
  - permission mode (`auto`, `acceptEdits`, `manual`); `auto` is the default.
  - `context_pct` for sessions (how full the context is); let `MockCore` raise it over time.
  - **Background subagents** have a description, progress and a final summary, but **no to-do items**.
  - **Sessions and foreground subagents** can have to-do items.
- **TodoItem**: id, subject, status (`pending`, `in_progress`, `completed`), blocked-by ids, created-by agent.
- **InboxItem**:
  - kind: `Permission { tool, input }`, `Question { text }` (never blocking, ADR 0007), `FinishedTurn { preview }`, `Error { text }`.
  - The agent it came from, and a timestamp.
  - "Needs you" items only. Activity (PR pushed, CI status, messages between agents) is a separate feed.
- **Conversation entries** per agent: my message, assistant text (streams in), and a compact tool-call line (e.g. `Edit src/routes.rs`).

## Layout (fixed, v1)

The sketch shows the tree over the inbox on the left; the app mirrors it (conversation left, ADR 0006) and stacks the inbox over the tree on the right, 40/60 (ADR 0008).

```
┌ tasks ──────────────────┬ coordinator ──────────────┐
│ ◐ ◆ coordinator         │ ── You · 10:14 ────────── │
│ ▾ #1231 business surface│ What's blocking slice-7?  │
│   ◐ ◆ slice-4           │                           │
│     ⎇ dovetail/slice-4  │ ── coordinator · 10:14 ── │
│     └ ✓ ◇ rebase        │ It needs FS_EVAL_VIRTUAL… │
│ ▾ ⚠ get_decision 500    │                           │
├ inbox (2) ──────────────┤                           │
│ ▸ ⚠ slice-7  Bash       │                           │
│   ? slice-4  Question   │ › i to chat               │
├─────────────────────────┴───────────────────────────┤
│ NORMAL  business · 3 tasks · 2 agents · 2 ⚠ · 2.1G  │
└─────────────────────────────────────────────────────┘
```

- **Right, top (40 %):** the inbox ("Needs you"). One line per item: kind glyph, source, a short title cut with `…` to fit, and the time. The title is what a permission request is for, or the first sentence of a question, finished turn or error. The full item opens with `Enter` (see the inbox keys below; task #5, 2026-11-09).
- **Right, bottom (60 %):** the task tree (ADR 0008). Project, then tasks, then each task's agents (with the branch under each slice), then each agent's to-do items. A background subagent shows as one node with a progress indicator.
- **Left:** a conversation buffer, full height. It shows the **coordinator** by default and switches when I open another agent.
- **Header row:** one row above the regions naming what is open: `worktree <path> on <branch>` inside a slice's worktree, `folder <path>` otherwise (home shown as `~`). While the open folder is not a Dovetail project yet (ADR 0005), its right end carries the warning `⚠ not a Dovetail project yet: :project add keeps it` (cut to `⚠ not a project yet` when narrow) in the needs-you colour; that warning shows here, not in the status bar. The right end also carries the usage indicator (decision #25, task #16): `maveron · opus-5-5 · 5h 34% (resets 14:20) · week 18% (resets Thu)`. It leads with the Claude profile of the project in view (ADR 0009), omitted when nothing names its account. The model is the one of the conversation in view, shown short (`claude-opus-5-5` → `opus-5-5`, a trailing date dropped). The limits are that profile's account's, from the latest `rate_limit_event` of a session running under it; a reset within 24 hours shows as `hh:mm`, a later one as the weekday. Each figure is dim below 70 %, orange from 70 % and red from 90 %. A window whose reset time has passed shows `—` (`5h —`) until the next report. As the row narrows the reset times go first, then the profile and the model, then the week, and the 5-hour figure goes last; next to the warning, the usage shortens before the warning does.
- **Status bar:** mode, project, task count, live agent count, "needs you" count, total memory, and the coordinator's context fill (e.g. `ctx 62%`).
- **Context fill colors** (status bar for the coordinator, next to each slice session in the tree; not for subagents): normal up to 60%, amber 60–80%, red above 80%.
- **Task tree grouping:** the tree groups tasks into `Unsorted` (when not empty), `Now`, `Next`, `Later` as described in the data model; the coordinator row stays at the top.
- Glyphs (adjust if something reads better): `○` todo, `◐` working, `⚠` needs you, `✓` done, `✗` error. `◆` session, `◇` subagent.

The mocks for every screen are on the layout canvas: https://claude.ai/artifact/D6yQ2Vejy8VEbQeEJWYtQL

## Readability (important: I have poor eyesight)

Dovetail draws all conversation text itself, so tune it for reading:

- **Width:** cap the reading column at **120 columns** (configurable), centered in wider panes. Code blocks and tables may use the full pane.
- **Spacing:** a blank row between paragraphs, between list items, and around code blocks and headings. Never render a dense block of text.
- **Emphasis by style AND color:** render markdown properly and keep the styling (bold stays bold, italic stays italic, code stays code), but also give each its own color:
  - body text: soft off-white, **never pure white**;
  - bold: a warm accent color;
  - italic: a second, distinct color;
  - inline code and file paths: a third color;
  - headings and speaker labels: the accent color;
  - tool summaries and older turns: dimmed.
  Put all colors in one theme struct so they become configurable later. Use `pulldown-cmark` for parsing; no syntax highlighting in code blocks yet.
- **Background:** my terminal is transparent and I want to keep that, so don't paint backgrounds by default. Add a toggle that gives the reading column a solid dark background, and turn it on automatically in focus mode.
- **Message headers:** each message opens with a blank row and a header rule as wide as the reading column, `── <name> · hh:mm ───`. The name is bold in the speaker's colour, the rule dim. `You` for my messages; agents are named exactly as in the tree (e.g. `coordinator`, `slice-4`). Tool summaries, notices and later reply chunks stay under the agent's one header, and the streaming `◐` sits in it (task #6, 2026-11-09).
- **Fold the noise:** tool activity in a turn collapses into one summary line (e.g. `Edited 3 files · ran tests ✓ · 42 s`), expandable. Hide thinking. Truncate long output with a "show more".
- **Reading keys** (buffer-local in a conversation, Normal mode): `j`/`k` scroll, `gg`/`G` top and bottom, `]]`/`[[` next and previous turn, `zc`/`zo` fold and unfold a turn.

Give the mock replies realistic markdown (bold leads, lists, code, file paths) so the theme can be judged.

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
| `:eod` | Close the day: the coordinator writes its handoff, Dovetail adds the facts, into `journal/YYYY-MM-DD.md` (ADR 0012) |
| `:q` on a day with no journal | "Close the day? y: handoff and journal, then quit · n: quit now · Esc: later". `y` runs `:eod` and quits once the file is written; `n` quits as `:q` does (asking if a turn runs); `Esc` cancels. Asked only in a registered project (ADR 0012) |
| `Ctrl-C` `Ctrl-C` | Quit in two steps, in every mode: the first press shows "Press Ctrl-C again to exit" for 2 s, and any other key cancels it. The second press quits like `:q`: if a turn is running it asks "A turn is running. Quit anyway?" (`y` quits). Interrupting a turn stays on `<leader>sx`. SIGINT also quits like `:q` (a second SIGINT answers yes); SIGTERM and SIGHUP quit at once: SIGHUP (a closed terminal) is treated like SIGTERM, since nothing is left to answer a prompt (decision #23; task #13, 2026-11-09) |
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
| `<leader>z` | Focus mode: the conversation alone |
| `<leader>sq` / `:ask` | Open a side chat (ADR 0011) in the side panel |
| `<leader>?` | Show or hide the side panel (side chats) |
| `<leader>ss` / `sn` / `st` / `sx` | Pick agent / new slice / take over (stub) / interrupt |
| `<leader>pp` / `pr` | Switch project / register project (stubs) |
| `<leader>sm` | Cycle the focused agent's permission mode: `auto` → `acceptEdits` → `manual`. Show a small badge in the tree for any agent not in `auto` |
| `<leader>t` | Quick capture: a one-line overlay. Anything that isn't a token is the title; `!p0`–`!p3` sets P-level, `@now`/`@next`/`@later` the bucket, `#1231` a PR. No bucket → `Unsorted`. When a task (or an agent on one) is focused, show "linked to: <task>" under the line, on by default, `Ctrl-l` to unlink. `Enter` saves and returns to where I was; `Tab` opens the full task detail view |

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
- Preview: the agent's last few turns, rendered like the main pane. For a background subagent, its description, progress and summary. A session whose subagents used tokens adds `N tokens in subagents` to its facts line, summed from their task events (M11, task #17).
- Keys: type to filter; `Ctrl-j` / `Ctrl-k` or arrows to move; `Enter` opens the agent in the main pane; `Esc` closes.
- Lists Dovetail's agents only.
- `<leader><leader>` is the same overlay over everything (tasks, agents, inbox items), so build one picker component and reuse it.

**Other views.**

- **Board (`<leader>b`, `:board`):** tasks as a kanban in the main region, columns Todo, In progress, Done. A task that needs you shows `⚠` and an amber border on its card, in its status column; done wins (B4). Cards show the id, the title cut to fit, the P-level and the names of live slices. `j`/`k` card, `Tab`/`Shift-Tab` next or previous column, wrapping (plain `h`/`l` stay focus movement), `H`/`L` move a card to the previous or next status (the cursor follows once the core confirms it; a refused change leaves it in place), `<` `>` move a card up or down its column (a column lists Now, Next, Later, then Unsorted, each in the task order `J`/`K` set in the tree; a card moves past neighbours in its own bucket only), `s` cycles the status, `a` quick capture, `Enter` the task detail (`Esc` there returns to the board), `Esc` or `<leader>b` back. Task #22, 2026-11-09.
- **Worktrees (`<leader>g`):** one row per session's worktree: branch, ahead/behind, changes, PR, setup state; the coordinator on the main checkout; subagents Claude isolated, under their parent; archived worktrees waiting for removal. The selected row shows path, setup, base, head, uncommitted files and commits ahead. `Enter` open the session, `e` open in nvim, `c` copy path, `X` remove (asks first if work is unpushed).
- **Side panel (`<leader>?`):** side chats in the side column, in place of the task tree; the inbox stays on top and the coordinator's conversation stays full height on the left (Layout B). It shows one side chat at a time, with its own chat bar: `i` or `Ctrl-j` types to it, `Ctrl-k` leaves the chat bar, `Tab`/`Shift-Tab` switch side chats (the title shows `2/3 ⇥`), `dd` closes the one shown (its session stops). `<leader>sq` and `:ask` open the new side chat here. `h` goes to the conversation and `l` back, `Ctrl-k` up to the inbox and `Ctrl-j` down again. `<leader>?` again gives the region back to the tree. Focus mode hides it; it is still open afterwards. Side chats cannot change files: Edit, Write and NotebookEdit are off, and Bash runs only read-only commands (`git log`/`show`/`diff`/`status`/`blame`, `rg`, `grep`, `ls`, `cat`, `head`, `tail`, `wc`, `find` without `-exec`/`-delete`, `cargo tree`, `cargo metadata`); anything else is denied without asking. Their mode does not cycle, and the tree shows them `read-only`. Decided 2026-11-11.
- **Focus mode (`<leader>z`, `:focus`):** only the conversation and the input, full width, background unchanged (ADR 0013); the status bar keeps the "needs you" count. It keeps the agent in view (the coordinator, a slice or a side chat). `<leader>z` again, or any move to the side column (`l`, `<leader>n`, `Ctrl-j`/`Ctrl-k` outside the conversation), leaves it. As built 2026-11-10.

**Buffer-local:**
- **Inbox:** `y` allow once, `A` (Shift-a) always allow, `d` deny, `D` deny with a reason (opens input), `r` reply to a question, `x` dismiss a finished turn or error, `Enter` open the item in full. `n`/`N` stay global search. Rows stay one line; the selected item shows no preview.
- **Inbox item detail:** `Enter` on an inbox item opens it in an `InboxDetail` buffer in the main region: the whole text in the reading column (questions and briefs as markdown; for a permission request, the tool and its full input). The same `y A d D r x` keys work there, `⏎` opens the agent, and `Esc` goes back to the inbox. Exception: `Enter` on a finished turn opens the agent's conversation, which is that item's full text, and opening it clears the item (ADR 0001). Task #5, 2026-11-09.
- **Task detail, research (R3, keys changed 2026-11-11):** a task with research gets a Research section (report, rounds with their briefs, findings, sources; a queued round shows as `queued (2nd in line)`). `j`/`k` scroll line by line again; `Tab`/`Shift-Tab` jump between the research rows (wrapping); `Enter` reads the selected row in the reading column (citation markers `[3]`, `[F3]`, `[S2]` coloured), `Esc` back to the detail. `e` on a selected research row opens that file in `$EDITOR` (the core's editor hand-off); with no row selected `e` is the notes action; in the reading view `e` edits the open file. `:research` lists the tasks that have research.
- **Research commands (2026-11-11):** `:round <task> <brief>` requests a round; while another round is open it is queued (its `brief.md` is written at once, the Activity feed says "round queued (2nd in line)") and opens, in request order, when the open one closes (`research_write_report` or `:round close`), when the coordinator is told "Round N for #T is now open; send its subagents." `:round drop <task> [n]` drops a queued round (the latest one by default). `:research kind <decision|question>` sets what the focused or selected task is for (a decision is held to the decision template); the coordinator can also set it with `research_start_round`'s `kind`. `dd` on a task with an open or queued round asks "Task #T has an open research round. Close it and archive? y/n": `y` closes the round (findings kept, noted in Activity), drops the task's queued rounds, archives, and the next queued round opens.
- **Unfiled findings (2026-11-11):** a coordinator subagent that used `WebFetch` or `WebSearch` while no round was open is stored with no task and raises a Needs-you item "Unfiled finding: '<label>' (N sources)". `y` or `Enter` opens a picker over the tasks and moves the finding and its source files to that task's research folder as a quick finding; `d` deletes it and its files.
- **Task tree:** `Enter` on an agent opens its conversation; `Enter` on a task opens a task detail view (title, P-level, bucket, notes, decisions, linked PR, agents; mock the notes and a couple of decision entries). `o` expand or collapse, `a` add task, `e` edit title, `s` cycle status, `b` cycle bucket, `p` cycle P-level, `J`/`K` reorder within `Now`, `dd` archive.

**Chat input (Insert mode):** plain multiline input that wraps at word boundaries (only a word longer than the line is broken; task #4, 2026-11-09). `Enter` sends. `Shift-Enter` (where the terminal reports it) and `Alt-Enter` insert a newline. `Ctrl-k` leaves the bar for the conversation above, keeping the draft and its cursor; `Ctrl-j` or `i` comes back; `PageUp`/`PageDown` scroll the conversation while typing (ADR 0010). **Completion:** `/` at the start of the bar lists the session's slash commands and skills with their descriptions; `@` lists paths (a plain `@foo` fuzzy-matches the session's files that git does not ignore; `@~/`, `@/`, `@./`, `@../` browse one directory at a time, dotfiles once a dot is typed). `Up`/`Down` or `Ctrl-p`/`Ctrl-n` move, `Tab` accepts, `Enter` accepts unless the row is already typed (then it sends), `Esc` closes the popup. The text goes to Claude Code as typed; it reads `/name` and `@path` itself (docs/spikes/slash-and-at.md). `:skills` (or `:commands`) opens the same list in the picker, with each description in the preview. `Ctrl-w` deletes a word, `Ctrl-u` the line. No vim editing inside the input. `Ctrl-V` asks the core to read the system clipboard (`wl-paste`, else `xclip`; the UI itself runs no process and writes no file, task #13): an image (PNG, JPEG, GIF or WebP, up to 5 MB, at most 10 per message) is attached and shown as a chip such as `[image 1 · 412 KB]` above the input, and the sent message shows the same chips; otherwise the clipboard's text is pasted at the cursor. `Backspace` on an empty input removes the last chip. Chat bars only (task #1, 2026-11-09).

**Interrupt** is a Normal-mode action (`<leader>sx`), because `Esc` belongs to mode switching.

A which-key popup (press Space, wait, see the bindings) is planned for later. Build it only if it falls out naturally.

## Milestones for this session (adjust order if you see a better one)

1. Workspace skeleton, event loop, mode state machine, status bar.
2. Fixed layout (conversation on the left, inbox over tree on the right) and focus movement.
3. Task tree buffer on mock data, with expand/collapse, glyphs, branch lines and live updates from `MockCore`.
4. Inbox buffer, with inline approval keys and the item detail.
5. Conversation buffer, with Insert mode sending to the mock and a streamed mock reply.
6. Command mode, the session switcher and the picker.
7. Board, worktree view and focus mode.
8. Quick capture, the Unsorted / Now / Next / Later tree grouping, the permission-mode switch and badge, and context fill in the status bar and tree.

Apply the readability rules from the start (milestone 5 onward), not as a final pass.

**Out of scope:** real Claude Code, SQLite, MCP, settings files, splits, Visual mode, TOML keymaps.

## Testing

- Unit tests for the mode state machine and keymap resolution.
- Rendering snapshot tests with ratatui's `TestBackend` (`insta` is fine).
- Run `cargo clippy` and `cargo test` before telling me a milestone is done.

## Reference

- The full design (decisions, architecture, spike results) is in the Dovetail design doc: https://claude.ai/artifact/Lbsz6jYHREqaPmdWpstK8N
- The implementation plan for this session: https://claude.ai/artifact/WYWjqZJsuADXjYPMepwoho
- The project README is in the Dovetail project as `claude/README.md`.

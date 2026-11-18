# Dovetail architecture guide

How Dovetail works inside, for someone who built it through agents and never read the code. Part 1 is the code as it is today. Part 2 sets each layer against the v2 design (`docs/design-v2.md`) and says how much changes.

Every claim links to the file that proves it. Read the keywords first; the rest uses them without explaining again.

## Keywords

| Word | What it is | Where |
| --- | --- | --- |
| **Core** | The one part that knows and changes everything: sessions, tasks, inbox, database. It runs on its own thread. | [`service.rs`](../crates/dovetail-core/src/service.rs) |
| **Core loop** | One `loop` inside the core. Everything that changes state passes through it, one item at a time. | `Core::run` in [`service.rs`](../crates/dovetail-core/src/service.rs) |
| **Input** | One item the loop handles: a command from the UI, an event from a session, a tool call, a finished background job. | `enum Input` in [`service.rs`](../crates/dovetail-core/src/service.rs) |
| **Snapshot** | The whole state the UI may see, as one plain value: tasks, agents, inbox, chat tails. | `struct Snapshot` in [`model.rs`](../crates/dovetail-api/src/model.rs) |
| **Event** | One change to the snapshot, sent by the core, with a number (`seq`). | `enum Event` in [`protocol.rs`](../crates/dovetail-api/src/protocol.rs) |
| **`seq`** | The event's number. Each is one more than the last, so the UI can tell when it missed one. | [`apply.rs`](../crates/dovetail-api/src/apply.rs) |
| **Command** | One request from the UI to the core ("send this prompt", "allow this"). | `enum Command` in [`protocol.rs`](../crates/dovetail-api/src/protocol.rs) |
| **`apply()`** | The one function that folds an event into a snapshot. The UI's copy changes no other way. | [`apply.rs`](../crates/dovetail-api/src/apply.rs) |
| **CoreLink** | The UI's end of the wire: a channel to send commands, a channel to receive the snapshot and events. | `struct CoreLink` in [`protocol.rs`](../crates/dovetail-api/src/protocol.rs) |
| **Session** | One running `claude` process that Dovetail drives without a terminal. Coordinator and slices are sessions. | [`session/mod.rs`](../crates/dovetail-core/src/session/mod.rs) |
| **Agent** | A row in the tree: a session, or a subagent running inside one. | `struct Agent` in [`model.rs`](../crates/dovetail-api/src/model.rs) |
| **Driver** | The only code that starts `claude` and reads what it prints. The core sees typed events, never JSON. | [`dovetail-driver`](../crates/dovetail-driver/src/lib.rs) |
| **Stream** | What a session prints: one JSON object per line (NDJSON). | [`parse.rs`](../crates/dovetail-driver/src/parse.rs) |
| **Control request** | A line where `claude` asks permission and waits for a line back. | `PermissionRequest` in [`event.rs`](../crates/dovetail-driver/src/event.rs) |
| **MCP bridge** | A tiny program (`dovetail mcp`) that Claude Code starts beside each session. It passes Dovetail's tool calls to the core. | [`mcp.rs`](../crates/dovetail/src/mcp.rs) |
| **Socket** | A local connection file, one per open project. The bridge and `dovetail add` reach the core through it. | [`socket.rs`](../crates/dovetail-core/src/socket.rs) |
| **DB thread** | The one thread that touches SQLite. Everyone else sends it a request and waits for the answer. | [`db.rs`](../crates/dovetail-core/src/db.rs) |
| **Migration** | A numbered SQL file that moves a database one version forward. Never edited after release, never run backwards. | [`db/migrations/`](../crates/dovetail-core/src/db/migrations/) |
| **`registry.db`** | One small database for the whole machine: the list of projects. | `~/.local/share/dovetail/registry.db` |
| **`state.db`** | One database per project: tasks, agents, chat, inbox, research. | `~/.local/share/dovetail/projects/<id>/state.db` |
| **Worktree** | A second checkout of a repo on its own branch. Each slice gets one, outside the repo. | [`worktree.rs`](../crates/dovetail-core/src/worktree.rs) |
| **Hook** | A command Claude Code runs at a set moment. Dovetail's hook creates the worktree. | [`hook.rs`](../crates/dovetail/src/hook.rs) |
| **Recipe** | A per-project TOML file: how to set up a worktree, extra environment, which Claude profile. | [`recipe.rs`](../crates/dovetail-core/src/recipe.rs) |
| **Inbox** | "Needs you": permission requests, questions, finished turns, errors. | [`inbox.rs`](../crates/dovetail-core/src/inbox.rs) |
| **Waiter** | What is held open behind an inbox item, such as the session's pending permission request. Lives in memory only. | `enum Waiter` in [`inbox.rs`](../crates/dovetail-core/src/inbox.rs) |

---

# Part 1: today

## 1. The parts

```
 ┌──────────────────────────── one process: `dovetail` ────────────────────────────┐
 │                                                                                 │
 │   dovetail-tui  (terminal UI)                                                   │
 │        │  ▲                                                                     │
 │ Command│  │ Snapshot, then Event(seq)          = CoreLink (two channels)        │
 │        ▼  │                                                                     │
 │   dovetail-core  (one thread, one loop)                                         │
 │        │           │                  │                    ▲                    │
 │        │ requests  │ start / send     │ git, gh,           │ tool calls,        │
 │        ▼           ▼                  ▼ worktrees          │ `dovetail add`     │
 │   DB thread    dovetail-driver    blocking tasks        unix socket             │
 │   (SQLite)         │                                       ▲                    │
 └────────────────────┼───────────────────────────────────────┼────────────────────┘
                      │ stdin / stdout, one JSON per line     │
                      ▼                                       │
                `claude` process  ── starts ──►  `dovetail mcp` (the bridge)
                (one per session)                 one per session
```

| Crate | Job | Knows about |
| --- | --- | --- |
| [`dovetail-api`](../crates/dovetail-api/src/lib.rs) | The vocabulary: `Snapshot`, `Event`, `Command`, `apply()`, the tool list, the socket messages. Data only, no behaviour beyond `apply`. | nothing else |
| [`dovetail-driver`](../crates/dovetail-driver/src/lib.rs) | Starts `claude`, turns each printed line into a typed `DriverEvent`, writes lines back. A fake driver replays recorded streams for tests. | api |
| [`dovetail-core`](../crates/dovetail-core/src/lib.rs) | The loop, the database, sessions, inbox, slices, worktrees, tool handlers. | api, driver |
| [`dovetail-mock`](../crates/dovetail-mock/src/lib.rs) | A pretend core with scripted data. Hands out the same `CoreLink`, so the UI cannot tell. | api |
| [`dovetail-tui`](../crates/dovetail-tui/src/lib.rs) | The terminal UI. Keeps its own snapshot, draws it, sends commands. | api only |
| [`dovetail-maps`](../crates/dovetail-maps/src/lib.rs) | Checks and draws structure maps. | api |
| [`dovetail`](../crates/dovetail/src/main.rs) | The binary. With no subcommand it wires core and TUI together. Subcommands: `mcp` (bridge), `hook` (worktree), `add`, `login`, `upgrade`. | all |

Two rules hold the design together:

1. **The UI and the core never share memory.** They exchange plain values over `CoreLink`. That is why a mock core, and later the page, can stand in without the other side changing.
2. **One loop changes state.** Slow work (git, the clipboard, a build) runs elsewhere and comes back to the loop as an `Input`. So two things never change the same state at once, and there are no locks to get wrong.

**Example.** The TUI never reads `state.db`. To show tasks it reads `snapshot.tasks`; to add one it sends `Command::CreateTask`; the new task arrives as `Event::TaskUpserted`. `scripts/check.sh` fails the build if the TUI writes a file or starts a process.

## 2. One chat message, end to end

You type "Start the web form" to the coordinator and press Enter.

```
 TUI                         core loop                      driver            claude
  │ Command::Prompt ─────────►│                               │                 │
  │                           │ prompt()                      │                 │
  │                           │  ensure_live()  (start or resume if not running)│
  │                           │  session.send(text) ─────────►│ write a line ──►│
  │                           │  add_entry_with(User)         │                 │
  │                           │   ├─► DB thread: upsert_entry │                 │
  │◄─ Event(41) ConversationAppended                          │                 │
  │◄─ Event(42) AgentUpserted (status: working)               │                 │
  │                           │                               │◄── text chunks ─│
  │                           │◄── Input::Driver(TextDelta) ──│                 │
  │                           │ queue_delta()  (held ≤ 33 ms) │                 │
  │◄─ Event(43) AssistantDelta "Will do. "                    │                 │
  │◄─ Event(44) AssistantDelta "The API slice…"               │                 │
  │                           │◄── Input::Driver(TurnResult) ─│◄── result ──────│
  │                           │ on_result()                   │                 │
  │◄─ Event(45) InboxUpserted (finished turn)                 │                 │
  │◄─ Event(46) AgentUpserted (status: idle)                  │                 │
```

Step by step, with the real names:

1. **TUI.** The chat bar pushes `Command::Prompt { agent, text, attachments }` ([`app.rs`](../crates/dovetail-tui/src/app.rs)); the event loop sends it on `CoreLink.commands` ([`event_loop.rs`](../crates/dovetail-tui/src/event_loop.rs)).
2. **Into the loop.** A small thread forwards it as `Input::Command` (`spawn` in [`service.rs`](../crates/dovetail-core/src/service.rs)). `Core::run` picks it up and calls `Core::command`, which calls `Core::prompt` ([`session/mod.rs`](../crates/dovetail-core/src/session/mod.rs)).
3. **To Claude.** `prompt` calls `send_input`, which calls `ensure_live` (starts or resumes the `claude` process if none runs), then `session.send(...)`. The driver writes one JSON line to the process's stdin ([`claude.rs`](../crates/dovetail-driver/src/claude.rs)).
4. **Recorded and shown.** `add_entry_with` gives the message an id, stores it through the DB thread (`Db::upsert_entry`), and emits `Event::ConversationAppended`. `emit` adds one to `seq` and sends it.
5. **Reply streams back.** `claude` prints lines. The driver's reader parses each (`map_line` in [`parse.rs`](../crates/dovetail-driver/src/parse.rs)) into a `DriverEvent`; a task forwards it as `Input::Driver`. `Core::on_driver` handles it. Text chunks go to `queue_delta`, which holds them up to 33 ms so the UI gets at most about 30 updates a second, then sends `Event::AssistantDelta`.
6. **Turn ends.** `on_result` raises a "finished turn" inbox item and sets the agent idle. The finished reply is stored once, complete.
7. **TUI.** For each message, `apply(&mut snapshot, seq, event)` ([`apply.rs`](../crates/dovetail-api/src/apply.rs)) folds it in, and the screen is redrawn only because something changed.

**What `seq` is for.** Events 41, 42, 43 arrive. If 44 is lost and 45 arrives, `apply` returns `Gap` and changes nothing; the UI sends `Command::Resync` and gets a fresh snapshot. The UI can never drift silently.

## 3. A tool call and a permission request

These look alike to the user but take different roads.

### A Dovetail tool call (`core_report_status`, `tasks_create`, `project_start_slice`…)

```
 claude ──MCP (stdin/stdout)──► dovetail mcp ──unix socket──► core loop ──► handler
        ◄──── tool result ─────              ◄── response ───           (Core::tool)
```

1. At launch the core writes an `mcp.json` for the session that names `dovetail mcp` and passes it the agent's id and the socket path ([`session/launch.rs`](../crates/dovetail-core/src/session/launch.rs)). Nothing is written into your repo.
2. When Claude calls `mcp__dovetail__core_report_status`, the bridge checks the arguments and sends `SocketRequestBody::ToolCall { agent, call }` over the socket ([`mcp.rs`](../crates/dovetail/src/mcp.rs), [`socket.rs`](../crates/dovetail-api/src/socket.rs)).
3. The socket server turns it into `Input::Tool`; the loop calls `Core::tool` ([`tools.rs`](../crates/dovetail-core/src/tools.rs)), which checks the agent's role and tool suites, does the work, and replies.

**Example.** A slice calls `core_report_status("writing tests")`. `Core::tool` sets `agent.status_line`, emits `Event::AgentUpserted`, and answers "Status set." The tree shows the new line.

Tool calls never wait for you. `core_ask_user` puts a question in the inbox and returns at once; your answer arrives later as a normal message (ADR 0007).

### A permission request (Claude wants to run `git push`)

```
 claude ── control request ──► driver ── PermissionRequest ──► core: permission_request()
   (waits)                                                        raise() → inbox item + Waiter::Control
                                                                         │
                                          you press y                    ▼
 claude ◄── control response ── driver ◄── answer_control() ◄── Command::AnswerPermission
```

1. Claude Code prints a control request and waits. The driver emits `DriverEvent::PermissionRequest`; it never waits itself, so the stream keeps flowing and other requests can arrive meanwhile (stream rule 3).
2. `Core::permission_request` ([`inbox.rs`](../crates/dovetail-core/src/inbox.rs)) calls `raise`, which stores an `inbox_items` row, emits `Event::InboxUpserted`, and keeps a `Waiter::Control` in memory holding the request's id.
3. You answer. The UI sends `Command::AnswerPermission { item, answer }`. `Core::answer_permission` resolves the item (`Event::InboxResolved`) and calls `session.answer_control(...)`; the driver writes the answer line and Claude carries on.

**Example.** The coordinator runs `gh pr merge 42`. The launch settings force that command to ask, so an item "Bash: gh pr merge 42" appears in Needs you. `y` lets the merge run; `d` denies it and Claude is told why.

**Why the Waiter matters.** The pending request lives inside the `claude` process. If Dovetail restarts, that process is gone, so the request cannot be answered any more. The row in the database is marked "expired at restart" and you get an Activity note instead (section 5).

## 4. The database

Two files. All access goes through the DB thread: callers use typed methods such as `Db::create_task` and never write SQL themselves ([`db.rs`](../crates/dovetail-core/src/db.rs)).

```
 ~/.local/share/dovetail/
 ├── registry.db                      one for the machine
 │     projects (id, name, kind, repo_path, git_common_dir, last_opened_ms)
 │
 └── projects/<id>/
       ├── state.db                   one per project
       ├── worktrees/<slice>/         slice checkouts
       ├── journal/                   end-of-day files
       └── research/                  readable copies of findings
```

### What hangs off what in `state.db`

This is the shape up to version 16. Version 17, added with this guide, puts `efforts` and `criteria` above `tasks`; see Part 2, "Work model".

```
 tasks ─┬─◄ task_deps            (this task waits for that one)
        ├─◄ task_related         (pairs)
        ├─◄ decisions ─◄ decision_evidence ─► findings / sources
        ├─◄ agents               (agents.task_id, may be empty)
        ├─◄ slice_queue          (slice starts waiting for a free place)
        ├─◄ maps ─◄ map_versions
        ├─◄ research_rounds ─◄ findings ─◄ finding_sources ─► sources
        └─◄ reports

 agents ─┬─◄ conversation_entries   (agent_id; chat text only, never tool output)
         ├─◄ inbox_items            (agent_id; the item itself is JSON in `payload`)
         ├─◄ activity, messages     (feeds)
         └─◄ todo_items             (who created each native to-do)

 standalone: coordinator_notes (messages held for a coordinator that is not running),
             kept_worktrees, unfiled_findings ─◄ unfiled_finding_sources
```

`A ─◄ B` means "B rows point at an A row".

| Table | One row is | Written by |
| --- | --- | --- |
| `tasks` | One of your tasks: title, status, bucket, notes, PR number | [`db/tasks.rs`](../crates/dovetail-core/src/db/tasks.rs) |
| `agents` | A coordinator, slice or subagent, with its Claude session id, transcript path, worktree, cost, status | [`db/sessions.rs`](../crates/dovetail-core/src/db/sessions.rs) |
| `conversation_entries` | One chat line of one agent | [`db/sessions.rs`](../crates/dovetail-core/src/db/sessions.rs) |
| `inbox_items` | One Needs-you item; `resolved_ms` is set when it is answered | [`db/sessions.rs`](../crates/dovetail-core/src/db/sessions.rs) |
| `decisions` | One decision-log entry; never edited, only superseded | [`db/tasks.rs`](../crates/dovetail-core/src/db/tasks.rs) |
| `research_rounds`, `findings`, `sources`, `reports` | Research: a round of subagents, what each reported, what each read, the written report | [`db/research.rs`](../crates/dovetail-core/src/db/research.rs) |
| `maps`, `map_versions` | A drawn map and every version of it | [`db/maps.rs`](../crates/dovetail-core/src/db/maps.rs) |

**Example: one slice's footprint.** Starting slice `api-guest` on task 52 writes: one `agents` row (`id = slice-api-guest`, `task_id = 52`, `branch = dovetail/api-guest`), one `conversation_entries` row for its brief, one `activity` row "slice api-guest started", and sets `tasks.status` of 52 to `in_progress`.

**What is not in the database.** The full transcript (Claude Code keeps it; Dovetail stores its path), tool output (only one summary line per turn), memory use, and the Waiters.

**Migrations.** Each database stores its version number. On open, `migrate` runs every file after that number, each in its own transaction ([`db.rs`](../crates/dovetail-core/src/db.rs)). A database from a newer Dovetail is refused, never touched. A test fails the build if the files are not numbered 0001…N in order.

## 5. Start, restart and crash

### Start

```
 dovetail (in a repo)
   1. read config.toml, open registry.db, find the project for this folder   tui.rs: run_real
   2. dovetail_core::spawn → open state.db (run migrations), bind the socket service.rs: spawn
   3. core thread: Core::load  → agents, chat tails, queue, open inbox items service.rs: load
   4. send Hello + Snapshot    → the UI can draw                              send_snapshot
   5. reconcile                → deal with what a previous run left behind    restore.rs
   6. coordinator resumes; slices resume when first touched (ensure_live)
```

The first frame never waits for git, `systemd` or Claude: those checks run after the snapshot.

### Quit, then open again

Quitting stops every session; Dovetail waits up to 10 s for them to exit ([`shutdown.rs`](../crates/dovetail-core/src/shutdown.rs)). State stays on disk exactly as it was. On the next open:

| Left behind | What `reconcile` does ([`restore.rs`](../crates/dovetail-core/src/restore.rs)) |
| --- | --- |
| An agent stored as "working" | Its turn was cut off. One inbox item with its last prompt. It is **never continued automatically**. |
| A pending permission request | Expired (the process that asked is gone). Activity note. |
| A pending question | Stays; your answer is sent as a normal message. |
| A slice whose worktree folder is gone | Archived, with an Activity note. |
| A worktree no agent owns | One inbox item: `y` removes it, `d` keeps it for good. Nothing is deleted without `y`. |
| Notes for a coordinator that was not running | Delivered when it next starts. |

Sessions resume with Claude Code's own resume: Dovetail chose the session id at first start and stored it with the transcript path, so the conversation continues where it stopped.

**Example.** You quit while slice `web-form` is mid-turn and a `git push` approval is waiting. Next open: "web-form's turn was cut off by a restart — Last prompt: …" in Needs you, and "web-form: Bash git push: request expired at restart" in Activity. You send web-form a message; `ensure_live` resumes its session and it carries on.

### Crash

A crash of Dovetail is the same as a quit, because nothing important lives only in memory: every chat line, inbox item and agent change is written as it happens. What is lost is small and known: text still streaming (the entry is stored when complete) and the Waiters.

A crash of one `claude` process is handled live: `on_exit` ([`session/mod.rs`](../crates/dovetail-core/src/session/mod.rs)) raises an Error item with the last lines of its error output. If the kernel killed it for memory, [`oom.rs`](../crates/dovetail-core/src/oom.rs) says so and restarts it once.

---

# Part 2: today against v2

The v2 design keeps the skeleton of Part 1: one loop, `Snapshot`/`Event`/`Command`, the driver, the DB thread, the bridge. It changes what a project is and adds a layer above tasks. The TUI stays the main client (ADR 0030); a page served by the core joins it for long reading and canvases.

Size: **S** = days, additive. **M** = about a week, touches existing paths. **L** = more than a week, or changes how something fundamental works.

| Layer | Today | v2 | Change | Size | Decided in |
| --- | --- | --- | --- | --- | --- |
| UI | Terminal UI in the same process | The same TUI, with v2 views and ADR 0030 §5 readability; plus one local page (reader, then canvases) | TUI views after the design session. Page: a small HTTP server in the core on `127.0.0.1`. The Tauri GUI is shelved on `gui-shelved`. | L | ADR 0030 |
| Core / UI seam | Two Rust channels | Channels for the TUI; the same messages as JSON for the page | Types exported to TypeScript (ts-rs) and `apply` written twice, tested against shared cases. **Built** for the GUI; the page reuses them. | S | ADR 0023, ADR 0030 |
| Process | One process: TUI and core | The same until `dovetail serve`, built last; clients then attach over the project socket | Socket carries commands and an update stream to several clients, a snapshot on attach, a core that outlives its clients | L | ADR 0020, ADR 0030 |
| Project | A folder; one repo in the `projects` row | A name, 0..n repos, one account | New `repos` table; opening by id, not by folder. **Built.** | M | ADR 0021 |
| Coordinator | Runs in the repo's main checkout | Runs in its own folder, every repo attached | New working folder means a fresh coordinator session once. **Built.** | M | ADR 0021 |
| Slice | Worktree of *the* repo | Worktree of one named repo | `project_start_slice` gains `repo`; `agents.repo_id`. **Built.** | S | ADR 0021 |
| Work model | Tasks in buckets | Efforts → criteria → tasks; buckets dropped | New tables, types, commands. **Built in this change.** Dropping buckets comes with the TUI's v2 views. | M | this PR, ADR 0030 |
| Chat | One conversation per agent, no tags | One coordinator chat; every line has an effort tag | `conversation_entries.effort_id`. **Built in this change.** | S | this PR |
| Task status | Set by hand, except "in progress" on slice start | Dovetail sets all of it | New `review` status; PR state drives the transitions. **Built** (stacked PR). | M | ADR 0024 |
| Session names | `coordinator`, same in every project | Unique per project: `<project>.<name>` | Changes what Claude Code sees as the name. **Built** (stacked PR). | S | ADR 0022 |
| Slice → coordinator | Claude Code's own messaging | A Dovetail tool | New tool `coordination_report`; role prompts change. **Built** (stacked PR). | M | ADR 0022 |
| Canvas | None | Canvases with cards that point at rows | New tables, commands and tools | L | ADR 0025 |
| Driver | NDJSON stream behind `SessionDriver` | Same | One new launch flag per attached repo (`--add-dir`). **Built.** | S | ADR 0021 |
| Database engine | SQLite, one thread, forward-only migrations | Same | — | — | — |
| MCP bridge, socket, hooks | As in section 3 | Same | — | — | — |

## Before and after

### Projects

```
 TODAY                                   V2
 registry.db                             registry.db
 └── projects                            ├── projects (id, name, account)
       id, name, kind,                   └── repos (id, project_id, name, path, git_common_dir)
       repo_path, git_common_dir
       (exactly one repo, or none)       "Shopfront" → shopfront-api, shopfront-web
                                         "Market notes" → no repos
 coordinator cwd: the repo               coordinator cwd: projects/<id>/home/, repos attached
 slice: worktree of the repo             slice: worktree of ONE repo, named at start
```

**Example.** Today, starting a slice needs only a task and a name, because there is one repo. In v2 the coordinator calls `project_start_slice { task: 12, name: "api-guest", repo: "shopfront-api" }`. With one repo the argument can be left out, so today's projects behave as before.

### Work model

```
 TODAY                          V2 (built in this change, not shown in the TUI)
 tasks                          efforts ─┬─◄ criteria   (open → claimed → met)
 (bucket: now / next / later)            └─◄ tasks      (tasks.effort_id, tasks.criterion_id)
                                conversation_entries.effort_id   (the effort tag)
```

**Example.** Effort `guest-checkout` (kind feature) has criterion 3 "pay as guest end to end". Task 52 "API: guest order endpoint" serves criterion 3. The agent claims criterion 3 with evidence "PR #41, test `guest_pays`": state `claimed`. You approve: state `met`. Only your command can do that last step; there is no tool for it.

Existing tasks land in one effort named `unsorted`. Existing chat lines keep no tag.

### How the page talks to the core

```
 TODAY                                    V2 (planned, ADR 0030 §4)
 TUI ── std channel ──► core              TUI ── std channel ──► core ◄── Command (JSON) ── page
 TUI ◄── std channel ── core              TUI ◄── std channel ── core ── ServerMsg (JSON) ─► page
        (same process)                     (TUI and core in one process; page in a browser tab,
                                            over HTTP on 127.0.0.1)
```

Nothing new is invented: `ServerMsg` and `Command` already have JSON forms, and the shelved GUI's TypeScript `CoreLink` ("subscribe", "send") and `apply` carry over to the page. The page follows the TUI's focus. When `dovetail serve` lands, the TUI moves from channels to the project socket.

### Task status

```
 TODAY   todo ──slice starts──► in_progress ──you mark it──► done
 V2      todo ──agent starts──► in_progress ──PR open──► review ──PR merged──► done
```

The first arrow exists today (`start_slice` in [`slice.rs`](../crates/dovetail-core/src/slice.rs)). The core already learns when a PR opens or merges (`on_worktree_gathered` in [`worktree_info.rs`](../crates/dovetail-core/src/worktree_info.rs)); v2 connects that to the task.

## What was built, and what waits

Built in this change: migration `0017`, the effort and criterion types, their commands and events, `apply`, the core handlers, and the effort tag on chat lines. No screen changes.

Decided and approved, not built yet: ADRs 0021 to 0025 in [`docs/adr/`](adr/), and ADR 0030's v2 TUI views, page and daemon. ADR 0023 (the Tauri shell) is superseded and ADR 0026 (previews) withdrawn.

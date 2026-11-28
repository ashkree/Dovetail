Dovetail's core: sessions, storage, worktrees, and the MCP tool handlers.

The core hands the UI the same [`dovetail_api::CoreLink`] that `MockCore` does, and launches
sessions through the registered [`Drivers`] (`ClaudeDriver`, `AgyDriver`, or fakes in tests).
All state lives in one single-threaded event loop; the UI talks to it only by sending
`Command`s in and receiving a `Snapshot` followed by sequenced `Event` deltas out.

# Layout

`dovetail-core` is organised into four infrastructure modules and eight domain modules
mirroring `dovetail-api` (plus `settings/`):

```text
src/
  lib.rs          module tree, crate docs, and public re-exports

  # Infrastructure
  engine/         the single-threaded Core loop, CoreOptions, CoreHandle, spawn,
                  startup checks, graceful shutdown, and thin command router
  storage/        the dedicated SQLite DB thread (Db, DbError, Pending),
                  forward-only migrations (registry.db + per-project state.db),
                  and cross-domain summary queries; domain SQL lives in each domain
  sessions/       session process lifecycle, kept separate from agents/:
                  Drivers, Claude/agy launch, stream state, restore, native takeover,
                  foreground wait, systemd cgroup memory caps, OOM recovery, context fill
  mcp/            Unix socket server (socket.rs) for `dovetail add`, staged upgrades,
                  and MCP bridge calls, plus thin role/suite-checked tool dispatch (tools.rs)

  # Domains (mirroring dovetail-api + settings/)
  tracking/       projects, multi-repo registration, home dir, tasks, decisions,
                  efforts, task read formatting, and tracking SQL (db/efforts.rs, db/tasks.rs)
  agents/         slices, tangents, side chats, journal (eod/fresh/morning),
                  native to-do lists, attachments, @-completion, slice reports,
                  and agy model catalog
  activity/       inbox permissions/questions, activity feed, peer messages,
                  status-bar warnings, totals, and driver usage reports
  visual/         maps (draw/load), canvases (blocks/cards/links), slice previews,
                  declared design variants, and visual SQL (db/canvas.rs, db/design.rs, db/maps.rs)
  research/       research projects/tasks, findings, sources, rounds, ask/browse,
                  file exports, markdown doc images, research tools, and research SQL (db.rs)
  code/           git worktree lifecycle (create/inspect/remove) and worktree info polling
  app/            clipboard paste/copy, external editor, browser opener, and `:upgrade`
  settings/       config.toml, project recipes, Claude profiles, and XDG paths
```

# The single-threaded `Core` loop

[`spawn`] starts two OS threads:

1. `dovetail-commands` — forwards synchronous `Command`s from the UI's `std::sync::mpsc`
   channel into the core's tokio `UnboundedSender<Input>`.
2. `dovetail-core` — runs a `current_thread` tokio runtime hosting the `Core::run` loop and
   the Unix socket server (`mcp::socket::serve`).

Everything that mutates core state flows through `Core::run` as an `Input`: UI commands,
socket requests (`dovetail add`, staged upgrades, MCP tool calls), `DriverEvent`s from
running sessions, and completion messages from background blocking tasks (git worktree
creation/removal, clipboard, editor, upgrade builds, OOM watcher). Handlers never block the
loop: session methods only enqueue, and git/filesystem work runs on `tokio::task::spawn_blocking`
and reports back as an `Input`.

## Domain state structs and thin routers

`Core` holds infrastructure handles (`db`, `paths`, `config`, `options`, channels, event
sequence counter, quit/shutdown timers) directly, and groups all mutable domain state into
one struct per module:

- `mcp::McpState` — primary and extra Unix socket paths
- `sessions::SessionState` — live handles, generations, takeover locks, cgroup scopes, OOM state, context windows, Claude profile home
- `tracking::TrackingState` — active project state, efforts, effort tags, send-back and reattach tracking
- `agents::AgentState` — agent rows, conversation tails, delta batching, slice queue, coordinator notes, native to-dos, slash commands, tangents, model catalog
- `activity::ActivityState` — open inbox items and waiters, activity/message feeds, warnings, totals, driver usage
- `visual::VisualState` — running slice preview processes
- `research::Research` — active research capture state
- `code::CodeState` — cached worktree info per slice
- `app::AppState` — editor state and `:upgrade` state

Three thin routers delegate to `impl Core` methods living inside each owning domain:

- `Core::command` (`engine/mod.rs`) — dispatches each `dovetail_api::Command` variant to its domain handler (`handle_task_command`, `handle_canvas_command`, etc.).
- `Core::tool` (`mcp/tools.rs`) — checks agent role and enabled tool suites, then dispatches each `dovetail_tools::Tool` variant to its owning domain.
- `Core::on_driver` (`sessions/mod.rs`) — routes each `dovetail_driver::DriverEvent` by generation and origin (session vs. subagent) into session, agent, and activity handlers.

Every state change emits one or more sequenced `dovetail_api::Event`s via `Core::emit`,
flushing buffered streaming deltas first so the UI always sees events in order.

# The dedicated SQLite storage thread

All SQLite access goes through one dedicated OS thread (`storage/mod.rs`) that owns the
`rusqlite::Connection`s for `registry.db` and the active project's `state.db` (or an
in-memory `state.db` for unregistered folders until `:project add` persists it).

Callers hold a cheap, cloneable `storage::Db` handle and invoke typed methods returning a
`storage::Pending<T>`, which can be `.await`ed in async code or blocked on with
`Pending::wait` during synchronous CLI startup.

Migrations are forward-only SQL files under `src/storage/migrations/{registry,state}/`,
tracked via `PRAGMA user_version`. Domain-specific SQL queries live next to their owning
domain (`tracking/db/`, `visual/db/`, `research/db.rs`), while `storage/mod.rs` runs closure
`Job`s submitted via `Db::run` against `Conns` on the single DB thread, and domain `db`
modules add typed methods directly via `impl Db`.

# Extending `dovetail-core`

## Adding a new command

1. Define the command and event types in `dovetail-api` under the owning domain.
2. Add the arm in `Core::command` (`src/engine/mod.rs`) delegating to a `handle_*_command`
   method in `src/<domain>/`.
3. Implement the handler as an `impl Core` method in `src/<domain>/`, mutating
   `self.<domain>`, persisting through `self.db` (or `self.persist`), and emitting events
   with `self.emit(...)`.
4. Add an integration test in `tests/<domain>.rs` using `tests/common/mod.rs`.

## Adding a new MCP tool

1. Define the tool schema and `Tool` variant in `dovetail-tools`.
2. Route the variant in `Core::tool` (`src/mcp/tools.rs`), checking role/suite gating.
3. Implement the handler in the owning domain (`src/<domain>/`), returning a
   `SocketResponseBody` via `crate::mcp::tools::{ok, error}`.
4. Cover it in `tests/<domain>.rs` via `Harness::call` or `socket_call`.

## Adding a new domain

1. Create `src/<domain>/mod.rs` with a `<Domain>State` struct (and `db.rs` / `db/` if it
   owns SQLite tables, plus a new contiguous migration in `src/storage/migrations/state/`).
2. Wire the module in `src/lib.rs`, add its state field to `Core` in `src/engine/mod.rs`,
   register its command/tool dispatch arms in `engine/mod.rs` and `mcp/tools.rs`, and add
   typed `impl Db` methods via `Db::run` in the domain's `db` module.
3. Add a matching integration test file `tests/<domain>.rs` reusing `tests/common/mod.rs`.

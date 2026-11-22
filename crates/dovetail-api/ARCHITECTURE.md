`dovetail-api`: the plain-data protocol between the Dovetail core and its clients.

Everything that crosses between the core and a client — the [`Snapshot`], [`Command`]s, [`Event`]s, and the [`apply`] reducer that folds events into a snapshot — lives here as plain Rust data. The crate has no I/O, no threads, and no async runtime (only `serde`, `serde_json`, and `std::sync::mpsc` in [`CoreLink`]). Agent-to-core MCP tools and unix-socket RPC messages live separately in `dovetail-tools`.

# The domain map

Code is grouped by **domain**, then by **feature**. A domain is only a folder: it has no types of its own and does not show up in [`Command`] or [`Event`]. A feature is one area of the protocol with its own commands, events and reducer.

| Domain | Features | What it is |
|---|---|---|
| `tracking/` | `project`, `effort`, `task`, `decision` | What the user is doing and why |
| `agents/` | `agent`, `conversation`, `model`, `mcp`, `tangent`, `journal` | The Claude sessions: their chats, models, MCP servers, side questions and journal |
| `activity/` | `inbox`, `feed`, `status` | What needs the user and what happened; `status` also holds the usage limits |
| `visual/` | `canvas`, `map`, `design` | Things drawn: canvases, maps, design variants |
| `research/` | `research`, `library` | Research runs and the library they fill |
| `code/` | `worktree` | The code the agents work on |
| `app/` | (one feature) | The app itself: resync, quit, open a URL, command failures; also the terminal hand-over, the clipboard and `:upgrade` |

`snapshot.rs`, `wire.rs`, `apply.rs` and `lib.rs` stay at the top.

## Paths and re-exports

The full path of a type is `domain::feature::Type`, for example `dovetail_api::tracking::task::Task`. To keep callers short, `lib.rs` also re-exports every feature module at the crate root, so `dovetail_api::task::Task` names the same type. Two domains cannot do that, because a domain and its namesake feature would share one name: `research` and `activity` re-export the items of their namesake feature (`research::research`, `activity::feed`) at the domain level, so `dovetail_api::research::ResearchSummary` and `dovetail_api::activity::ActivityItem` work, and the other feature is reached as `dovetail_api::library` or `dovetail_api::status`. Use the short path in callers; use the full path in docs when the domain helps the reader.

# The two directions

A connection carries [`Command`]s into the core and [`ServerMsg`]s out from it over [`CoreLink`]:

- **Out from the core ([`ServerMsg`]).** Every connection starts with [`ServerMsg::Hello`] carrying [`PROTOCOL_VERSION`], followed by [`ServerMsg::Snapshot`] (the full client-visible state, trimmed with [`Snapshot::trimmed`]), and then a stream of numbered [`ServerMsg::Event`] messages.
- **In from clients ([`Command`]).** The UI sends commands asking the core to act (create a task, send a prompt, switch a model, resync).

[`Command`] and [`Event`] are top-level enums with one variant per feature, wrapping that feature's own enum (for example, `Command::Task(TaskCommand::Create(..))`). Both use `#[serde(rename_all = "snake_case")]`, so on the wire each command or event is a nested object such as `{"task":{"create":{..}}}`. Today [`CoreLink`] uses in-process `mpsc` channels; a daemon carries the same values as NDJSON over a socket.

# The four-file feature pattern

Each feature is a folder with up to four files:

```text
tracking/task/
  mod.rs      the feature's types and ids; `pub use` of its command and event types
  command.rs  TaskCommand and command-only payload types
  event.rs    TaskEvent and event-only payload types
  apply.rs    pub(crate) fn apply(state: &mut Snapshot, event: TaskEvent)
```

Not every feature needs all four:

- **Event-only** features (`feed`) have no `command.rs`.
- **Pass-through** features (`library`, `mcp`) carry events the UI reacts to directly without storing anything in [`Snapshot`], so they have no `apply.rs`.
- Extra files are fine: `agents/conversation/attachment.rs` holds the attachment types; `visual/canvas` exposes `pub mod geometry`.

The `app` feature is the one place several things share a feature: its `AppCommand` and `AppEvent` carry the terminal hand-over (`OpenEditor`, `TakeOver`, `TerminalLend`, …), the clipboard (`Paste`, `Pasted`, …) and `:upgrade` (`StartUpgrade`, `UpgradeBuilding`, …), named by prefix. `status` likewise carries the usage limits: `UsageChanged` (live figure), `StatusCommand::LoadUsage` and `UsageLoaded` (the per-driver usage menu, with `DriverUsage`, `UsageGroup`, `LimitWindow`). `LimitWindow` is not folded into `UsageWindow`: it is a named window from an on-demand report with no status word, while `UsageWindow` is the live figure from a session event.

```text
src/
  lib.rs        module list, this document, and the root re-exports
  snapshot.rs   the flat Snapshot struct, trimmed(), and the tail-cap constants
  wire.rs       PROTOCOL_VERSION, ServerMsg, CoreLink, and the top-level Command and Event
  apply.rs      ApplyResult, apply(), and the shared upsert / push_bounded helpers
  <domain>/<feature>/   as above
tests/features/
  main.rs, common.rs, coverage.rs, and <domain>/<feature>.rs (one test binary)
```

# Keeping a snapshot up to date

A client stores the [`Snapshot`] it receives from [`ServerMsg::Snapshot`] and passes every later [`ServerMsg::Event`] through [`apply`]:

1. **Sequence check.** `apply` expects `seq == state.seq + 1`:
   - `seq < expected` returns [`ApplyResult::Stale`] and leaves `state` unchanged.
   - `seq > expected` returns [`ApplyResult::Gap`], leaves `state` unchanged, and tells the client to send `Command::App(AppCommand::Resync)`.
   - `seq == expected` advances `state.seq` and dispatches the event.
2. **Per-feature reducer.** `apply` matches [`Event`] and calls `<feature>::apply(state, event)` (or does nothing for the pass-through features). Reducers change the flat fields on [`Snapshot`] with the helpers in `apply.rs`: `upsert` replaces by key or appends, `push_bounded` appends and drops the oldest past a cap.
3. **Tail caps (`snapshot.rs`).** [`Snapshot::trimmed`] cuts each agent's conversation to [`snapshot::CONVERSATION_TAIL`] entries before sending a snapshot, setting `has_more` when entries were dropped. Clients keep up to [`snapshot::CONVERSATION_KEEP`] entries as older history is paged in. The `feed` reducer bounds `activity` and `messages` to [`snapshot::ACTIVITY_TAIL`] and [`snapshot::MESSAGE_TAIL`].

# How to add a feature

1. Pick the domain. Create `src/<domain>/<feature>/` with `mod.rs`, `event.rs` and, unless it is event-only or pass-through, `command.rs` and `apply.rs`. Add `pub mod <feature>;` to `src/<domain>/mod.rs` and a `pub use <domain>::<feature>;` to `src/lib.rs`. A new domain is a new folder with a `mod.rs` of `pub mod` lines, plus `pub mod <domain>;` in `lib.rs`.
2. Add `<Feature>(<Feature>Command)` and/or `<Feature>(<Feature>Event)` to `Command` and `Event` in `src/wire.rs`.
3. Add an arm in `src/apply.rs` calling `<feature>::apply(state, event)` (or add the variant to the pass-through no-op arm).
4. If the feature adds state, add the fields to `src/snapshot.rs` (with `#[serde(default)]` where older snapshots may omit them).
5. Add `tests/features/<domain>/<feature>.rs` with serde round-trips, reducer tests and `pub fn cases() -> Vec<Case>` covering every `<Feature>Event` variant (`Case::no_op` for pass-through events). Register it in `tests/features/<domain>/mod.rs`.

# How to add a variant to a feature

1. Add the variant (and any payload struct) to the feature's `command.rs` or `event.rs`; re-export a new public payload type from its `mod.rs`.
2. For an event, handle it in the feature's `apply.rs` (or leave it to the pass-through arm).
3. Add a round-trip check and a `Case` in the feature's test file. `tests/features/coverage.rs` finds every `pub enum <Feature>Event` under `src/` and fails the build if `all_cases()` misses a variant.

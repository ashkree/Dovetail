`dovetail-api`: the plain-data protocol between the Dovetail core and its clients.

Everything that crosses between the core and a client — the [`Snapshot`], [`Command`]s, [`Event`]s, and the [`apply`] reducer that folds events into a snapshot — lives here as plain Rust data. The crate has no I/O, no threads, and no async runtime (only `serde`, `serde_json`, and `std::sync::mpsc` in [`CoreLink`]). Agent-to-core MCP tools and unix-socket RPC messages live separately in `dovetail-tools`.

# The two directions

A connection carries [`Command`]s into the core and [`ServerMsg`]s out from it over [`CoreLink`]:

- **Out from the core ([`ServerMsg`]).** Every connection starts with [`ServerMsg::Hello`] carrying [`PROTOCOL_VERSION`], followed by [`ServerMsg::Snapshot`] (the full client-visible state, trimmed with [`Snapshot::trimmed`]), and then a stream of numbered [`ServerMsg::Event`] messages.
- **In from clients ([`Command`]).** The UI sends commands asking the core to act (create a task, send a prompt, switch a model, resync).

[`Command`] and [`Event`] are top-level enums with one variant per feature, wrapping that feature's own sub-enum (for example, `Command::Task(TaskCommand::Create(..))`). Both use `#[serde(rename_all = "snake_case")]`, so on the wire each command or event serialises as a tagged nested object such as `{"task":{"create":{..}}}`. Today [`CoreLink`] uses in-process `mpsc` channels; a daemon carries the same values as NDJSON over a socket.

# Layout

```text
src/
  lib.rs          module list, #![doc = include_str!("../ARCHITECTURE.md")], and root
                  re-exports (Command, Event, ServerMsg, CoreLink, PROTOCOL_VERSION,
                  Snapshot, apply, ApplyResult)
  snapshot.rs     flat Snapshot struct, trimmed(), and the four tail-cap constants
  wire.rs         PROTOCOL_VERSION, ServerMsg, CoreLink, top-level Command and Event
  apply.rs        ApplyResult, apply(), and the shared upsert / push_bounded helpers
  <feature>/      24 feature folders, one per domain area
    mod.rs        the feature's types and ids; explicit `pub use` of sub-enums and
                  command/event payloads (no glob re-exports)
    command.rs    <Feature>Command and command-only payload types
    event.rs      <Feature>Event and event-only payload types
    apply.rs      pub(crate) fn apply(state: &mut Snapshot, event: <Feature>Event)
tests/
  features/       one integration test binary: main.rs, common.rs, coverage.rs, and
                  one <feature>.rs per feature
```

Not every feature folder needs all four files:

- **Event-only features** (`activity`, `status`, `usage`) have no `command.rs`.
- **Pass-through features** (`app`, `clipboard`, `library`, `mcp`, `terminal`) carry events the UI reacts to directly without storing anything in [`Snapshot`], so they have no `apply.rs`.
- **Extra feature submodules**: `conversation/attachment.rs` holds image attachment types and limits; `canvas/mod.rs` exposes `pub mod geometry`.

# Keeping a snapshot up to date

A client stores the [`Snapshot`] it receives from [`ServerMsg::Snapshot`] and passes every subsequent [`ServerMsg::Event`] through [`apply`]:

1. **Sequence check.** `apply` expects `seq == state.seq + 1`:
   - `seq < expected` returns [`ApplyResult::Stale`] and leaves `state` unchanged.
   - `seq > expected` returns [`ApplyResult::Gap`] with `expected` and `got`, leaves `state` unchanged, and tells the client to send `Command::App(AppCommand::Resync)`.
   - `seq == expected` advances `state.seq = seq` and dispatches the event.
2. **Per-feature reducer.** `apply` matches [`Event`] and delegates to `<feature>::apply(state, event)` (or does nothing for the pass-through features). Reducers mutate the flat fields on [`Snapshot`] using shared helpers in `apply.rs` (`upsert` to replace by key or append, `push_bounded` to append and drop the oldest items past a cap).
3. **Tail caps (`snapshot.rs`).** [`Snapshot::trimmed`] cuts each agent's conversation to [`snapshot::CONVERSATION_TAIL`] entries before sending a snapshot, setting `has_more = true` when entries were dropped. Clients keep up to [`snapshot::CONVERSATION_KEEP`] entries as older history is paged in via `ConversationEvent::Page`. The `activity` reducer bounds `activity` and `messages` to [`snapshot::ACTIVITY_TAIL`] and [`snapshot::MESSAGE_TAIL`].

# Adding a feature

1. Create `src/<feature>/` with `mod.rs`, `event.rs`, and — unless the feature is event-only or pass-through — `command.rs` and `apply.rs`. Re-export the sub-enums and any public payload types explicitly in `mod.rs` (no globs), and add `pub mod <feature>;` to `src/lib.rs`.
2. Add the `<Feature>(<Feature>Command)` and/or `<Feature>(<Feature>Event)` variant to `Command` and `Event` in `src/wire.rs`.
3. Add an arm in `src/apply.rs` calling `<feature>::apply(state, event)` (or add the variant to the pass-through no-op arm).
4. If the feature adds state to [`Snapshot`], add its fields in `src/snapshot.rs` (with `#[serde(default)]` where older snapshots may omit them).
5. Add `tests/features/<feature>.rs` with serde round-trips, reducer tests, and `pub fn cases() -> Vec<Case>` covering every variant of `<Feature>Event` (use `Case::no_op` for pass-through events). Register `mod <feature>;` and `<feature>::cases()` in `tests/features/main.rs`.

# Adding a variant to an existing feature

1. Add the variant (and any payload struct) to `src/<feature>/command.rs` or `src/<feature>/event.rs`. If you add a new public payload type, re-export it explicitly in `src/<feature>/mod.rs`.
2. For an event, handle the new variant in `src/<feature>/apply.rs` (or keep it in a pass-through feature).
3. Add a round-trip check and a `Case` in `tests/features/<feature>.rs` that exercises the new event variant. `tests/features/coverage.rs` scans `pub enum <Feature>Event` in every `src/*/event.rs` and asserts that `all_cases()` produces every variant, so any uncovered event fails `cargo test`.

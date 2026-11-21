`SessionDriver`: runs headless agent CLI sessions and turns their output into typed events.

The driver hides each CLI behind [`SessionDriver`] and [`Session`]. The core never sees a process, a pipe or a raw line unless it asks for one: every stdout line becomes a typed [`DriverEvent`] that still carries, or can carry, its raw JSON. Each driver maps its own protocol onto the one shared event vocabulary, so the core handles every driver the same way. [`DriverKind`] names the driver.

# Stream rules (spike S1, `docs/spikes/driver.md`; these are law)

1. **Route by `parent_tool_use_id`.** A subagent's events arrive on the parent session's stream. Every event that can come from a subagent carries an [`Origin`]. A non-`None` `parent_tool_use_id` names the `Task` tool use that spawned it. `agent_id` is set too when the CLI provides it, which it always does on subagent permission requests.
2. **The first `result` is not the end.** A background subagent produces several [`DriverEvent::TurnResult`]s per prompt (`result_index` counts them), each after a fresh `Init`. A session is done when the process exits ([`DriverEvent::Exited`]) or when no background tasks remain ([`DriverEvent::BackgroundTasksChanged`] with an empty list). A `result` alone never ends it.
3. **Answer permission requests concurrently.** The stdout reader never waits for an answer. All [`Session`] methods only enqueue, so a held [`DriverEvent::PermissionRequest`] never blocks another request, the stream, or an interrupt. Answers may come in any order.
4. **Record session ids yourself.** Dovetail picks the id (`--session-id`) and stores it; the `Init` event confirms it. Resume with [`SessionSpec::resume`].
5. **Never crash on the unknown.** Unknown line types, subtypes and fields end up in [`DriverEvent::Unknown`] with the raw JSON, and so do non-JSON lines.

# Layout

```text
src/
  lib.rs        the traits (SessionDriver, Session), DriverError, UserInput, ControlAnswer
  event.rs      DriverEvent and the types it carries: the vocabulary every driver maps onto
  spec.rs       SessionSpec (what any driver needs to launch), PermissionMode, SystemdScope
  process.rs    the child process under any driver: spawn with piped stdio, stdin writer,
                stdout line reader, stderr tail, supervisor (reaps, then sends Exited), fixture log
  fake.rs       FakeDriver: replays a recorded fixture and records what it is sent
  base64.rs     image encoding for UserInput
  claude/       Claude Code over stream-json
    mod.rs        ClaudeDriver, ClaudeSession (the Session methods as control requests)
    args.rs       ClaudeOptions and the argv: headless flags, native take-over flags
    reader.rs     the stdout reader: lines through parse, control answers routed to their request
    parse.rs      stream-json line → DriverEvent (public: tests and the core map lines with it)
    switch.rs     the live model switch: set_model → apply_flag_settings → get_settings
    mcp.rs        the MCP menu: mcp_status, reconnect, toggle, authenticate
    transcript.rs reading what a native run appended to the JSONL transcript (take-over)
```

# How a session runs

`start` spawns the process (inside `systemd-run --user --scope` when the spec has a scope) and leaves four tasks running. They are the stdin writer, the stdout reader, the stderr tail and the supervisor. The [`Session`] handle only sends to them. The supervisor owns the child: on `shutdown` it closes stdin and kills after the grace period, on `kill` or drop it kills, and once the process is reaped it sends [`DriverEvent::Exited`], always last.

Claude-specific requests that take several steps, the model switch and the MCP menu, keep their state behind a mutex shared by the handle and the reader. The handle starts a request; the reader matches each `control_response` to it and sends the next step or the final event.

# Tests

[`FakeDriver`] replays NDJSON fixtures (`tests/fixtures/`, `{"t","dir","msg"}` per line) through the same parser the real driver uses, and records every [`Sent`] item. Setting [`SessionSpec::log_path`] on a real session writes this format, so any session can become a fixture.

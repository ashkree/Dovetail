`SessionDriver`: runs headless agent CLI sessions and turns their output into typed events.

The driver hides each CLI behind [`SessionDriver`] and [`Session`]. The core never sees a process, a pipe or a raw line unless it asks for one: every stdout line becomes a typed [`DriverEvent`] that still carries, or can carry, its raw JSON. Each driver maps its own protocol onto the one shared event vocabulary, so the core handles every driver the same way. [`DriverKind`] names the driver.

# Stream rules (spike S1, `docs/spikes/driver.md`; these are law)

1. **Route by `parent_tool_use_id`.** A subagent's events arrive on the parent session's stream. Every event that can come from a subagent carries an [`Origin`]. A non-`None` `parent_tool_use_id` names the `Task` tool use that spawned it. `agent_id` is set too when the CLI provides it, which it always does on subagent permission requests.
2. **The first `result` is not the end.** A background subagent produces several [`DriverEvent::TurnResult`]s per prompt (`result_index` counts them), each after a fresh `Init`. A session is done when the process exits ([`DriverEvent::Exited`]) or when no background tasks remain ([`DriverEvent::BackgroundTasksChanged`] with an empty list). A `result` alone never ends it.
3. **Answer permission requests concurrently.** The stdout reader never waits for an answer. All [`Session`] methods only enqueue, so a held [`DriverEvent::PermissionRequest`] never blocks another request, the stream, or an interrupt. Answers may come in any order.
4. **Record session ids yourself.** For Claude Code, Dovetail picks the id (`--session-id`) and stores it, and the `Init` event confirms it. agy cannot be told an id: it picks its own, and the agy driver reports it in the first `Init` (from the `session/new` answer), before any other session event. The core stores that id. Resume with [`SessionSpec::resume`], passing the stored id as `session_id`.
5. **Never crash on the unknown.** Unknown line types, subtypes and fields end up in [`DriverEvent::Unknown`] with the raw JSON, and so do non-JSON lines.

# Layout

```text
src/
  lib.rs        the traits (SessionDriver, Session), DriverError, UserInput, ControlAnswer
  event.rs      DriverEvent and the types it carries: the vocabulary every driver maps onto
  spec.rs       SessionSpec (what any driver needs to launch), PermissionMode, SystemdScope
  process.rs    the child process under any driver: spawn with piped stdio, stdin writer,
                stdout line reader, stderr tail, supervisor (reaps, then sends Exited), fixture log
  base64.rs     image encoding for UserInput
  fake/         FakeDriver: replays a recorded fixture and records what it is sent
    mod.rs        FakeDriver, ReplayOptions, Sent, the recorder
    replay.rs     the replay loop, through the fixture's driver's own mapping
    session.rs    the fake session handle
  claude/       Claude Code over stream-json
    mod.rs        ClaudeDriver, ClaudeSession (the Session methods as control requests)
    args.rs       ClaudeOptions and the argv: headless flags, native take-over flags
    reader.rs     the stdout reader: lines through parse, control answers routed to their request
    parse.rs      stream-json line → DriverEvent (public: tests and the core map lines with it)
    switch.rs     the live model switch: set_model → apply_flag_settings → get_settings
    mcp.rs        the MCP menu: mcp_status, reconnect, toggle, authenticate
    transcript.rs reading what a native run appended to the JSONL transcript (take-over)
  agy/          agy over ACP (`agy acp`, JSON-RPC 2.0 on stdio; docs/spikes/agy-headless.md)
    mod.rs        AgyDriver, AgySession: move lines between the process and the conversation
    conversation.rs  the protocol state, with no I/O: lines in, events and lines out
    args.rs       AgyOptions, McpServerSpec, the argv, session/new params, mode names
    rpc.rs        request ids and what each was for
    turn.rs       session/update → DriverEvent; chunks gathered into messages; the result
    models.rs     agy's ids (`<family>-<effort>`) as Dovetail's model and effort
    permission.rs session/request_permission and its answer
```

# The two drivers

| | Claude Code | agy |
| --- | --- | --- |
| Protocol | stream-json lines, `control_request`s | ACP: JSON-RPC requests, `session/update` notifications |
| Session id | Dovetail's (`--session-id`) | agy's, from `session/new` (rule 4) |
| Start-up | `initialize`, then `get_settings`, `list_models` | `initialize` → `session/new` or `session/resume` → the spec's model and mode; prompts wait until then |
| Prompts | written at once; Claude Code queues them | queued in the driver, one `session/prompt` at a time |
| Messages | complete `assistant` lines after their deltas | only chunks: the driver closes a message at a tool call or the turn's end |
| Interrupt | `interrupt` control request | `session/cancel`; the prompt answers `cancelled` (reported as Claude Code's `error_during_execution`) |
| Model switch | `set_model` → `apply_flag_settings` → `get_settings` | one `session/set_model` with the family-effort id |
| Permissions | `can_use_tool`; `--allowedTools` pre-allows | `session/request_permission`; the driver answers `AgyOptions::allowed_tools` itself, and denies everything else in `DontAsk` |
| MCP | `--mcp-config` file; the MCP menu | servers inline in `session/new`; no menu (answered as a refusal) |
| Role prompt | `--append-system-prompt-file` | the file's text in `_meta.antigravity.systemPrompt.append` |
| Settings | `--settings` file | `--cli_dir_name` folder holding `settings.json` |

# How a session runs

`start` spawns the process (inside `systemd-run --user --scope` when the spec has a scope) and leaves four tasks running. They are the stdin writer, the stdout reader, the stderr tail and the supervisor. The [`Session`] handle only sends to them. The supervisor owns the child: on `shutdown` it closes stdin and kills after the grace period, on `kill` or drop it kills, and once the process is reaped it sends [`DriverEvent::Exited`], always last.

Requests that take several steps keep their state behind a mutex shared by the handle and the reader. For Claude Code these are the model switch and the MCP menu: the handle starts a request, and the reader matches each `control_response` to it and sends the next step or the final event. For agy the whole conversation is that state, and both sides only feed it and send on what it returns.

# Tests

[`FakeDriver`] replays NDJSON fixtures (`tests/fixtures/`, `{"t","dir","msg"}` per line) through the same mapping the real driver uses, and records every [`Sent`] item. Claude Code fixtures are mapped line by line. agy fixtures (`tests/fixtures/agy/`, [`FakeDriver::for_agy`]) go through a conversation that also reads the recorded requests, so each answer finds its request. Setting [`SessionSpec::log_path`] on a real session writes this format, so any session of either driver can become a fixture. `tests/agy_live.rs` runs one real agy turn when `DOVETAIL_LIVE_TESTS=1`.

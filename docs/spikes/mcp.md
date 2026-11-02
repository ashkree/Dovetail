# S3: MCP behaviour

Run 2026-11-02, Claude Code 2.1.316, subscription login, headless `claude -p --output-format stream-json --verbose`. Scratch repo `/tmp/dovetail-spikes/mcp/`. About 5 live sessions (one smoke test, one 15-minute hold, one foreground subagent, one background subagent; all with tiny prompts). All claude processes have exited.

Code: `spikes/mcp/server.py` (toy stdio MCP server, tools `ping(hold_seconds, tag)` and `who(tag)`, logs every request with its `_meta` to `$SPIKE_LOG`) and `spikes/mcp/mcp.example.json`. Launch flags used: `--mcp-config <file> --strict-mcp-config --allowedTools mcp__spike__who mcp__spike__ping [Agent]`.

## Verdicts

| Check | Verdict | Evidence |
| --- | --- | --- |
| 15-minute hold in a main session | **pass** | `ping(900)` started at t0, server answered at t0+900.0 s, the session returned `pong hold15`, `is_error: false`, session duration 906 s. No setting changed (default `MCP_TOOL_TIMEOUT`), no progress notifications sent. |
| Call from a foreground subagent | **pass** | The subagent (an `Agent` tool call) called `mcp__spike__who`; the server received it and the result came back to the subagent. |
| Call from a background subagent | **pass** | Same, with `run_in_background: true` on the `Agent` call. The subagent called the tool and reported `pong bg`. |
| Can the server tell which agent called? | **pass, by stream correlation** | Every `tools/call` carries `params._meta["claudecode/toolUseId"]` (the tool_use id of the call) and `progressToken`. The same tool_use id appears in the stream-json `assistant` event whose `parent_tool_use_id` names the calling subagent (null for the main session). |

No fallback is needed for any check.

## Details

- The hold ran in a main session with the server idle on stdin (no heartbeats). 15 minutes is therefore safe; I did not test beyond that. Claude Code documents `MCP_TOOL_TIMEOUT` (ms) for longer holds; not tested, not needed for the pass.
- The server's tool call runs on a thread, so the server stays responsive during a hold.
- `_meta` on a call looked like (ids shortened): `{"claudecode/toolUseId": "toolu_01PG...", "progressToken": 2}`. There is no agent id or session id in `_meta`; the subagent is not identified directly.
- Attribution recipe for the core: the MCP bridge forwards `toolUseId` to the app. The core's stream parser already sees every `tool_use` block with its `parent_tool_use_id`, so it maps `toolUseId` to the calling agent (main session if `parent_tool_use_id` is null, else the subagent). Race to handle: the `tool_use` stream event and the MCP call can arrive in either order; look up the id from a map filled by the stream, and wait briefly (or park the call) if it is not there yet.
- Foreground run trace: main `Agent` tool_use `toolu_014n...` (parent null); subagent's tool_use `toolu_01PG...` has `parent_tool_use_id = toolu_014n...`; the server logged `claudecode/toolUseId = toolu_01PG...`.
- Because the bridge is per session (`dovetail mcp` is started per Claude Code process, and subagents share the parent's MCP connection), the session identity is known from which process connected; only subagent attribution needs the correlation above.
- At startup Claude Code sent a `server/discover` probe (protocol 2026-07-28 `_meta`) before `initialize` (protocol 2025-11-25). The toy server answered the probe with an empty result and Claude Code carried on. The real `dovetail mcp` should answer unknown methods with a JSON-RPC error `-32601` or an empty result, and must not hang on them. Subagent tool schemas were fetched via `ToolSearch` (deferred tools), so subagents may need one extra step to use `ask_user`.

## Consequences for the design

- `ask_user` with `blocking: true` can hold the tool call open for at least 15 minutes. The bounded-hold fallback ("return pending, deliver the answer as a user message") is **not** needed for the main session. Longer holds (hours, overnight) are untested; if the user wants that, run a follow-up with `MCP_TOOL_TIMEOUT` set high.
- Subagent guidance may keep `ask_user` and `report_status`; attribute them to the subagent via `toolUseId`.
- App exit while a call is held: untested here (covered by the M9 "clear error, not a hang" check).

## Open

- Holds longer than 15 minutes and the effect of `MCP_TOOL_TIMEOUT`: not run.
- Whether holding a call blocks the session's other work, such as queued user messages or interrupts: not run. Interrupt behaviour during a hold should be checked in M9.

# agy subagents over ACP

**Status:** run 2026-11-27 against `agy` 1.5.18 (`gemini-3.8-flash-medium`) in scratch repo `/tmp/agy-subagent-spike` (task #60). Probe script is `spikes/agy-headless/subagents.py`, recorded fixture is `spikes/agy-headless/out/subagent-live.ndjson`.

## Summary

When `agy` spawns a subagent via `invoke_subagent`:
1. `invoke_subagent` arrives on the ACP wire as a normal `tool_call` (`kind: "other"`), followed ~10ms later by an immediate `tool_call_update` (`status: "completed"`).
2. The subagent runs completely out-of-band in its own conversation under `~/.gemini/antigravity-cli/brain/<subagent-id>/`. Child tool calls (`run_command`, `read_file`, `send_message`) are **never sent** over the parent ACP connection.
3. No child tool calls, no `parent_tool_use_id`, and no `task_*` events appear on the ACP wire.
4. The ACP `tool_call_update` message contains **no `rawOutput` and no `content`** (even though the internal transcript records the subagent's `conversationId`).
5. When the subagent finishes, it sends an internal message (`send_message`) back to the parent session. The parent's in-flight `session/prompt` RPC **blocks** until this message arrives, receives it internally as a system message, resumes generation with `agent_thought_chunk` / `agent_message_chunk` reporting the subagent's result, and then completes the prompt with `stopReason: "end_turn"`.

---

## Wire Protocol Traces

### 1. Launch (`tool_call`)

When the model delegates to a subagent, ACP emits a standard `tool_call`:

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "fbb5f946-5951-470c-875f-d58881e18ea3",
    "update": {
      "sessionUpdate": "tool_call",
      "toolCallId": "call_2299841148900147759",
      "name": "invoke_subagent",
      "status": "in_progress",
      "kind": "other",
      "title": "Run invoke_subagent",
      "args": {
        "Prompt": "Read lines.txt in the current directory and count the lines. Reply with just the count.",
        "Role": "Line Counter",
        "TypeName": "research"
      },
      "rawInput": {
        "Prompt": "Read lines.txt in the current directory and count the lines. Reply with just the count.",
        "Role": "Line Counter",
        "TypeName": "research"
      }
    }
  }
}
```

Key observations:
- `name` is `"invoke_subagent"`.
- `args` / `rawInput` carry `Role` (human-readable title/description), `TypeName` (subagent type, e.g. `"research"`), `Prompt`, and optional `Model` / `Workspace`.
- `kind` is `"other"`.

### 2. Immediate Tool Call Update (`tool_call_update`)

Approximately 9ms after `tool_call`, ACP emits `tool_call_update`:

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "fbb5f946-5951-470c-875f-d58881e18ea3",
    "update": {
      "sessionUpdate": "tool_call_update",
      "toolCallId": "call_2299841148900147759",
      "name": "invoke_subagent",
      "status": "completed",
      "kind": "other",
      "title": "Run invoke_subagent",
      "args": {
        "Prompt": "Read lines.txt in the current directory and count the lines. Reply with just the count.",
        "Role": "Line Counter",
        "TypeName": "research"
      },
      "rawInput": {
        "Prompt": "Read lines.txt in the current directory and count the lines. Reply with just the count.",
        "Role": "Line Counter",
        "TypeName": "research"
      }
    }
  }
}
```

Key observations:
- `status` is `"completed"`.
- **No `rawOutput` and no `content`** are provided on the wire.
- In `agy`'s internal engine transcript, the tool output was:
  ```json
  {
    "conversationId": "8edae341-019a-43e6-a1b8-455d2298218a",
    "logAbsoluteUri": "file:///home/mave/.gemini/antigravity-cli/brain/8edae341-019a-43e6-a1b8-455d2298218a/.system_generated/logs/transcript.jsonl",
    "workspaceUris": ["file:///tmp/agy-subagent-spike"]
  }
  ```
  However, this object is stripped by the ACP server before sending to the client.

### 3. Out-of-band Execution & Handback

- The child process runs under its own conversation id (`8edae341-019a-43e6-a1b8-455d2298218a`).
- Child tool calls (`run_command`, `read_file`, etc.) are logged only in the child's transcript. **None appear on the parent ACP connection.**
- Once complete, the child calls `send_message` targeting the parent session.
- The parent session receives the message via internal messaging:
  ```
  <SYSTEM_MESSAGE>
  [Message] timestamp=2026-11-27T10:13:29Z sender=8edae341-019a-43e6-a1b8-455d2298218a priority=MESSAGE_PRIORITY_HIGH content=3
  </SYSTEM_MESSAGE>
  ```
- The parent session in ACP remains inside the **active `session/prompt` call** during this entire interval (22 seconds in our test).
- Upon receiving the message, the parent model streams:
  ```json
  {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "\n\nThe subagent reported that `lines.txt` has 3 lines."}}
  ```
  and then completes `session/prompt`:
  ```json
  {"id": 4, "result": {"stopReason": "end_turn", "usage": {"inputTokens": 15306, "outputTokens": 362, ...}}}
  ```

---

## Comparison: Claude Code vs. agy ACP

| Feature | Claude Code (stream-json) | agy (ACP) |
| --- | --- | --- |
| Launch Tool Name | `Agent` or `Task` | `invoke_subagent` |
| Launch Arguments | `description`, `subagent_type`, `prompt`, `run_in_background` | `Role`, `TypeName`, `Prompt`, `Model`, `Workspace` |
| Lifecycle Events | Native `task_started`, `task_updated`, `task_completed` (`system/task_*`) | None. Only `tool_call` and immediate `tool_call_update` |
| Subagent Routing | Events carry `parent_tool_use_id` matching launch tool call | None. Child runs out-of-band |
| Subagent Progress | `tool_use_count`, `token_count`, `duration_ms` in `task_updated` | None on wire |
| Launch Tool Result | Returns report or `outputFile` path when subagent completes | Returns ~10ms after launch with no `rawOutput` |
| Completion Signal | `task_completed` and/or foreground `ToolResult` | Subagent `send_message` to parent; parent streams chunks; prompt ends with `TurnResult` |

---

## Mapping to Dovetail

### 1. Recognizing `invoke_subagent` in Core & TUI

- **`crates/dovetail-core/src/session/subagent.rs`**:
  - `launches_subagent(&name)`:
    ```rust
    pub(crate) fn launches_subagent(tool: &str) -> bool {
        matches!(tool, "Agent" | "Task" | "invoke_subagent")
    }
    ```
  - `subagent_launched`:
    Map `Role` to description/name and `TypeName` to subagent type:
    ```rust
    let description = field("description").or_else(|| field("Role"));
    let name = description
        .clone()
        .or_else(|| field("subagent_type"))
        .or_else(|| field("TypeName"))
        .unwrap_or_else(|| "subagent".into());
    let subagent_type = field("subagent_type").or_else(|| field("TypeName"));
    ```
    Set `background: true` for `invoke_subagent` (since agy subagents run asynchronously).

- **`crates/dovetail-tui/src/activity.rs`**:
  - Update `activity_of`:
    ```rust
    "Task" | "Agent" | "invoke_subagent" => with("Waiting on subagent", text()),
    ```
    Extracting `Role` or `Prompt` for the activity label.

- **`crates/dovetail-core/src/research/capture.rs`**:
  - Update candidate tracking in `launched`:
    ```rust
    description: field("description").or_else(|| field("Role")),
    subagent_type: field("subagent_type").or_else(|| field("TypeName")),
    ```

### 2. Node Lifecycle & Completion Gap

In Claude Code, a subagent node closes when `task_completed` or foreground `ToolResult` arrives.
In agy ACP:
- `tool_call_update` arrives ~10ms after `tool_call`, signaling that the subagent was *spawned*, not that it *finished*.
- If Dovetail treats `ToolResult` for `invoke_subagent` as completion, the subagent node would close almost instantaneously while the subagent is still running.

**Resolution Strategy for Task #60:**
1. In `subagent.rs`, treat `invoke_subagent` as background (`AgentKind::Subagent { background: true }`). Foreground completion via `ToolResult` will not immediately close background subagents.
2. When the parent turn concludes (`DriverEvent::TurnResult` in `crates/dovetail-core/src/session/subagent.rs`), any running subagents under that parent that have not received explicit completion can be transitioned to `Done` (or have their status updated based on the turn's conclusion).
3. If granular subagent events are desired in future stages, Dovetail's MCP server can be passed to subagents (or subagent log files under `~/.gemini/antigravity-cli/brain/` can be tailed), but recognizing `invoke_subagent` with `Role`/`TypeName` is sufficient to populate the TUI agent tree during active turns.

# agy as a second session driver

**Status:** run 2026-11-21 against `agy` 1.5.9 (its ACP server reports 1.5.14) and Claude Code 2.1.325, in a scratch folder under `/tmp` (task #46, stage 1). Probes are in `spikes/agy-headless/`, their logs in `spikes/agy-headless/out/` (fixture format `{"t","dir","msg"}`). Each run needs `mkdir -p /tmp/agy-spike/repo` first. Then run `stream.sh <name> <input>` (stream-json), `input-events.sh` (which input events are accepted), `python3 acp.py <name> <scenario> [agy acp flags]` (ACP: hello, tool, cancel, model, mcp, resume), `python3 claude_usage.py <name>` (Claude Code's usage requests, with no prompt sent), or `python3 strings.py <regex>` (search the agy binary).

## Recommendation

1. **Drive agy over ACP (`agy acp`), not stream-json.** ACP has everything the Claude Code driver uses: interrupt, a live model and effort switch, permission requests the client answers, resume, per-session MCP servers, a role prompt and extra folders. agy's stream-json input takes only `user` lines: `control_request` is refused as "not supported yet", and `interrupt`, `cancel` and `set_model` are silently ignored.
2. **Dovetail's MCP server works in agy unchanged.** `dovetail mcp` is passed in `session/new`'s `mcpServers` with the same env as today. The tools show as `mcp_dovetail_<tool>`.
3. **Permissions go through Dovetail** once the session runs with its own settings folder: `agy acp --cli_dir_name <abs dir>` with `{"toolPermission":"request-review"}` in that folder's `settings.json`. Auth is unaffected. Without this, the user's own `toolPermission: always-proceed` applies and nothing is ever asked. Allow-rules in that file did not stop prompts for Dovetail's own tools, so the agy driver allows those itself (they are pre-allowed for Claude Code too, through `--allowedTools`).
4. **Usage data is real for both drivers, and neither needs a model call.**
   - Claude Code: the `get_usage` control request on any live (even idle) session. It returns the 5-hour and weekly windows, extra-usage credits and a weekly breakdown. The per-turn `rate_limit_event` stays as it is.
   - agy: `agy --output-format json -p=/usage` (alias `/quota`), a one-shot process that takes about a second. It returns quota groups (Gemini models, Claude and GPT models, Gemini 4 Argon), each with a weekly and a 5-hour bucket. `session/new` adds the plan tier.
5. **Switching driver mid-session is not possible.** The two keep conversations in different stores: Claude Code's JSONL versus agy's `conversations/<id>.db`, and neither can load the other's. A driver switch means a new session in the same cwd, seeded through the existing `:fresh` flow. A Claude session at its usage limit cannot write a handoff, so the seed is Dovetail's facts plus the old transcript's path. agy can read that file itself.
6. **agy offers Claude models on Google's quota** (`claude-opus-5-5-{low,medium,high}`, `claude-sonnet-5-5-*`) as well as Gemini. When Claude Code hits its limit, an agy session can keep working on Opus 5.5.
7. **The driver crate:** agy becomes a second implementation of the same `SessionDriver`/`Session` traits and the same `DriverEvent` vocabulary. Claude Code's code moves into `claude/` and agy's into `agy/`. Process plumbing that is shared today but sits in `claude.rs` moves to `process.rs`. The core gets a driver kind per session and a set of drivers instead of one. See the design below.

| Term | Meaning here |
| --- | --- |
| ACP | Agent Client Protocol: JSON-RPC 2.0 over stdio. The client (Dovetail) sends `initialize`, `session/new`, `session/prompt`; the agent sends `session/update` notifications and `session/request_permission` requests |
| driver kind | Which CLI runs a session: `claude` or `agy`. Fixed for the life of a session |
| model family / effort | agy model ids are `<family>-<effort>` (`gemini-3.8-flash-low`); its `configOptions` split them into `model` and `thought_level` |
| quota group | agy's shared limit pool: a set of models with a weekly and a 5-hour bucket |
| driver switch | A new session on the other driver, seeded by `:fresh`, in the same cwd |

## Design

### Before and after

```
before                                     after
dovetail-driver/src/                       dovetail-driver/src/
  lib.rs      traits, UserInput,             lib.rs        traits, DriverKind, UserInput, ControlAnswer
              ControlAnswer                  event.rs      DriverEvent vocabulary (unchanged shapes)
  event.rs    DriverEvent                    usage.rs      DriverUsage (both drivers' limits, one shape)
  spec.rs     SessionSpec + claude argv      spec.rs       SessionSpec (driver-neutral) + PermissionMode
  claude.rs   ClaudeDriver + process         process.rs    spawn, stdin writer, line reader, stderr tail,
              plumbing (659 lines)                         supervisor, fixture Log (from claude.rs)
  parse.rs    stream-json → events           fake.rs       FakeDriver; replays through a driver's mapper
  switch.rs   set_model chain                claude/
  mcp.rs      MCP menu requests                mod.rs      ClaudeDriver, ClaudeSession
  fake.rs     FakeDriver                       args.rs     claude_args, native_args (from spec.rs)
  transcript.rs                                parse.rs, switch.rs, mcp.rs, transcript.rs (moved)
  base64.rs                                    usage.rs    get_usage → DriverUsage
                                             agy/
                                               mod.rs      AgyDriver, AgySession
                                               args.rs     `agy acp` argv, session/new params
                                               rpc.rs      JSON-RPC ids, pending requests, answers
                                               map.rs      session/update → DriverEvent
                                               turn.rs     chunk accumulation into AssistantMessage
                                               models.rs   availableModels/configOptions → ModelInfo;
                                                           set_model / thought_level
                                               usage.rs    `agy -p=/usage` JSON → DriverUsage
                                             base64.rs
```

`dovetail_driver::parse` stays a public path: it re-exports `claude::parse`.

### Traits

`SessionDriver` and `Session` keep their shape, with these changes:

- `SessionDriver::kind(&self) -> DriverKind`.
- `SessionDriver::usage(&self) -> Result<oneshot::Receiver<DriverUsage>, DriverError>`. This is driver-level, because limits are per account. Claude asks any live session with `get_usage`; with none live, it starts an idle `claude -p` that is sent no prompt. agy runs the one-shot `/usage`. `FakeDriver` returns a canned report.
- `Session::session_id()` keeps returning Dovetail's id. **Stream rule 4 gains a clause:** agy picks its own conversation id (it cannot be told one). The core records it from the first `Init`, which the agy driver emits from the `session/new` answer before any other event. Resume passes it back.
- `Session::set_model(model, effort)`: agy sends `session/set_model {modelId: <family>-<effort>}`. Its `config_option_update` becomes `DriverEvent::ModelSettings`, and a JSON-RPC error becomes `ModelSwitchFailed`. That matches what the core expects today.
- `Session::mcp(...)`: ACP has no MCP menu. agy answers `McpOutcome::Failed("not supported by agy")` at once.
- `Session::set_permission_mode`: `session/set_mode` (see the mapping below).

### `SessionSpec`

The neutral fields stay: id/resume, name, cwd, model, effort, permission mode, role prompt file, add_dirs, env, systemd scope, log path. Claude Code's file paths and flags move into `claude: ClaudeOptions` (settings path, MCP config path, worktree, allowed tools, replay flag, extra args). agy's go into `agy: AgyOptions` (settings dir, plus the MCP servers as a typed `Vec<McpServerSpec {name, command, args, env}>`, which agy needs inline). The core's `session/launch.rs` splits the same way, into `launch/claude.rs` and `launch/agy.rs`. Both build from the shared bridge env.

### ACP → `DriverEvent`

| ACP | DriverEvent |
| --- | --- |
| `session/new` / `session/resume` answer | `Init {session_id, model: models.currentModelId, permission_mode: modes.currentModeId, version: agentInfo.version}`, `Models` (families × `thought_level` options), `ModelSettings {switch: None}` |
| `agent_message_chunk` | `TextDelta` |
| `agent_thought_chunk` | `ThinkingDelta` |
| chunks up to a `tool_call` or the turn's end | `Assistant(AssistantMessage)`: built by `agy/turn.rs`, with `usage` from the prompt answer |
| `tool_call` | `Assistant` with one `ToolUse {id: toolCallId, name, input: rawInput}` |
| `tool_call_update` `completed` / `failed` | `ToolResult {tool_use_id, content, is_error: failed}` |
| `session/request_permission` (a request with an id) | `PermissionRequest {id, tool_name, input: rawInput, tool_use_id, description: title}`; Allow → `optionId` of `allow_once`, Deny → `reject_once` |
| `session/prompt` answer `{stopReason, usage}` | `TurnResult {subtype: success / cancelled, model_usage: [{context_window: last usage_update.size, tokens}]}` |
| `available_commands_update` | `Commands` |
| `session_info_update.title` | `SessionTitle` |
| `current_mode_update` | `Status {permission_mode}` |
| `usage_update {used, size}` | held for the next `TurnResult`'s context window |
| anything else | `Unknown(raw)` (rule 5) |

Rules 2, 3 and 5 hold unchanged: the end is still `Exited`, permission answers go out of order through the same enqueue-only writer, and the unknown is never fatal. Rule 1 (route by origin) was not probed: agy's subagents (`invoke_subagent`, `browser_subagent`) look like tool calls, so for now every agy event is the session's own (`Origin::default()`).

### Permission modes

| Dovetail | agy |
| --- | --- |
| `Manual` | `default` |
| `AcceptEdits` | `accept-edits` |
| `Plan` | `plan` |
| `Auto` | `auto` (the CLI log once said "auto mode is unavailable, falling back to default"; the core reads the mode back from `current_mode_update`) |
| `DontAsk` (side chats) | `default`; the agy driver denies anything outside the session's allow list without asking |

### Core and api

- `dovetail_api::DriverKind { Claude, Agy }` (serde `claude`/`agy`), with `Agent.driver` (default `Claude`, so older rows read unchanged). New api types get no `ts` attributes (task #52).
- `ModelOption.driver: DriverKind`, so the picker groups by driver. agy's options come from its `session/new` answer, or from `agy models` when no agy session has run.
- `Command::SwitchDriver {agent, driver, model, effort}`, which runs the `:fresh` flow with a new driver. `SetModel` for the session's own driver is unchanged.
- `Command::LoadUsage` → `Event::UsageLoaded(Vec<DriverUsage>)`, for the usage menu. `DriverUsage {driver, plan, groups: [{name, models, windows: [{name, used_fraction, resets_at_ms}]}], credits}`. Claude's `get_usage` gives percents (0–100) and agy gives `remaining_fraction`; both become `used_fraction` (0–1), like today's `UsageWindow`.
- `Core.driver: Arc<dyn SessionDriver>` becomes `Core.drivers: Drivers`, a map keyed by `DriverKind`. Tests pass a `FakeDriver` per kind.
- Worktrees: Claude Code creates a slice's worktree through `--worktree` and the `WorktreeCreate` hook. agy has no such flag, so the core calls `worktree::create` itself (the function the hook calls) before launch, and starts agy in that folder.

## Open questions for stage 2

1. **Messages to agy sessions.** Coordinator → slice messages use Claude Code's native `SendMessage`, which an agy session can neither send nor receive. Slice → coordinator already works for any driver, through `coordination_report`, which arrives as a note. Options: (a) stage 2 ships without coordinator → agy-slice messages; (b) a Dovetail tool that delivers a message as a note (the `send_note` path), which is a new tool in the coordination suite. I recommend (b), as its own task after #46.
2. **Slices and `:fresh`.** `:fresh` is coordinator-only today. A driver switch for a slice needs the same flow per slice: a new session in its worktree, seeded with its task notes and the old transcript's path. Is that in #46, or is stage 2 limited to switching the coordinator and starting new slices on agy?
3. **Picker behaviour.** When the user picks another driver's model, should the picker confirm first ("starts a new session, seeded by a handoff")? I would confirm, with handoff as the default and the clean start as the other choice.
4. **The `--cli_dir_name` folder** is per session, under Dovetail's run dir (`<runtime>/<project>/<agent>/agy/`). agy still writes its logs and conversations to `~/.gemini/antigravity-cli/`, as Claude Code writes its own to `~/.claude/`.

## Evidence

### stream-json (`stream.sh`, `input-events.sh`)

- `-p` takes the prompt as its value: `--input-format stream-json --output-format stream-json -p=""`.
- Input lines are `{"event":"user","message":{"role":"user","content":"…"}}`, not Claude Code's `type`.
- Output (`out/s1-basic.ndjson`): `{"event":"init","conversation_id","init":{"model","cwd","tools":[…],"permission_mode"}}`, then `{"event":"step_update","step_update":{"step_index","state":"ACTIVE|DONE","step_type":"user_input|agent_response","text_delta","usage"}}`, then `{"event":"result","result":{"status":"SUCCESS|ERROR","response","num_turns","usage":{input_tokens,output_tokens,thinking_tokens,cache_read_tokens,total_tokens},"finished_reason"}}`.
- Control (`out/input-events.txt`): `control_request` → `error: … "control_request" is not supported yet`; `interrupt`, `cancel`, `control`, `set_model` → "ignoring unsupported stream input message event".

### ACP (`acp.py <name> <scenario>`)

- `initialize` (`out/a1-hello.ndjson`): `agentCapabilities {loadSession, promptCapabilities {image, embeddedContext}, sessionCapabilities {resume, close, load, list, additionalDirectories}}`, `_meta.antigravity.systemPrompt.append: true`, and four `authMethods`.
- `session/new {cwd, mcpServers}` → `{sessionId, models {currentModelId, availableModels[23]}, modes {currentModeId: default, availableModes: default, accept-edits, auto, plan}, configOptions [model (families), thought_level (low…xhigh), mode], _meta.antigravity.tier {name: "Google AI Pro"}}`.
- A turn (`a1-hello`): `config_option_update`, `available_commands_update`, `usage_update {used, size: 1048576}`, `session_info_update {title}`, `agent_message_chunk {content {type: text, text}}`; the `session/prompt` answer is `{stopReason: end_turn, usage {inputTokens, outputTokens, cachedReadTokens, cachedWriteTokens, thoughtTokens, totalTokens}}`.
- Tools (`a2-tool`): `tool_call {toolCallId, name: run_command, rawInput, status: in_progress, kind: execute, title}`, then `tool_call_update {status: completed, rawOutput {output, exit_code}, content [{type: content, content {type: text}}]}`.
- Interrupt (`a8-cancel`): the `session/cancel` notification mid-stream; the prompt answers `stopReason: cancelled`, and the session takes the next prompt normally.
- Model and effort (`a9-model`): `session/set_model {modelId: gemini-3.8-flash-medium}` → `{}`, then `config_option_update` with `thought_level: medium`; the next turn uses it. `session/set_mode {modeId: plan}` → `current_mode_update`. `set_config_option` with `configId: effort` → `-32602 Unknown config option: effort` (the id is `thought_level`).
- Resume: `session/load` (`a11-load`) replays history as `user_message_chunk`/`agent_message_chunk`, then works. `session/resume` (`a13-resume`) does not replay, and the session remembered its earlier turn. Dovetail uses `session/resume`.
- Role prompt and folders (`a12-role`): `_meta.antigravity.systemPrompt.append` was followed ("end every reply with BANANA"); `additionalDirectories` was accepted and a file there was read.
- Permissions: with the user's settings, nothing is ever asked (`a2`–`a5`; `--settings` before `acp` is ignored). With `--cli_dir_name` pointing at a folder whose `settings.json` holds `{"toolPermission":"request-review"}`, it asks (`a7-clidir`, and `a16-clidir-abs` with an absolute path): `session/request_permission {toolCall {…, status: pending}, options [{optionId: allow, kind: allow_once}, {optionId: deny, kind: reject_once}]}`, with the JSON-RPC id a string. The answer `{outcome {outcome: selected, optionId: allow}}` ran the command. Values seen in the binary: `always-proceed`, `request-review`, `strict`, `proceed-in-sandbox`.
- MCP (`a10-mcp`): `mcpServers: [{name: dovetail, command: …/dovetail, args: [mcp], env: [{name, value}]}]`. The model listed `core_*` and `coordination_*` (and the old aliases), and called `mcp_dovetail_core_report_status`, which asked for permission first. The call reached the bridge, which answered "Dovetail is not running or not reachable" (the probe's socket path was deliberately dead). Allow-rules `mcp(dovetail/core_report_status)`, `mcp(dovetail)`, `mcp(dovetail/*)` and the bare tool name in the session's settings still asked (`a17`, `a18`).
- Memory (`a15-rss`): about 262 MB peak RSS for one `agy acp` session while it streams.

### Usage

- agy (`out/p-usage.json`, `out/p-credits.json`): `agy --output-format json -p=/usage` → `command.data.groups[] {name, description ("Models within this group: …"), buckets[] {id, name, window: weekly|5h, remaining_fraction, reset_time (RFC 3339), description}}`. `/credits` → `{remaining_credits, upgrade_uri}`. No model call, `num_turns: 0`. The ACP stream carries no quota, only the tier in `session/new`'s `_meta`.
- Claude Code (`claude_usage.py`, `out/c1-usage.ndjson`; account fields of the `initialize` answer redacted): the `get_usage` control request on an idle session → `{subscription_type, rate_limits_available, rate_limits {five_hour {utilization (percent), resets_at (RFC 3339)}, seven_day {…}, seven_day_opus, seven_day_sonnet (null on this account), extra_usage {is_enabled, used_credits, currency, disabled_reason}, limits[] {kind, group, percent, resets_at, severity, is_active}, seven_day_breakdown}, session {total_cost_usd, model_usage}}`. `get_context_usage` (context categories) and `get_session_cost` (text) also answer with no model call. The per-turn `rate_limit_event` (already parsed) carries utilization as 0–1.

### Switching

Claude Code resumes only its own JSONL sessions (`--resume <id>`). agy resumes only its own conversations (`session/load`/`session/resume <id>`, stored in `~/.gemini/antigravity-cli/conversations/<id>.db`). Neither takes the other's history, so a driver switch is a new session.

### Side effects of the spike

The probes created about 20 short agy conversations, now in the user's agy history. The temporary settings folder `~/.gemini/antigravity-cli/dovetail-probe/` was removed afterwards. The user's agy `settings.json` was not changed.

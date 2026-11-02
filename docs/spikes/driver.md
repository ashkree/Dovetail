# S1: Driver crate and fixtures

Run 2026-11-02 on Claude Code 2.1.316, subscription login (Pro), default model `claude-sonnet-5-5`. All sessions ran headless (`claude -p --input-format stream-json --output-format stream-json --verbose`) in the scratch repo `/tmp/dovetail-spikes/driver/repo`. 17 live sessions in total, all with one-line prompts: 13 on the hand-written layer, 2 per candidate crate. API-equivalent cost was about $0.03 per session. Every `claude` process has exited.

Code is in `spikes/driver/`:

- `raw/`: the hand-written NDJSON layer (`lib.rs`) and one subcommand per check (`main.rs`).
- `sdkrs/`: the `claude-agent-sdk-rs` program.
- `claw/`: the `rusty_claw` program.
- `cfg/`: the WorktreeCreate hook, settings template and appended prompt for check E.
- `fake-claude/claude` and `measure-rss.sh`: check I, measured without live sessions.
- `run.sh`: sets up the scratch repo and runs one check.

## Decision

**Neither crate passes. Use the fallback: a hand-written NDJSON layer.** It passed A to H on its own, needs only `tokio` and `serde_json`, and is the smallest of the three.

Both crates fail C, the check that matters most for the inbox: the permission request is never answered and the turn hangs. Both also fail H. Neither has had a release since February 2026.

## Verdicts

| Check | claude-agent-sdk-rs 0.6.4 | rusty_claw 0.1.0 | Hand-written layer |
| --- | --- | --- | --- |
| A Multi-session | pass | pass | **pass** |
| B Interrupt | not run (out after C) | not run (out after C) | **pass** |
| C Concurrent permissions | **fail** | **fail** | **pass** |
| D Mode switch | not run; cannot express `auto` (see below) | not run; cannot express `auto` | **pass** |
| E Flag passthrough | not run | not run; emits invalid flags | **pass** |
| F Streaming | not run | not run | **pass** |
| G Resume, including in a worktree | not run | not run | **pass** |
| H Tolerance | **fail** | **fail** | **pass** |
| I Cost | 64 crates, +0.36 MB, 4.9 MB RSS | 53 crates, +0.28 MB, 4.6 MB RSS | 18 crates, base, 4.3 MB RSS |

"Not run" means the crate had already failed C. Under the decision rule it could no longer win, so I did not spend live sessions on it. The notes in its section come from reading its source.

### claude-agent-sdk-rs 0.6.4

- **C fails.** The `can_use_tool` option is declared in `ClaudeAgentOptions` but never wired. The control loop handles only `hook_callback` and `mcp_message`. A `can_use_tool` request gets `eprintln!("Error handling control request: Unsupported control request subtype: can_use_tool")` and no response. Live run: the callback was called 0 times, and session 2 hung until the 60 s timeout. Upstream PR #23 ("Wire up can_use_tool option") is still open.
- **H fails.** `Message` is a closed enum. `rate_limit_event`, which arrives on every turn, becomes `Err(MessageParseError)`, and the raw JSON is dropped on purpose ("Don't include original data"). Upstream PR #25 is open for this.
- **D, by reading the source:** `PermissionMode` has no `Auto` or `Manual` variant, so `set_permission_mode(auto)` cannot be sent.
- **Other problems:**
  - It writes to stderr with `eprintln!`, which would corrupt the TUI.
  - It runs `claude --version` and `which` on every connect.
  - It does not drain stderr unless a callback is set. Upstream #31 reports a crash on stderr output.
  - Upstream #28 and #29 report stream-death and silent-hang bugs, both still open.
- **A passes:** two clients, no cross-talk.

### rusty_claw 0.1.0

- **C fails.** The CLI sends the tool input as `input`, but `IncomingControlRequest::CanUseTool` requires a field named `tool_input`. Every real request therefore fails to deserialize, is logged at `warn` and dropped. Live run: the handler was called 0 times and the turn hung for 150 s.
- **C would fail even with the field fixed.** The router `await`s each control request inline, so a held request blocks every later request and all stream output for that session. Its reply also has the wrong shape: it sends `{"allowed": true}` instead of `{"behavior": "allow", ...}`.
- **The handler loses context.** It receives only `(tool_name, tool_input)`, so `agent_id`, `tool_use_id`, the reason and the "always allow" suggestions are lost.
- **H fails.** A closed `Message` enum turns `system/session_title_changed`, which appears whenever `--name` is set, into an error.
- **It drops events between turns.** Lines arriving while no turn stream is installed are discarded ("If no sender is installed (between turns), messages are discarded"). Background-subagent events and the second `result` arrive exactly then.
- **E, by reading the source:** it emits flags that do not exist (`--permission-prompt-tool-name`, `--session-name`, `--system-prompt-preset`, `--beta`). It sends `--setting-sources ""` by default. It never sends `--permission-prompt-tool stdio`. `PermissionMode` invents `allow`, `ask`, `deny` and `custom`, and has no `auto`.
- **Maintenance:** one release, 33 downloads, no issue tracker activity.

## Hand-written layer: evidence per check

The layer is about 200 lines in `spikes/driver/raw/src/lib.rs`. It spawns `claude` and keeps every stdout line as a raw `serde_json::Value`. It routes `control_response` lines to waiting requests by `request_id`, and surfaces `can_use_tool` requests to the caller, who may answer them from any task in any order.

| Check | Result |
| --- | --- |
| A | Two sessions with chosen ids ran concurrently. Every line carried its own `session_id`; no foreign ids. Both returned `result/success` in about 2.5 s and exited 0. |
| B | `sleep 60` started; the `interrupt` control request was answered with `{"still_queued": []}`. `result/error_during_execution` (`terminal_reason: "aborted_streaming"`) arrived 0.0 s later. The same pid then answered "after". No `sleep` process was left behind. |
| C | Manual mode, two parallel `Read`s outside the working directory. Both `can_use_tool` requests arrived at 3.0 s. I answered #2 at once, and its tool result arrived at 3.0 s. #1 was held, answered at 63.0 s, and its result came back at 63.0 s. Final result: "uno dos". |
| D | **auto:** Bash `python3 -c` ran with 0 prompts. **`set_permission_mode acceptEdits`:** answered `{"mode": "acceptEdits"}`; then Write had 0 prompts and Bash had 1. **`set_permission_mode manual`:** answered `{"mode": "default"}`; then Write had 1 prompt. Same process throughout. Each switch also emits `system/status` with `permissionMode`. |
| E | Every flag reached the process. **`--settings`:** the WorktreeCreate hook ran. **`--worktree spike-e`:** `init.cwd` is the hook's worktree. **`--mcp-config`:** `init.mcp_servers` shows `spike: connected`, and the `mcp__spike__*` tools are listed. **`--append-system-prompt-file`:** the reply was the codeword from the file. The flag is hidden from `--help` but works. **`--session-id`:** `init.session_id` equals the chosen id. **`--name`:** shows up as `system/session_title_changed {title}`. **`--include-partial-messages`:** `stream_event` lines appear. **`--permission-prompt-tool stdio`:** checks C and D. **Env:** the hook saw `DOVETAIL_SPIKE_MARK`, and `CLAUDE_CODE_ENABLE_TODO_TOOLS=1` added the `Task*` tools. |
| F | 21 `text_delta` stream events came before the final `assistant` message. The first was at index 4; the assistant message was at index 25. |
| G | **Plain:** `--resume <id>` with the same id answered "alpha", the earlier reply. **Worktree:** resumed with `cwd` set to the main checkout; `init.cwd` was the worktree, and the session still knew the codeword. The WorktreeCreate hook did not run again on resume. |
| H | No crash. Every line stays raw JSON. The run saw these types: `rate_limit_event`, `system/{init,status,session_title_changed,thinking_tokens,permission_denied,task_started,task_progress,task_updated,task_notification,background_tasks_changed}`, `stream_event`, and `tool_result_meta` on `user` lines. |
| I | Dependencies: 18 crates, `tokio` and `serde_json` only. Stripped binary 1.39 MB. Peak RSS 4.3 MB driving two sessions. The crate programs measured 4.6 to 4.9 MB. |

## Re-checks on 2.1.316

- **`permissionMode: auto` on Pro:** yes. Init reported `"permissionMode": "auto"` with model `claude-sonnet-5-5`, and the classifier cleared Bash (`tool_result_meta ... "reason_type": "classifier"`).
- **Double `result` with a background subagent:** confirmed, and there can be more than two. The run produced three:
  - #1 "launched" (`result_index` 0);
  - #2 the summary after the subagent finished (`result_index` 1);
  - #3 empty, `num_turns: 0` (`result_index` 2), after the subagent's own Bash background task notified.

  Each was preceded by a fresh `system/init`. `result_index` counts them, so use it rather than counting yourself. Done still means the process exited, or no background tasks are left (`background_tasks_changed`).
- **`--session-id` sets the id:** yes, in init and on every line. It also survives resume. Dovetail can choose ids.
- **Context window:** reported in two places, with the same value on every result:
  - `result.modelUsage.<model>.contextWindow` (1000000 for `claude-sonnet-5-5`), plus `maxOutputTokens` (128000);
  - interrupted results instead carry `model_usage_since_last_result`, also with `contextWindow`.

  So M13 can read the window from the stream; no model table is needed (plan B29).
- **`--permission-prompt-tool stdio` is still required.** Without it, `--permission-prompts` defaults to `host`, but a manual-mode request was auto-denied: a `system/permission_denied` event was emitted and no `can_use_tool` reached stdin (fixture `permission_denied_no_prompt_tool`). With it, requests behave as Spike 3 recorded. Request fields: `tool_name`, `display_name`, `input`, `tool_use_id`, `description`, `decision_reason`, `decision_reason_type`, `permission_suggestions` (addRules with a destination). A request from a subagent also carries `agent_id` (fixture `subagent_permission`). `--permission-prompts none` would deny all prompts; Dovetail never wants that.

## Protocol facts for the driver (M6)

- **Launch:** `-p --input-format stream-json --output-format stream-json --verbose --permission-prompt-tool stdio` plus the per-session flags. The process stays alive between turns until stdin closes, then exits 0. Every run in this spike exited cleanly this way.
- **User message (stdin):** `{"type":"user","message":{"role":"user","content":"..."},"parent_tool_use_id":null}`. No `initialize` control request is needed; I sent none.
- **Outbound control:** `{"type":"control_request","request_id":"<ours>","request":{"subtype":"interrupt"|"set_permission_mode","mode":...}}`. The reply is `{"type":"control_response","response":{"subtype":"success","request_id":"<ours>","response":{...}}}`. The mode the reply names for `manual` is `default`.
- **Permission answer:** `{"type":"control_response","response":{"subtype":"success","request_id":"<theirs>","response":{"behavior":"allow","updatedInput":<input>}}}`. Only allow was exercised; deny and `updatedPermissions` are listed under Open.
- **`result` fields worth keeping:** `subtype` (`success` or `error_during_execution`), `result_index`, `terminal_reason`, `stop_reason`, `modelUsage`, `permission_denials`, `subagent_stats`, `queued_turn_count`.
- **Other events:**
  - `rate_limit_event` arrives on every turn.
  - A long Bash inside a subagent was moved to the background (`task_started` with `task_type: "local_bash"`), so `task_*` events are not only about subagents.
  - Messages from the subagent's report arrive via its `SubagentHandback` tool.

## Fixtures

All are in `crates/dovetail-driver/tests/fixtures/`, one JSON object per line: `{"t": <ms since spawn>, "dir": "in"|"out"|"err", "msg": <the line>}`. Both directions are kept, so the fake driver can replay timing and check what the core sent, such as permission answers and control requests. Home paths are rewritten to `/home/user`. No emails, tokens or keys appeared; `apiKeySource` is `"none"`.

| Fixture | Content |
| --- | --- |
| `plain_turn` | One turn, auto mode, chosen session id, `--name` |
| `partial_stream` | One turn with `--include-partial-messages` |
| `subagent_background` | Background subagent with `task_*` events and three `result`s |
| `double_result` | Same recording as `subagent_background`: one live run covers both. Tests for the two assert different things. |
| `concurrent_permissions` | Two `can_use_tool` requests; #2 answered at once, #1 held for 60 s |
| `todo_tools` | `ToolSearch`, then `TaskCreate` ×2 and `TaskUpdate` |
| `resume` | `--resume` of the `plain_turn` session |

Extras, recorded during the same runs:

| Fixture | Content |
| --- | --- |
| `interrupt` | Check B |
| `mode_switch` | Check D |
| `flags` | Check E, inside a worktree |
| `resume_worktree` | Check G, worktree session resumed from the main checkout |
| `subagent_permission` | Foreground subagent's request, with `agent_id` |
| `permission_denied_no_prompt_tool` | No `--permission-prompt-tool` flag |

## Consequences for the plan

- `dovetail-driver` implements the NDJSON layer itself, as about one module: process spawn, line reader, control-request correlation, and a permission-request channel. The dependency row "`claude-agent-sdk-rs` or `rusty_claw`" becomes "none: `tokio` and `serde_json`".
- **Keep each line's raw JSON next to the typed parse.** Parse lazily into typed events and keep unknown types as `Raw(Value)`.
- **Never block the stdout reader on a permission answer.** Requests go out over a channel and answers come back through a shared stdin writer.
- **`--permission-prompt-tool stdio` belongs in the launch list.** Plan A13 is resolved: use it.

## Open

- **Deny and "always allow" answers.** `behavior: "deny"` and replies carrying `updatedPermissions` (the "always allow" rules) were not exercised. Only allow was used. Check both in M7 with the fake driver plus one live run.
- **Auto mode escalations.** No auto-mode classifier escalation or classifier denial was observed. Same as the digest's untested note.
- **Leftover scratch worktree.** The scratch worktree `/tmp/dovetail-spikes/driver/worktrees/spike-e` and branch `dovetail/spike-e` are left in the scratch repo so G can be rerun. They are not in any real repo.

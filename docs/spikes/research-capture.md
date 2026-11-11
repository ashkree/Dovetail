# Spike S7: research capture fidelity

Run 2026-11-11 on Claude Code 2.1.316, subscription login, coordinator and subagents on `claude-haiku-5-5` (Haiku spawned subagents without trouble, so Sonnet was not needed). Five live sessions, one prompt each, in the scratch repo `/tmp/dovetail-spikes/research/repo` (one file, `notes.txt`; pages fetched: `https://example.com`). Launch as Dovetail does: `-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-mode auto --permission-prompt-tool stdio`, with `WebFetch,WebSearch,Read,Agent` allowed. Reported cost `total_cost_usd`: 0.0138 + 0.0071 + 0.0087 + 0.0185 + 0.0054 = **about $0.054** (all sessions; subscription, so API-equivalent). Every `claude` process this spike started has exited.

Code: `spikes/research-capture/{record,summarise,inspect,sanitise}.py` (outside the workspace). Fixtures (sanitised: home path, thinking signatures and the `/tmp/claude-<uid>` path replaced, no email, key or account id): `crates/dovetail-driver/tests/fixtures/research_round_{foreground,background}.ndjson` (the two required), plus `research_websearch.ndjson` and `research_mcp_subagent.ndjson` as evidence for (d) and (e). `crates/dovetail-driver/tests/research_fixtures.rs` replays the first two through `FakeDriver`.

## Answers

| | Answer | Evidence |
| --- | --- | --- |
| (a) subagent `WebFetch` and `Read` inputs in the stream, tagged? | **Yes, foreground and background.** Full `assistant` lines with `tool_use` blocks, `parent_tool_use_id` equal to the coordinator's `Agent` call id (which is also `task_started.tool_use_id`). `WebFetch` input is `{url, prompt}`, where `prompt` is a question the subagent wrote, not the user's. `Read` input is `{file_path}`. The subagent also calls `ToolSearch` (to load `WebFetch`/`WebSearch`): not a source. | `research_round_foreground`, `research_round_background` |
| (b) `tool_result` present, complete, raw or summary? | **Present for every call, in the stream, tagged with the same parent.** `Read`: the file with line numbers (`1\t...`), complete for the 2-line file; truncation of long files is **not tested**. **`WebFetch`: a model summary, never the page.** The result was `Example Domain`, 14 characters, the answer to the subagent's own `prompt` input. So what is cached is query-dependent. | same; `user` lines `tool_result` |
| (c) full final report, where? | **Both foreground and background, in three identical places**, because Claude Code 2.1.316 subagents end with a `SubagentHandback` tool call: (1) the subagent's `SubagentHandback` `tool_use` input `message`; (2) `task_notification.handback_report.text` (with `handback: "send"`); (3) foreground only, on the coordinator's `Agent` `tool_result` line as `tool_use_result.handbackReport.text` (the `content` text of that result says "delivered to you as a message ... not repeated here", so the `content` has **no report**). `task_notification.summary` is that boilerplate sentence, not a summary of the work. The background launch `tool_result` carries `outputFile`. The output file is complete (see below). | `research_round_foreground` (Agent result, notification), `research_round_background` |
| (d) `WebSearch` results list URLs? | **Yes.** The subagent's `tool_result` content is one string: `Web search results for query: "..."`, then `Links: [{"title","url"}, ...]` (about 10 entries, URLs only as titles and links, no snippets), then a model-written prose synthesis of the results, then a harness line ("REMINDER: You MUST include the sources..."). The prose can carry text from third-party pages (here a result contained text addressed to an AI, which the subagent flagged and ignored). Parse the `Links:` JSON array for source rows; treat the rest as untrusted text. `tool_use_result` is null on the user line. | `research_websearch` |
| (e) subagent calling `mcp__dovetail__*` | **Same as any tool.** `assistant` `tool_use` named `mcp__dovetail__who`, tagged with the parent id, followed by a tagged `tool_result` (`pong `). This run had the harness auto-background the call (`is_backgrounded: true`), and the recording stops at the first `result`, so the closing notification is not in the fixture; the tool call and result are. | `research_mcp_subagent` |

## The output file

`task_notification.output_file` (`/tmp/claude-<uid>/<project>/<session>/tasks/<agent>.output`) is a **symlink to the subagent's transcript JSONL** at `~/.claude/projects/<project>/<session>/subagents/agent-<agent>.jsonl` (about 335 KB for a 4-call subagent). It is complete: the prompt, every `tool_use` and `tool_result` (including the same summarised `WebFetch` result), and the `SubagentHandback` call with the report. It is also noisy: `attachment`, `api-request-blob` and `api-request-shape` lines carry the whole request context, **including the user's email address and connector lists**. Never copy it into a findings file or a repo. Read it only for `assistant`/`user` lines with `tool_use`/`tool_result`/`text` blocks. It holds nothing the stream does not, so it is a fallback for a missed stream (restart, dropped lines), not a primary source. The `/tmp` symlink may vanish; the target under `~/.claude` persists.

## Other facts that matter for M17

- **A background subagent keeps streaming after the coordinator's turn `result`** (fixture: `result` "launched", then more subagent lines, then `task_notification`, then a second `init` and a second turn). A round or finding must close on `task_notification`/`task_updated`, never on a turn result. A foreground subagent ends before the coordinator's next message.
- The spec's `Notification { output_file, summary }` is right, but **`summary` is not the report** when a handback was used; the report is `handback_report.text`. `TaskEvent.raw` already keeps it (`session/subagent.rs::summary_of` reads it).
- A `Started` event arrives for a foreground call too (`is_backgrounded: false`) and the `Agent` call can be turned into a background task by the harness (the `mcp` run).
- Subagent lines also include `thinking` and `user` text blocks (the subagent's prompt, tagged); ignore them for sources.

## Conclusion

Each finding body comes from the subagent's `SubagentHandback` message, read from `task_notification.handback_report.text` (identical to the `tool_use` input and, for foreground, to `Agent` `tool_use_result.handbackReport.text`); when a subagent ends without a handback, fall back to the `Agent` result text or the concatenated assistant text of the subagent, and as a last resort the output file's transcript. A Source row comes from the tool call, and its "cached copy" is **what the tool returned to the agent, not an archive of the page**: for `WebFetch` that is a short model answer to a question the subagent chose (a few hundred characters at most for a simple question; here 14), for `Read` the file's lines (numbered), for `WebSearch` a list of titles and URLs plus a model synthesis. The UI must say "what the agent saw", and the raw page bytes are only available if Dovetail fetches the URL itself, which the design does not do.

## Fallbacks M17 must use

1. **Report:** prefer `handback_report.text` from the notification; if absent, `tool_use_result.handbackReport.text` on the foreground `Agent` result; then the subagent's `SubagentHandback` input; then, for a subagent without a handback, the `Agent` result content (not exercised here: every subagent in 2.1.316 used handback); finally parse `output_file`.
2. **Never use `task_notification.summary` as the body** when `handback` is `send`.
3. **WebFetch cache:** store the summary, label it so, and also store the `prompt` input beside it (the cache is meaningless without the question).
4. **WebSearch:** extract URLs from the `Links:` JSON; keep the prose as untrusted text.
5. **Round closing:** on `task_notification`/`task_updated`, not on `result`.

## Not verified

- `Read` truncation on a large file (the `Read` tool documents a 2000-line default; the stream was not tested with one).
- A subagent that ends without `SubagentHandback` (where the report would be in the `Agent` result `content`).
- `WebFetch` of a large page, a redirect, or a failing URL (`is_error` shape).
- The `mcp` subagent's closing notification (recording stopped at the first `result`).
- Cost of subagents separately from the coordinator (`total_cost_usd` is reported once per turn and is assumed to include them).
- Behaviour on other versions than 2.1.316.

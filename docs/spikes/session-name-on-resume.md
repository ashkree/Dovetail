# Spike: does a resumed session take a new name?

Gates ADR 0022, point 5. Run 2026-11-16 with Claude Code 2.1.325, in a scratch git repo under the session scratchpad, with the cheapest model and a one-word prompt.

## Procedure

```bash
claude -p "Reply with exactly: ok" --session-id "$ID" --name scratch.first \
  --model haiku --output-format stream-json --verbose
claude -p "Reply with exactly: ok" --resume "$ID" --name scratch.second \
  --model haiku --output-format stream-json --verbose
```

## Result: pass

| Run | Exit | Name in the stream | Session id |
| --- | --- | --- | --- |
| start, `--name scratch.first` | 0 | `system` / `session_title_changed`, `title: "scratch.first"` | `$ID` |
| resume, `--name scratch.second` | 0 | `system` / `session_title_changed`, `title: "scratch.second"` | `$ID` (same) |

A resume accepts a different `--name`: the session keeps its id and conversation and reports the new title. So sessions started before ADR 0022 take the `<project>.<name>` form on their next start, with no special case.

## Not covered

That another session's `SendMessage` reaches the session under the new name after a resume. The name is what Claude Code's messaging addresses, and the title change is reported, but no second session was started to send to it. The first real slice start after this change will show it; the fallback is in the core already (`agent_by_name` accepts the bare name too).

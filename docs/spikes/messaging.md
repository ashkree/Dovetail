# S2 — Inbound messaging

Run 2026-11-01 on Claude Code 2.1.316, model `haiku`, scratch repo `/tmp/dovetail-s2`. Two headless sessions, `alpha` (the receiver) and `beta` (the sender), each launched with:

```
claude -p --input-format stream-json --output-format stream-json --verbose --name <n> --model haiku \
  --allowedTools "ListAgents,SendMessage[,Bash(sleep *)]" --settings '{"crossSessionInbound":"accept"}'
```

stdin was a FIFO held open, so A stays idle. Launcher: `messaging-evidence/start.sh`. Permission mode was left at the default; I did not use `bypassPermissions`. No blocking env vars (`DISABLE_TELEMETRY` and similar) were set.

Raw streams, unedited, in `docs/spikes/messaging-evidence/`:

| File | Run |
| --- | --- |
| `run1-alpha-idle.ndjson`, `run1-beta-sender.ndjson` | Steps 1 (idle A, no replay flag) |
| `run2-alpha-replay.ndjson`, `run2-beta-sender.ndjson` | Steps 2 and 3 (A started with `--replay-user-messages`) |

Caveat: the streams include the full `init` tool list and a `ListAgents` result that names the user's own interactive session (`dovetail-37`). Nothing secret, but scrub before any wider sharing.

## Answers

**a. Does A's stream show the inbound message, with the sender identifiable? Yes.** Without any flag, an idle A emits no `user` event for it, but the `result` that ends the resulting turn carries an `origin` object:

```json
"origin": {"kind":"peer","from":"uds:/run/user/1000/cc-socks/469606.sock","verifiedPeerPid":469606,
           "verifiedPeerProcStart":"1143268","msg_id":"cfdb7e08-…","name":"beta",
           "fromMode":"prompting","body":"ping-1 from beta"}
```

The turn is bracketed by `command_lifecycle` events (`started`, then `completed`), which carry only a `command_uuid`. The sender is identified by `name`, the socket, and a kernel-verified PID. `msg_id` equals the `msg_id` in the sender's `SendMessage` tool result, so the two streams join exactly.

With `--replay-user-messages`, a `user` event with `isReplay: true` and the same `origin` appears at the start of the turn, so the message shows up before any assistant output. Its text is wrapped:

```
Another Claude session sent a message:
<cross-session-message from="uds:/run/user/1000/cc-socks/….sock" from-name="beta" from-mod…>ping-2-midturn</cross-session-message>
```

For the timeline, use the `origin` object. It is on the `user` event with replay and on the `result` without it. Use `--replay-user-messages` if the timeline should show the message when it arrives, not when the turn ends.

**b. Does an idle A take a turn with no stdin input? Yes.** Run 1: A was idle after its first `result`. About 40 s later beta's `SendMessage` arrived and A started a turn by itself (`command_lifecycle started`, `init`, assistant reply, `result`, `completed`). Nothing was written to A's stdin. The relay fallback is not needed for waking.

**c. Is it delivered mid-turn? It is queued, not injected.** Run 2: A was generating a long essay (turn 18:34:09 to 18:34:28). Beta's `SendMessage` returned at 18:34:12.9. A's turn finished untouched, and the message started a new turn at 18:34:28.6, about 16 s after sending. Order is preserved. The message never appears inside the running turn. A first attempt (the `sleep 40` test) was not a real mid-turn test, because the harness backgrounded the sleep and A's turn had already ended. The message still started its own turn while the background task ran, and the task-notification turn followed it.

**d. Does `--name` set the name shown by `ListAgents`? Yes.** Beta's `ListAgents` listed `alpha [8038ff]  ·  interactive  ·  idle  ·  started 23s ago`, and `SendMessage` to `"alpha"` resolved. Beta's own header read "This session is beta [1f5890]". The `--name` flag also emits a `system/session_title_changed` event as the first line of the stream.

## Other observations

- The bracketed id in `ListAgents` (`8038ff`) is not a prefix of the stream's `session_id` (`9437ce64-…`). Don't try to match them. Match on name, or on `msg_id` and the socket path. Dovetail assigns the names, so that is enough.
- Headless sessions are reported as `interactive`. `ListAgents` also lists unrelated sessions on the machine, so the coordinator will see them. Filter by the names Dovetail assigned.
- The peer's `fromMode` ("prompting") is in `origin`. The sender's result says delivery goes to the receiver's inbox and that a "[Cross-session delivery notice]" follows if the receiver refuses or holds it. That notice never appeared with `accept`.
- Haiku receivers treat the message as information and do not act or reply unless it asks. A role prompt has to say that slices act on coordinator messages.
- `SendMessage` is a deferred tool in this build. Beta had to call `ToolSearch select:SendMessage` first, which adds one turn. List it in `--allowedTools` and mention it in role prompts.
- The `SendMessage` input schema carries `to`, `message`, `summary`, and echoed `type`, `recipient`, `recipient_kind`, `content` fields. Read `to` and `message`, and tolerate the rest.

## Verdict

**M8 does not need the relay.** Questions a, b and d pass, and c needs no action. An idle slice wakes on its own, so the coordinator can direct slices, which keeps self-hosting unblocked.

For the message timeline:
- Build it from `SendMessage` tool calls in the sender's stream and `origin` objects in the receiver's stream, joined on `msg_id`.
- Launch sessions with `--replay-user-messages`, so receipt is visible at the start of the turn.
- Expect queueing: a message sent to a busy slice waits for the end of its turn.

## Cost

About 4 live headless sessions, roughly 7 short turns in total. The reported cost was about $0.004 for a typical turn, so well under $0.05 overall. Sessions: alpha and beta in run 1, then alpha (with replay) and beta in run 2. All were killed afterwards, with none left running.

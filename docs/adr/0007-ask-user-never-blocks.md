# `ask_user` never blocks

Refines the `ask_user` behaviour in the design (Decision 7a–c's "blocked flag" and Decision 25's "optionally blocking"). Task #2, decision #4; approved by the user in the coordinator session of 2026-11-09.

A blocking `ask_user` (`blocking: true`) held the tool call open until the user answered from Needs you. While it was held, the session's turn could not end, so chat messages the user typed to that session queued silently behind it. The user reads questions more easily in the chat than in the inbox, and expects a chat reply to reach the agent at once.

**Decision:** `ask_user` always returns at once ("Question sent; the answer will arrive as a new message"). The `blocking` parameter is gone from the tool schema; an old caller that still passes it is not refused, the field is ignored.

**Consequences:**

- The question is still an inbox item (`InboxKind::Question { text }`, no `blocking` field), and it is also shown in the agent's conversation as a notice.
- Answering it from the inbox sends the agent a normal user message, as the non-blocking path already did.
- A chat message to a session with open questions goes through immediately and resolves those items as answered with the message's text.
- Questions no longer put the agent in "needs you", and no longer sort above permission requests. At restart a waiting question no longer stands in for a cut-off turn's report, since the agent kept working after asking.
- The `blocking` column of `inbox_items` stays for old rows; new rows write 0. Old stored payloads with `"blocking"` still parse.
- The spike result that long holds work (`docs/spikes/mcp.md`) stays true but is no longer relied on.

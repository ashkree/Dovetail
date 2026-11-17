# "Need you" counts only items that block an agent; the coordinator makes no finished-turn item

**Status: Accepted.** Decided by the user on 2026-11-17 (`docs/plan/smoke-test.md`, decision 4).
Built: see `docs/plan/smoke-decisions.md`.

Amends ADR 0001 (one finished-turn item per agent).

## Why

In the first live run `⚠ 2 need you` showed with nothing to answer: both items were finished
turns. With several slices the count never drops to zero, the user stops reading it, and a
permission request that blocks a slice looks like the idle items around it.

## Decision

1. The count in the bottom bar, on Home and in `Totals.needs_you` covers items that block an
   agent or wait for an answer: permission requests, questions, criterion reviews, errors.
2. Finished turns stay in the inbox, one per agent as ADR 0001 says, listed after the blocking
   items and dimmed. They are not counted.
3. The coordinator raises no finished-turn item: its chat is always docked, so its replies are
   already in view.

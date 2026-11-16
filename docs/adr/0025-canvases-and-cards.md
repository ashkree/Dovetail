# Canvases and cards live in `state.db`; a card names its target by kind and id

**Status: Accepted.** Approved by the user in the session of 2026-11-15. Built: see `docs/plan/canvas-core.md` (state migration `0018_canvases.sql`, the `canvas` tool suite, the GUI reading cards from the core).

Refines section 10 of `docs/design-v2.md` (the `canvases`, `cards` and `card_links` tables) and settles how a card's reference stays valid. Framework layouts and the proposed "decision rounds" are left to the canvas milestone.

## Decision

**1. Canvases and cards are core state, in the project's `state.db`.** They travel like everything else: in the `Snapshot`, changed by `Command`s, announced by `Event`s. The frontend keeps only what belongs to one window (pan, zoom, selection). An agent adds a card through an MCP tool handled in the core, so the user's canvas and the agent's see the same rows.

**2. Tables.**

```
 efforts ─┬─◄ canvases (id, effort_id, title, framework)
          └─◄ cards    (id, effort_id, canvas_id, kind, x, y,
                        ref_kind, ref_id, text, added_by, created_ms)
                 └─◄ card_links (from_card, to_card, label)
```

Three changes from the design's sketch:

- **`ref_kind` beside `ref_id`, and `ref_id` is text.** The design has `ref_id` alone. Its targets live in five tables, and maps have text ids while the rest have numbers. `ref_kind` is one of `finding`, `source`, `decision`, `map`, `task`.
- **A card belongs to an effort; its canvas may be empty.** A card with no canvas is in the **incoming** strip. Research output arrives there; placing it sets `canvas_id`, `x` and `y`. No separate incoming table.
- **`added_by`** is `user` or an agent id.

**3. A card points; it does not copy.** A card with a reference stores no text of its own. The core fills in its title when it builds the snapshot (the finding's label, the task's title) and loads the body on request, as research content loads today. Idea and question cards exist nowhere else, so they keep their words in `text`.

Example rows:

| id | effort | canvas | kind | x, y | ref_kind | ref_id | text |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 90 | 7 | 4 | finding | 320, 180 | finding | 41 | |
| 91 | 7 | | source | | source | 12 | |
| 92 | 7 | 4 | idea | 40, 60 | | | Ask Mollie about EU-only pricing |

Card 91 is in incoming. Card 92 has no reference.

**4. A reference stays valid because its target is never deleted.** That is already how the five target tables behave: findings, sources, decisions and map versions are only ever added; a task is archived, not removed. This ADR makes it a rule: **a row a card can point at is never deleted.** The core checks that the target exists when the card is created, in the same database request. There is no foreign key, because one column points at five tables (the same choice as `decision_evidence`, migration 0014).

A card whose target is missing anyway (a damaged database) shows as "missing" and does not stop the snapshot from loading.

**5. Agents add and link; they never move or delete.** The agent's tools are "add a card to incoming" and "link two cards". There is no tool that sets a position or removes a card, so "the agent never moves the user's cards" holds by construction, not by a check.

**6. A chat reply becomes an idea card by copying its text.** It is a new thought the user chose to keep, and it should survive whatever happens to the conversation.

**7. A map on a canvas** is a card with `ref_kind = map`. It shows the map's latest version. The map model and the canvas model stay separate, as the design says.

## Alternatives rejected

- **Keep canvases in the frontend (browser storage or a JSON file).** Agents could not add cards, and a second window would not see them.
- **One reference column per target table** (`finding_id`, `source_id`, …) with real foreign keys. Five mostly empty columns and a migration for every new card kind.
- **Copy the target's text into the card.** The card then goes stale when a finding's body is corrected, and research text is stored twice.
- **Cascade-delete cards with their target.** Cards would vanish from a canvas the user arranged. Never deleting targets is simpler and already true.
- **A separate `incoming` table.** The same columns as `cards` minus the position.
- **Let agents position cards within a framework.** Convenient for the question tree, but it is exactly the "agent moved my cards" the design rules out. The frontend can lay out incoming cards by their links instead.

## Consequences

- Unfiled findings (`unfiled_findings`) are deleted when dropped, so a card can point at a finding only once it is filed under a task.
- The snapshot grows by every card of the open project. Titles only; bodies load on request.
- Open point 6 of the design (how stored findings map onto finding cards) is answered: one card row per finding, created in incoming when the finding is filed.

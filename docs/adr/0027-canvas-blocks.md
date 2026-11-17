# One canvas per effort; layouts are blocks on it, and agents may add blocks in empty space

**Status: Accepted.** Decided by the user on 2026-11-17 (`docs/plan/smoke-test.md`, decisions 2
and 8). Built: see `docs/plan/smoke-decisions.md`.

Amends `docs/design-v2.md` §6 ("Any number of canvases per effort. Each starts from a
framework") and ADR 0025 (its `canvases` table, and rule 5, "Agents add and link; they never
move or delete").

## Why

The first live run showed the cost of choosing a framework first: until the user picked one,
an effort had no canvas, so cards an agent added had nowhere visible to land, and the user had
to choose a structure before seeing what arrived.

## Decision

**1. Every effort has one canvas, from the start.** It needs no row: the canvas is the effort's
blocks and cards. There is no framework picker.

**2. A layout is a block on the canvas.** A block is a framed region with a layout and a title,
at a position on the effort's canvas. Layouts: **question tree**, **options compare**, **free
area** (later **decision rounds**). The table `canvases` becomes `blocks` (adding `x`, `y`);
`cards.canvas_id` becomes `block_id`; `free_board` becomes `free_area`.

```
┌ Canvas · Checkout ───────────────────────────────────────────┐
│ ┌ question tree ─────────┐   ┌ options compare ────────────┐ │
│ │ Q: which provider?     │   │ Stripe     Paddle    Adyen  │ │
│ │  ├ Q: fees? [finding]  │   │ [card]     [card]           │ │
│ └────────────────────────┘   └─────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
incoming · 1   [source] VAT rules 2026
```

**3. Agents add blocks in empty space, and cards into blocks.** Rule 5 of ADR 0025 becomes:
*an agent adds and links; it never moves, resizes or deletes anything already on the canvas.*
The new tool `canvas_add_block` makes a block, which the core places right of everything on the
effort's canvas. `canvas_add_card` may name a block and a parent card (`under`): the core puts
the card in the next free spot under its parent, or in the block's next free cell. The core
does all placement, so "never moves" still holds by construction: no tool takes a position.

**4. New agent cards are marked until seen.** A card an agent puts into a block is unseen
(`cards.seen = 0`) until the user selects it. A card with no block still goes to the incoming
strip, which always shows under the canvas.

**5. The user makes blocks too**, with `b` then `1 2 3`; the core places them the same way.

**6. Not now:** converting a block from one layout to another (make a new block and move the
cards), moving a whole block, and modelled criteria rows in options compare (a card goes in its
option's column).

## Alternatives rejected

- **Keep "pick a framework first", but show the incoming strip under the picker.** Fixes the
  invisible cards but keeps the up-front choice the user did not want.
- **One canvas for the whole project, switching effort pans the camera.** Raised by the user and
  dropped: the chat tag and "where was I" follow one effort.
- **Agents position cards.** Still rejected: the core places them, from the block and parent.

# Build: the smoke test's eight decisions

Builds the answers in `docs/plan/smoke-test.md` §Decisions (settled with the user on
2026-11-17; the user said to start the build without a separate plan review). Branch
`claude/smoke-decisions`. One PR, reviewed with `code-review`, merged.

New ADRs: 0027 (one canvas per effort, layouts as blocks; amends design v2 §6 and ADR 0025),
0028 ("need you" counts blocking items only; amends ADR 0001).

## Steps

Each step ends with a status line here, then a commit and push.

1. **Viewer follows the picked slice** (decision 1). `ui.viewerAgent` (an agent id or null):
   `Enter` on a slice row in the tree sets it and opens the viewer; `viewerAgent()` in
   `gui/src/keys/actions.ts` returns it when it is still a slice with a preview, else the focused
   effort's slice as today. The chat tag is not touched.
2. **No user-started slices in the GUI** (decision 3). Drop `Space s n`, the `newSlice` action
   and its form (repo picker) from the GUI; the core's `NewSlice` stays for the frozen TUI. Doc
   rows go.
3. **"Need you" counts blocking items** (decision 4, ADR 0028). Core: `Totals.needs_you` and Home's `ProjectCounts.needs_you` count everything but
   finished turns. GUI: the bottom bar and Home read the blocking count; the inbox lists
   blocking items first and finished turns dimmed below them, and leaves out the coordinator's
   finished turn (the core keeps it for the frozen TUI).
4. **Permission mode per agent, `Shift-Tab`** (decision 5). `Shift-Tab` in the chat (normal
   and insert mode) cycles the coordinator; in the tree, the agent under the cursor (the
   coordinator when the row is not an agent). `Space s m` uses the same rule from any zone. The
   bottom bar shows `<agent> · <mode>` for that target. Canvas and viewer keep their
   `Tab`/`Shift-Tab`.
5. **Home** (decision 6). After `n` the new project is selected (`awaitProject`, cleared like
   `awaitEffort`). The add-repo form's title names the project. `x` opens a picker of the
   selected project's repos, then a confirm, then sends `RemoveRepo` (the core already refuses
   while a slice is open in that repo).
6. **Soft effort limit** (decision 7). `Config::max_efforts` (default 4) from `config.toml`;
   `Snapshot::effort_limit` carries it from both Home's and a project's core; the GUI reads it
   instead of the pending state. `config.toml` is optional and `scripts/bootstrap.sh` writes
   none, so the key is documented with the others in `crates/dovetail-core/src/config.rs`. Answers design v2 open point 9: a running effort is an open effort with an open slice
   (as `ProjectCounts.running` already counts).
7. **Canvas blocks** (decisions 2 and 8, ADR 0027).
   - State migration `0021_blocks.sql`: `canvases` → `blocks` (adds `x`, `y`),
     `cards.canvas_id` → `block_id`, `free_board` → `free_area`, a `seen` flag on cards
     (existing rows seen).
   - API: `Canvas`/`CanvasId`/`Framework` → `Block`/`BlockId`/`Layout`; `Card.block`;
     `Command::CreateBlock { effort, layout, title }` (the core places it), `RenameBlock`,
     `PlaceCard { block, … }`, `SeeCard`; `Event::BlockUpserted`. Layout sizes the core and the
     GUI share (`CARD_W`, `CARD_H`, `GAP`) live in `dovetail_api::canvas`.
   - Core placement (`canvas.rs`): a new block goes right of everything on the effort's
     canvas; a card added into a block goes under its parent (`under`) in the next free
     column, else into the block's next free grid cell. Nothing already placed moves.
   - Tools: `canvas_add_block { effort?, layout, title }` (new, canvas suite) and
     `canvas_add_card` gains `block` and `under`; a card with a block lands placed and unseen,
     one without goes to incoming as before.
   - GUI: the effort's canvas is all its blocks: each a framed group with its layout and
     title, cards inside; unseen cards marked "new" until selected; the incoming strip always
     shows; an effort with no blocks shows the empty board with the `b` hint. `b` opens the
     which-key "new block" pop-up (`1` question tree, `2` options compare, `3` free area); bare
     `1 2 3` go. `a` on an incoming card places it in its parent's block, else the last block,
     else a new free area.
   - Role prompts (`config/roles/*.md`) mention blocks only if they mention canvases now.
8. **Tests and checks.** Tests for each GUI action above against the fake core link; core
   tests for placement, the inbox count and the migration; `scripts/check.sh`;
   `scripts/install.sh` (core and tool changes).

## Risks and open questions

- Converting a block's layout is out (decision 2); "decision rounds" is a later block kind.
- Options compare places a card in its option's column (`under` = the option card); criteria
  rows are not modelled yet. Moving a whole block is not in this build: the user moves cards.
- The `?` map and which-key read the keymap, so the new rows show without extra work.

## Status

1. Done: `Enter` on a slice row sets `ui.viewerAgent` and opens the viewer.
2. Done: `newSlice`, its form, `new_slice` in the GUI protocol, translation and MockCore removed.
3. Done, changed: the core keeps the coordinator's finished-turn item (seven core tests and the
   frozen TUI rely on ADR 0001's one-per-agent item); the GUI leaves it out of the inbox. ADR 0028
   says so.
4. Done: `modeTarget()` in `actions.ts`; the view's agents carry their mode.
5. Done: `awaitProject`; the add-repo form already named its project; `x` sends `remove_repo`.
6. Done: `Snapshot::effort_limit`; Home sums the projects' running counts.
7. Done: migration `0021_blocks.sql`, `Board` placement in `db/canvas.rs`, `canvas_add_block`,
   `SeeCard`, the GUI's blocks and `b` pop-up (`PREFIX_POPUPS` in the keymap, drawn by
   `WhichKey`). MockCore places more simply than the core (no gap search).
8. Tests: `actions.test.ts` (viewer pick, mode target, new project, need-you count, `b`, seen),
   `canvas.test.ts` (blocks, placement in MockCore), `sessions.rs` (block and card placement,
   tools), `db/canvas.rs` (migration from version 20).


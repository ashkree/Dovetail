# Screens

Mock screens from the design phase, exported from the two Claude Design canvases ("Dovetail TUI layout" and "Dovetail Visual Maps Mock"). Each screen is a standalone `.html` file (open it in a browser) with a `.png` render beside it. The HTML is the more precise reference for glyphs, spacing and text. Colours use the Twilight theme (ui/) and the maps palette (maps/); they show intent, and the real theme lives in the theme struct.

These are **reference, not spec**: where a screen disagrees with `docs/design.md` or `docs/design-digest.md`, the docs win. Mock data (task names, PRs, `FS_EVAL_VIRTUAL_KEY`) is illustrative only.

## ui/

| File | Screen | Status |
| --- | --- | --- |
| `b.*` | **Layout B, the v1 default** (Decision 15): tree over inbox on the left, conversation full height on the right. Shows glyphs, permission-mode badges, branch lines, to-do items, subagent progress, the inbox preview with an "always allow" rule and the reading theme | **v1 target** |
| `navigation.*` | Every screen and the key that reaches it, around the main layout (rev 121 of the design doc). Includes morning review, end-of-day record, picker, search, quick capture, task detail, command line, focus, map mode, side helper, insert, board, worktrees, activity, message timeline, take-over | **v1 + v1.1 overview** (v1.1 screens are tagged) |
| `switcher.*` | Telescope-style session switcher (`<leader>ss`) | v1 |
| `board.*` | Kanban board in the main region (`<leader>b`) | v1 |
| `worktrees.*` | Worktree view, one row per session's worktree (`<leader>g`) | v1 |
| `focus.*` | Focus mode, coordinator only with a solid reading column (`<leader>z`) | v1.1 |
| `themes.*` | Reading themes: Dusk (default), Ember, Fjord, High contrast | reference for the theme struct |
| `main.*` | Layout A (the UI brief's sketch) | superseded by B |
| `c.*` | Layout C, three columns | rejected |

## maps/

| File | Screen | Status |
| --- | --- | --- |
| `canvas.*` | Image canvas: the main map view (kitty graphics) | **v1 target** |
| `outline.*` | Outline: side view beside the canvas, and the fallback without image support | **v1** |
| `main.*` | Node kinds legend (style per kind) | reference |
| `drawing.*` | Map drawn in text characters | **dropped** (Nov 1); do not build |

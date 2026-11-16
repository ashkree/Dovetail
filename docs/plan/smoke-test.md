# Live smoke test of the v2 GUI (2026-11-16)

First run of the Tauri GUI against real Claude Code sessions. Setup: `scripts/dev-gui.sh --reset`
(state under `target/dovetail-dev/`), scratch repos `/tmp/dvt-smoke/shop-web` (static page;
recipe `[repos.shop-web] preview = "python3 -m http.server {port} --bind 127.0.0.1"`) and
`/tmp/dvt-smoke/shop-api`. The window ran on a headless Hyprland output and was driven by keys
only (`hl.dsp.send_shortcut`), with `grim` screenshots, the dev `state.db` and the process tree
as evidence. Branch `claude/smoke-test-fixes-0713cc`.

| # | Step | Result | Evidence |
| --- | --- | --- | --- |
| 1 | New project form (Home `n`) | pass | "Smoke" with shop-web created; Home row shows it, account default |
| 2 | Add repo form | pass | `r` on Smoke added shop-api; Home row and top bar show both repos |
| 3 | Coordinator chat, reply streams | fixed (fix 1) | "Reply with one word: ready" → "ready" (Sonnet 5.5); `claude` + `dovetail mcp` children of the GUI |
| 4 | Permission request answered from the inbox | fixed (fix 2) | `Space s m` ×2 → Manual; `Bash · ls /tmp/dvt-smoke` came to the inbox; `y` ran it, the reply listed both repos |
| 5 | New slice (`Space s n`), task status, `coordination_report` | pass | repo picker → shop-web; worktree `…/projects/smoke/worktrees/header` on `dovetail/header` of shop-web; task To do → In progress → Review; report shows as one collapsed `▸ header` line |
| 6 | Viewer: preview, `design_declare_variants`, switch options | fixed (fixes 3, 4) | `python3 -m http.server` started in the slice's scope, page shown; slot `headline` a/b declared; `2` → page shows "Best Smoke Shop", `design_variants.selection = {"headline":"b"}` |
| 7 | Canvas: `canvas_add_card` card appears | fixed (fix 5) | card "Try a third headline" in the incoming strip once a canvas exists (`1`); `gi a` placed it |
| 8 | Close window: sessions reaped; reopen restores state | pass | after close: no `claude`, `dovetail mcp`, preview server or `dovetail-*` scope left (twice); reopen restored chat, effort, task (Review), slice, inbox, Manual mode, canvas card, variant pick; coordinator resumed with `--resume` |

## Real-session spend

6 turns, about $0.35 (from `agents.cost_usd`): coordinator $0.25 over 5 turns (2 typed,
1 canvas ask, 2 started by Dovetail's "slice ready" notice and the slice's report), slice
$0.10 over 1 turn.

## Findings

1. **Fixed — chat bar dropped every message in a new project.** `send` returned when no effort
   was in focus; a new project shows none (Unsorted is hidden while it has no tasks). Untagged
   messages now go to Unsorted. `gui/src/keys/actions.ts`.
2. **Fixed — `Space s m` was a "not in this build" toast**, so auto mode (whose classifier
   approved the `ls`) could not be left and no permission request could be produced. Wired to
   the core's existing `CyclePermissionMode` for the coordinator; MockCore answers it too.
3. **Fixed — a new effort never took focus.** The action looked the effort up right after
   sending `create_effort`, before the core had added it; focus stayed on Unsorted, so the viewer
   said "No slice in this effort has a web preview yet". It now takes focus when the core's
   `effort_upserted` arrives (`awaitEffort`, like `awaitNewCard`).
4. **Fixed — viewer option keys did nothing with one slot.** `ui.slot` starts at 2 (the mock's
   set); the index is now clamped to the set's slots (`slotIndex`).
5. **Fixed — the empty canvas showed `1 · 2 · 3` but no key was bound** (mouse only). `1 2 3`
   now start a canvas when the effort has none; row added to `docs/gui-layout-and-keys.md`.
6. **Fixed — bottom bar showed `efforts N / 0 ▲`** (a warning on every run): the core sends no
   soft limit. With none, only the count shows.
7. **Fixed — the recipe written for a new project had no `preview` line.** The starter now has a
   commented one (`crates/dovetail-core/src/recipe.rs`).

Tests: `gui/src/keys/actions.test.ts` covers 1, 2, 3, 4 and 5 against a fake core link.
`scripts/check.sh` passes; `scripts/install.sh` rerun after the recipe change.

## Decisions for the user

1. **The viewer follows the focused effort only.** Moving the tree cursor onto a task or slice
   does not change focus, so with Unsorted focused the viewer says no slice has a preview while
   one exists. Should the tree cursor focus its effort, or the viewer fall back to any slice?
2. **Agent cards are invisible until a canvas exists.** `canvas_add_card` lands in the incoming
   strip, but an effort with no canvas shows only the framework choice. Show the strip anyway,
   or start a canvas when the first card arrives?
3. **"Slice ready" costs a coordinator turn and confuses it.** The notice made the coordinator
   answer "I haven't messaged it. Tell me what to brief it on", though the form had sent the
   brief. Should the notice name the brief (or not start a turn)?
4. **"X finished a turn" counts as "need you"** (`⚠ 2 need you` with nothing to answer). Keep?
5. **`Space s m` acts on the coordinator only.** Should it act on the agent selected in the tree?
6. **After `n` on Home the new project is not selected**, and `r` (add repo) acts on the
   selected row, so a repo is easily added to the wrong project.
7. **The per-machine soft limit** in the bottom bar has no source in the core; it is hidden now.
   Wanted as a `config.toml` setting?
8. Keys `1 2 3` on the empty canvas (finding 5) extend the still-proposed keymap.

## Not covered

- Streaming was seen only as finished replies (screenshots every few seconds); partial messages
  are on (`--include-partial-messages`) but the token-by-token render was not watched.
- "Keep this combination", deny/always-allow answers, and a second repo's slice were not run.

## Review (code-review skill, PR #85)

- Standards: empty-canvas hints now read from the keymap (`keyText`), one `FRAMEWORK_KEY` list,
  one `focusEffort` (in `ui.ts`), `gui/DECISIONS.md` §11 amended for `Space s m`. Left as is:
  `effort_limit` 0 meaning "none" (the API has no option there yet).
- Spec: `awaitEffort` is now a flag cleared by `command_failed` and by a new snapshot, so a
  refused or renamed effort cannot take focus later, and names the core spells differently still
  match. The `1 2 3` keys row is marked as awaiting review. The `?` map lists them even when a
  canvas exists (they do nothing then); left for the keymap review.

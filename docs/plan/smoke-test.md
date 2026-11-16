# Live smoke test of the v2 GUI (2026-11-16)

First run of the Tauri GUI against a real Claude Code session. Setup: `scripts/dev-gui.sh --reset`
(state under `target/dovetail-dev/`), scratch repos `/tmp/dvt-smoke/shop-web` (static page,
`preview = "python3 -m http.server {port} --bind 127.0.0.1"`) and `/tmp/dvt-smoke/shop-api`.
The window was driven by keys only (`hyprctl dispatch sendshortcut` on a headless output),
screenshots with `grim`. Branch `claude/smoke-test-fixes-0713cc`.

| # | Step | Result | Evidence |
| --- | --- | --- | --- |
| 1 | New project form (Home `n`) | pass | "Smoke" with shop-web created; Home row shows it, account default |
| 2 | Add repo form | pass | `r` on Smoke added shop-api; row and top bar show both repos |
| 3 | Coordinator chat, reply streams | fixed (fix 1) | after fix, "Reply with one word: ready" → "ready" (Sonnet 5.5), `claude` + `dovetail mcp` under the GUI |
| 4 | Permission request answered from the inbox | fixed (fix 2) | in Manual mode `Bash · ls /tmp/dvt-smoke` came to the inbox; `y` ran it, reply listed both repos |
| 5 | New slice (`Space s n`), task status, `coordination_report` | not reached | |
| 6 | Viewer: preview, `design_declare_variants`, switch options | not reached | |
| 7 | Canvas: `canvas_add_card` card appears | not reached | |
| 8 | Close window: sessions reaped; reopen restores state | not reached | |

## Real-session spend

Turns: 2 coordinator turns so far.

## Findings

1. **Fixed — chat bar dropped every message in a new project.** `send` returned when no effort
   was in focus; a new project shows none (Unsorted is hidden while it has no tasks). Untagged
   messages now go to Unsorted. `gui/src/keys/actions.ts`.
2. **Fixed — `Space s m` was a "not in this build" toast.** Auto mode's classifier approves an
   `ls`, so no permission request could be produced. Wired to the core's existing
   `CyclePermissionMode` for the coordinator. `actions.ts`, `protocol.ts`, `translate.ts`.
3. **Noted — the recipe written for a new project has no `preview` line**, so the viewer has
   nothing to start until the user adds one by hand (`crates/dovetail-core/src/recipe.rs`
   template).

## Decisions for the user

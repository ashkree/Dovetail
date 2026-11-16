# Live smoke test of the v2 GUI (2026-11-16)

First run of the Tauri GUI against a real Claude Code session. Setup: `scripts/dev-gui.sh --reset`
(state under `target/dovetail-dev/`), scratch repos `/tmp/dvt-smoke/shop-web` (static page,
`preview = "python3 -m http.server {port} --bind 127.0.0.1"`) and `/tmp/dvt-smoke/shop-api`.
The window was driven by keys only (`hyprctl dispatch sendshortcut` on a headless output),
screenshots with `grim`. Branch `claude/smoke-test-fixes-0713cc`.

| # | Step | Result | Evidence |
| --- | --- | --- | --- |
| 1 | New project form (Home `n`) | not reached | |
| 2 | Add repo form | not reached | |
| 3 | Coordinator chat, reply streams | not reached | |
| 4 | Permission request answered from the inbox | not reached | |
| 5 | New slice (`Space s n`), task status, `coordination_report` | not reached | |
| 6 | Viewer: preview, `design_declare_variants`, switch options | not reached | |
| 7 | Canvas: `canvas_add_card` card appears | not reached | |
| 8 | Close window: sessions reaped; reopen restores state | not reached | |

## Real-session spend

Turns: 0. Cost: $0.

## Findings

## Decisions for the user

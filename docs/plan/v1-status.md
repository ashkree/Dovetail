# v1: what's finished, what's left

State of `main` after the night of 2026-11-10/11 (wave-4 review merged). Source of truth for order of work stays `docs/implementation-plan.md`; this is a summary for the user.

## Finished

| Stage | Milestones | State | Evidence |
| --- | --- | --- | --- |
| Spikes | S0–S5 | Done. S5's look in kitty is still yours to judge | `docs/spikes/*.md` |
| Usable on mock | M1–M4 | Done | `docs/reviews/m1.md`–`m4.md` |
| Tracker | M5 | Done | wave 1 track A |
| Live sessions | M6–M9 | Code merged and used daily; the live-check boxes are unticked | waves 1–2; `scripts/live-checks.sh` |
| Self-hosting | M10 | Reached. Dovetail has been building itself since 2026-11-09; one box ("repeated once on this repo") is unticked | Daily-driver fixes table |
| Subagents, to-dos | M11 | Done on fake-driver replays; not checked live | PRs #12, #13 |
| Board, worktree view | M12 | Done | — |
| Memory caps, context fill | M13 | Done | PR #31, `docs/spikes/cgroup.md` |
| Take-over | M14 | Built; the real round trip is yours to try | PR #34 |
| Maps stage 1 | M15 | All six slices merged: (a) model, (b) tool and storage, (c) merman renderer, (d) swimlane layout, (e) shelf and outline, (f) kitty canvas and MAP mode. "Works in kitty" and "a real agent draws both map types" are yours to try | PRs #32, #35, #39, #38, #42, #47 |

**Built beyond the original list:** non-blocking `ask_user` (ADR 0007), inbox over the tree (0008), a Claude account per project (0009), chat bar keys and completion (0010), side chats (0011), end of day and a morning coordinator (0012), focus mode (0013, pulled from M16), `:upgrade` / `dovetail upgrade` (0014, PR #41), the decision log and `log_decision` (0003), `create_task`, `start_slice`, `list_tasks`, `get_task` and `update_task` (PRs #20, #25), archive tools, usage limits in the header, daily-driver fixes #1–#17, and the review-wave-4 cleanup (PR #40).

## Left for v1

Overnight 2026-11-10/11 the three build sessions stalled on a permission prompt; they were rerun on 2026-11-11 and merged as PRs #45, #46 and #47. The wave-4 review merged as PR #44 (`docs/reviews/wave-4.md`). The research docs PR #43 is open and waiting for you.

| Item | Who | Status |
| --- | --- | --- |
| M15 (f): kitty canvas, MAP mode, mouse only in MAP mode | agent | **Done**, PR #47; mouse capture proven by a test. Left for you: the look in kitty (below) |
| M15: the canvas in kitty, the outline elsewhere | you | open: see "Try the map canvas" |
| M16: activity feed events moved from M7 (permission answers, coordinator start and exit, task changes, PR and CI), startup warnings audit, RSS measurement | agent | **Done**, PR #46: activity items for permission answers, coordinator start/exit, task changes and PR changes; startup warnings for Claude Code version and missing recipe; memory caps; `scripts/rss-check.sh` measures 67 MB for the UI over 10 000 streamed turns. CI events are not emitted (no observed source; question in the PR) |
| Follow-ups #3 (attach an image from a pasted or dropped path) and #8 (clean up saved paste attachments); the digest's v1 roadmap audit | agent | **Done**: PR #45; the audit is the section below |
| Wave-4 code review from `review-wave-3` | agent (Opus) | **Done**: `docs/reviews/wave-4.md`, six questions for you |
| `/retro` for wave 4, then the `review-wave-4` tag | you | after reading the review |
| Answer the review's six questions (blocking `systemctl`, editor path check, `update_task` and Now, morning coordinator day rule, `<leader>y`, side chat read-only) | you | open |
| Research docs PR #43 (ADRs 0015–0018) and its conflict list | you | open, not merged |
| Live checks M6–M9, M11, M14 round trip, M15 "a real agent draws both map types" | you | run `scripts/live-checks.sh`; real sessions cost money, so agents don't run them |
| S6 (`ask_user` role-prompt reliability), now also covering the research suite | you review | open |
| S5 look in kitty, and the canvas look (M15 f has landed) | you | open: see "Try the map canvas" |
| M16: a full working week in place of tmux; RSS under 100 MB after a day; cutting anything in the v1 roadmap | you | open |

Agents update this file's "Left for v1" table when their work lands.

## Try the map canvas

In kitty, from the repo root (the mock seeds a structure map and a swimlane map; no real session runs):

```bash
cargo run --release -- --mock
```

Then `␣M` for the shelf, `j`/`k` to a map and `Enter`: the map opens in MAP mode, full width, with the outline on the right. Try `hjkl` (the yellow ring moves), `+`/`-`/`0`, `Ctrl-hjkl`, `Enter` on "dovetail-tui" (drills into the group) and `Backspace`, `n`/`p` on the swimlane, `v`, a click, the wheel and a drag. `Esc` leaves MAP mode; text selection with the mouse works again. Check that pan and zoom feel instant (no repaint), that the picture never covers the picker (`␣␣`), and that a second terminal without kitty graphics (or `DOVETAIL_GRAPHICS=none`) shows the outline alone. For a real agent drawing both map types, ask the coordinator in a scratch project to `draw_map` a structure map and a swimlane map of something small.

## v1 roadmap audit

Each item of the digest's §12 v1 list checked against `main` on 2026-11-11. **Shipped** = the code is there and covered by tests; the unticked live-check boxes in the plan (real Claude Code sessions, which agents don't run) are listed separately at the end. Nothing is cut; the "Candidates" column only marks what you may want to cut or defer. Your call.

| # | v1 item | Verdict | Evidence | Candidate |
| --- | --- | --- | --- | --- |
| 1 | `SessionDriver` on a community crate | Shipped, **by decision not as written**: both community crates failed S1 (permission requests never answered), so the driver is a hand-written NDJSON layer behind `SessionDriver` | `docs/spikes/driver.md`; `crates/dovetail-driver/src/{claude,parse,event}.rs`, `fake.rs` | none; the roadmap line is outdated |
| 2 | Headless launch with injected settings, hooks and role prompts | Shipped | `session/launch.rs` (`--settings` with the `WorktreeCreate` hook, MCP config, role prompt as launch flags; nothing in the repo) | none |
| 3 | Registry + `state.db`, resume after restart | Shipped | `db.rs` (`registry.db`, per-project `state.db`, versioned state migrations), `restore.rs`, `session/mod.rs` resume | none |
| 4 | Buffers in the fixed layout | Shipped | `region.rs`, `buffers/{conversation,tree,inbox,inbox_detail,board,feed,task_detail,worktrees,map_shelf,map_outline}.rs`; layout per ADR 0006/0008 | none |
| 5 | Normal/Insert/Command, hardcoded keymap through the action registry, `nucleo` picker | Shipped | `mode.rs`, `keymap.rs` + `action.rs` (one table), `command.rs`, `picker.rs`, `switcher.rs` | none |
| 6 | Two-tier inbox with inline approvals | Shipped | `core/inbox.rs`, `buffers/inbox.rs`, `inbox_detail.rs` (Needs-you / Finished tiers, Permission items answered in place) | none |
| 7 | Maps stage 1 | Shipped, look in kitty unchecked | Model, `draw_map`/`read_map` tools, storage, merman renderer, swimlane layout, shelf and outline (`dovetail-maps`, `buffers/map_*.rs`, PRs #32/#35/#38/#39/#42); the kitty canvas and MAP mode with mouse (`canvas/`, `buffers/map_canvas.rs`, PR #47). "Works in kitty" and "a real agent draws both map types" are yours | none |
| 8 | Context fill display | Shipped | `core/context.rs`; `context_pct` in `buffers/tree.rs`, coloured by `theme.context`; header limits (decision #25) | none |
| 9 | Quick capture + `dovetail add` + `create_task` | Shipped | `capture.rs` (`<leader>t`, `a`), `crates/dovetail/src/add.rs`, `create_task` in `core/tools.rs` (ADR 0002) | none |
| 10 | Readability | Shipped | `render/markdown.rs` (pulldown-cmark), `render/wrap.rs` (word wrap), `render/reading_column.rs`, `theme.rs` (softened colours, transparent background), collapsed finished turns (ADR 0001), message headers | none. No syntax highlighting (v1.1) |
| 11 | Auto mode + `<leader>sm` + concurrent approvals | Shipped | `Action::CyclePermissionMode` on `<leader>sm`; `PermissionMode` in `model.rs`; one inbox item per request, none blocks another (live box M7 unticked) | none |
| 12 | One worktree per slice via hooks | Shipped | `worktree.rs`, `recipe.rs`, `crates/dovetail/src/hook.rs`, `config/projects/dovetail.toml`; worktree view `buffers/worktrees.rs` | none |
| 13 | Task tracker | Shipped | `db/tasks.rs`, `buffers/{board,task_detail}.rs`, `list_tasks`/`get_task`/`update_task` tools | none |
| 14 | Agent tree with subagents | Shipped (on fake-driver replays) | `session/subagent.rs`, `buffers/tree.rs` (PR #12) | none |
| 15 | Background subagent nodes + native to-do items | Shipped (on replays) | `session/subagent.rs`, `todos.rs` (inotify watch, reread on parse error; PR #13) | none |
| 16 | Take-over | Shipped, round trip unchecked live | `takeover.rs` (`<leader>st`, PR #34) | none |
| 17 | cgroup caps + memory display | Shipped | `memory.rs`, `oom.rs`, `docs/spikes/cgroup.md`; `memory_mb` in the tree and switcher | none |
| 18 | `ask_user`, `report_status` | Shipped | `core/tools.rs`, `crates/dovetail/src/mcp.rs`; `ask_user` never blocks (ADR 0007) | none |
| 19 | Role prompts | Shipped | `config/roles/{coordinator,slice,side-chat}.md`, installed by `scripts/bootstrap.sh` | none. S6 (wording reliability) is still open |

**Missing or partial:** none since PR #47 landed the canvas (item 7). Items 14 and 15 are done but never exercised against a real session.

**Not on the list but promised for v1 by the plan (M16):** activity feed events (permission answers, coordinator start/exit, task changes, PR and CI), the startup-warnings audit and the RSS measurement are in progress in another session. They are unfinished until that lands.

**Live checks still unticked** (need real sessions and money, so yours): M6 (streaming, interrupt, reaping), M7 (manual-mode request answered, concurrent requests, mode switch), M8 (hook-created worktree, 15-minute hook, `gh pr merge` ask rule, archive), M9 (`ask_user` round trip, status lines, `create_task` marking, S6), M10 (repeat on this repo), M11, M14, M15.

**Candidates to cut or defer, for you:** none of the 19 items is a clear cut.

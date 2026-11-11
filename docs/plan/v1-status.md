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
| Maps stage 1 | M15 | All six slices merged: (a) model, (b) tool and storage, (c) merman renderer, (d) swimlane layout, (e) shelf and outline, (f) kitty canvas and MAP mode. "Works in kitty" and "a real agent draws both map types" are yours to try | PRs #32, #35, #39, #38, #42, #PRNUM |

**Built beyond the original list:** non-blocking `ask_user` (ADR 0007), inbox over the tree (0008), a Claude account per project (0009), chat bar keys and completion (0010), side chats (0011), end of day and a morning coordinator (0012), focus mode (0013, pulled from M16), `:upgrade` / `dovetail upgrade` (0014, PR #41), the decision log and `log_decision` (0003), `create_task`, `start_slice`, `list_tasks`, `get_task` and `update_task` (PRs #20, #25), archive tools, usage limits in the header, daily-driver fixes #1–#17, and the review-wave-4 cleanup (PR #40).

## Left for v1

Overnight 2026-11-10/11: the three build sessions (map canvas, hardening, follow-ups) did not start, so nothing of theirs landed: no branch, worktree or PR. The wave-4 review ran and merged (`docs/reviews/wave-4.md`). The research docs PR #43 is open and waiting for you.

| Item | Who | Status |
| --- | --- | --- |
| M15 (f): kitty canvas, MAP mode, mouse only in MAP mode | agent | **Done**, PR #PRNUM; mouse capture proven by a test. Left for you: the look in kitty (below) |
| M15: the canvas in kitty, the outline elsewhere | you | open: see "Try the map canvas" |
| M16: activity feed events moved from M7 (permission answers, coordinator start and exit, task changes, PR and CI), startup warnings audit, RSS measurement | agent | **Not done**: the overnight session did not run; reschedule |
| Follow-ups #3 (attach an image from a pasted or dropped path) and #8 (clean up saved paste attachments); the digest's v1 roadmap audit | agent | **Not done**: the overnight session did not run; reschedule |
| Wave-4 code review from `review-wave-3` | agent (Opus) | **Done**: `docs/reviews/wave-4.md`, six questions for you |
| `/retro` for wave 4, then the `review-wave-4` tag | you | after reading the review |
| Answer the review's six questions (blocking `systemctl`, editor path check, `update_task` and Now, morning coordinator day rule, `<leader>y`, side chat read-only) | you | open |
| Research docs PR #43 (ADRs 0015–0018) and its conflict list | you | open, not merged |
| Live checks M6–M9, M11, M14 round trip, M15 "a real agent draws both map types" | you | run `scripts/live-checks.sh`; real sessions cost money, so agents don't run them |
| S6 (`ask_user` role-prompt reliability), now also covering the research suite | you review | open |
| S5 look in kitty, and the canvas look once M15 (f) lands | you | open |
| M16: a full working week in place of tmux; RSS under 100 MB after a day; cutting anything in the v1 roadmap | you | open |

Agents update this file's "Left for v1" table when their work lands.

## Try the map canvas

In kitty, from the repo root (the mock seeds a structure map and a swimlane map; no real session runs):

```bash
cargo run --release -- --mock
```

Then `␣M` for the shelf, `j`/`k` to a map and `Enter`: the map opens in MAP mode, full width, with the outline on the right. Try `hjkl` (the yellow ring moves), `+`/`-`/`0`, `Ctrl-hjkl`, `Enter` on "dovetail-tui" (drills into the group) and `Backspace`, `n`/`p` on the swimlane, `v`, a click, the wheel and a drag. `Esc` leaves MAP mode; text selection with the mouse works again. Check that pan and zoom feel instant (no repaint), that the picture never covers the picker (`␣␣`), and that a second terminal without kitty graphics (or `DOVETAIL_GRAPHICS=none`) shows the outline alone. For a real agent drawing both map types, ask the coordinator in a scratch project to `draw_map` a structure map and a swimlane map of something small.


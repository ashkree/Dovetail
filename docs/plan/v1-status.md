# v1: what's finished, what's left

State of `main` at `ed3f4c8` (PR #41, 2026-11-10), with the work scheduled for the night of 2026-11-10/11. Source of truth for order of work stays `docs/implementation-plan.md`; this is a summary for the user.

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
| Maps stage 1 | M15 | (a) model, (b) tool and storage, (c) merman renderer, (d) swimlane layout, (e) shelf and outline merged | PRs #32, #35, #39, #38, #42 |

**Built beyond the original list:** non-blocking `ask_user` (ADR 0007), inbox over the tree (0008), a Claude account per project (0009), chat bar keys and completion (0010), side chats (0011), end of day and a morning coordinator (0012), focus mode (0013, pulled from M16), `:upgrade` / `dovetail upgrade` (0014, PR #41), the decision log and `log_decision` (0003), `create_task`, `start_slice`, archive tools, usage limits in the header, daily-driver fixes #1–#17, and the review-wave-4 cleanup (PR #40).

## Left for v1

| Item | Who | Status |
| --- | --- | --- |
| M15 (f): kitty canvas, MAP mode, mouse only in MAP mode | agent | scheduled overnight (`dovetail-v1-map-canvas`) |
| M16: activity feed events moved from M7 (permission answers, coordinator start and exit, task changes, PR and CI), startup warnings audit, RSS measurement | agent | scheduled overnight (`dovetail-v1-hardening`) |
| Follow-ups #3 (attach an image from a pasted or dropped path) and #8 (clean up saved paste attachments); the digest's v1 roadmap audit | agent | scheduled overnight (`dovetail-v1-followups`) |
| Wave-4 code review from `review-wave-3` | agent (Opus) | scheduled for 05:00 (`dovetail-v1-review`) |
| `/retro` for wave 4, then the `review-wave-4` tag | you | after the review |
| Live checks M6–M9, M11, M14 round trip, M15 "a real agent draws both map types" | you | run `scripts/live-checks.sh`; real sessions cost money, so agents don't run them |
| S6 (`ask_user` role-prompt reliability), now also covering the research suite | you review | open |
| S5 look in kitty | you | open |
| M16: a full working week in place of tmux; RSS under 100 MB after a day; cutting anything in the v1 roadmap | you | open |

The overnight sessions update this file's "Left for v1" table when their work lands.

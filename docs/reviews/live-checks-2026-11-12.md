# Live checks, 2026-11-12

Run by an agent at the user's request, from a cloud container, against real Claude Code 2.1.322 on the user's account. Model: Haiku (`DOVETAIL_LIVE_MODEL` default), tiny prompts. Scratch repos under `/tmp`, separate XDG dirs; nothing touched a real project. Commit `a36378f` (`main` after PR #71), debug build.

## How it was run

- `scripts/live-checks.sh driver permissions hook resume mcp`, with `DOVETAIL_LIVE_TESTS=1`.
- The `app` check (steps 1–11) by driving the real `target/debug/dovetail` in a 200x50 tmux pane and reading the screen after each step.
- Extra in-app checks the script doesn't cover: failed setup (M8), subagents and to-dos (M11), maps (M15), take-over (M14), restart and resume.

Container quirks, not Dovetail bugs: no `uuidgen` (a shim was used), and no systemd user bus, so memory caps were off. The app showed the expected "memory caps off" warning and ran on. The container's own Claude Code session variables were cleared for the nested sessions.

## Results

| Check | Plan box | Result |
| --- | --- | --- |
| driver: streamed turn in auto mode; own prompt replayed | M6 | PASS |
| permissions: manual-mode request reaches the host | M7 | PASS |
| hook: worktree outside the repo, 75 s setup finished, outcome recorded, repo untouched | M8 (B5) | PASS |
| resume: worktree session resumed from its worktree; transcript path matches the core | M10 | PASS |
| mcp: slice sees no coordinator tools; old names listed as aliases; side chat sees Core only; app absent gives a clear error | M9 | PASS |
| 1. "not a Dovetail project yet", gone after `:project add` | ADR 0005 | PASS |
| 2. Reply streams in | M6 | PASS |
| 3. `<leader>sx` interrupts: working → "Interrupted" → idle | M6 | PASS (see note 1) |
| 4. Manual mode: Permission item, `y` from the inbox, file written; badge follows auto → acceptEdits → manual → auto | M7 | PASS (see note 2) |
| 4b. Two writes in one message, answer the second first | M7 | NOT PROVABLE IN ONE SESSION (note 2) |
| 5. `core_ask_user`: in the chat and Needs you, returns at once; `r` reply arrives as a message | M9, ADR 0007 | PASS |
| 6. `core_report_status` line in the tree | M9 | PASS |
| 7. `tasks_create` lands in Unsorted marked ✦; `project_start_slice` raises a Permission item in manual mode; slice set up on `dovetail/demo-1`; the coordinator gets "ready"; the task moves to Now | M8, M9, ADR 0004 | PASS |
| 8. SendMessage both ways; `<leader>m` timeline shows both | M8 | PASS |
| 9. Slice commits; `dd` raises "Remove worktree · 1 unpushed commit"; `y` removes worktree and branch; repo clean | M8 | PASS |
| 10. `gh pr merge 1` in auto mode raises a Permission item (ask rule); denied | M8 (B7) | PASS: the rule holds, no fallback needed |
| 11. `:q` (then `n` at "Close the day?"): no `claude` process, no zombies | M6 | PASS (twice) |
| Reopen: coordinator resumes the same conversation (remembers the earlier answer) | M10 | PASS |
| Failed setup (`exit 3`): inbox error ✗ with the log path; no user message in the slice's stream | M8 | PASS (see note 3) |
| Background subagent node with summary and "done"; to-do items (alpha, beta) under the coordinator | M11 | PASS |
| Coordinator draws a structure map and a swimlane map; both on the shelf; outline fallback in tmux | M15 | PASS (kitty look still yours) |
| Take-over: `<leader>st` on a slice opens native Claude Code on the same session; a native turn; `/exit`; headless resumes and knows both the headless and the native facts; worktree kept | M14 | PASS (see note 4) |
| Non-auto init shows a warning | M6 | NOT RUN (note 5) |

## Notes

1. Claude Code now blocks a bare `sleep N` in Bash, and Haiku refuses a Python sleep as a workaround, so the script's step 3 ("Run sleep 30") ends the turn before you can interrupt it. A long streamed reply was interrupted instead. Step 3 in `scripts/live-checks.sh` now says so, and the script falls back to the kernel UUID when `uuidgen` is missing.
2. Asked for two parallel Write calls, Claude Code emitted both `tool_use` blocks but sent **one `can_use_tool` at a time**: the second request arrived only after the first was answered. So "answer the second first" can't be done inside one session, and "one held request never blocks another" can only be tested across two sessions (coordinator and slice both asking). Left unticked.
3. A slice whose setup failed keeps its worktree and `dovetail/bad-1` branch, shown with ✗ in the tree, until archived. No doc says whether that is wanted. It looks deliberate, since you can read the setup log in place. Flagging it for your call.
4. On `/exit` after a native turn, Claude Code 2.1.322 asks "Keep worktree / Remove worktree", because a `WorktreeCreate` hook made the worktree. The default is Keep. Choosing Remove runs Dovetail's `worktree-remove` hook, which is a no-op by design (A9), so the worktree should survive. The prompt didn't appear on a second take-over with no native turn, so the Remove path itself wasn't exercised. The first take-over also hit Claude Code's first-run onboarding and folder-trust prompt, because this container had never run Claude Code interactively. A user's own machine won't see that.
5. The warning needs Claude Code to report a non-auto mode at init when auto was asked. Forcing that needs a Claude Code setting outside Dovetail's launch flags. In this container that would have changed the host session's own settings, so it wasn't done.

## Still open after this run

- M6: non-auto init warning (note 5).
- M7: one held request never blocks another, across two sessions (note 2).
- M9: S6 (role-prompt reliability review).
- M10: the rehearsal repeated once on this repo (needs your OK; agents don't run sessions on real projects).
- M15: the canvas in kitty (yours).
- M16: a working week in place of tmux; RSS after a day.

# Wave 4 review

Review of `review-wave-3...cf15fe2` (about 105 commits: M11 leftovers, M12 board and worktree view, M13 memory caps, OOM restart and context fill, M14 take-over, M15 (a)–(e) maps, profiles, chat bar keys and completion, side chats, end of day, focus mode, `:upgrade`, the coordinator task tools), with `/code-review` (two Opus reviewers, Standards and Spec). Unattended run, 2026-11-11.

**Result:** `scripts/check.sh` passes before and after the fixes. No real Claude Code session was run. M15 (f), M16 and follow-ups #3 and #8 were scheduled overnight but did not start (no branch, worktree or PR), so they are not in this review.

## Standards

No hard violations. Checked and holding: no panics in library code, blocking work (clipboard, paths, editor, take-over, upgrade build) in `spawn_blocking`, SQLite only through `Db`, redraw only when dirty, nothing written into user repos, no background painting, kitty keyboard flags only when supported.

| Finding | Kind | Outcome |
| --- | --- | --- |
| `memory::clear_stale_scope` runs `systemctl` synchronously on the core's loop (`session/mod.rs:216`, the launch path); `sample_memory` reads every `/proc/*/stat` on the loop | AGENTS.md "tokio for process/IO in the core", judgement | **Question for you.** The `systemctl` call only runs after a crash, and `launch` is synchronous with four callers, so moving it to `tokio::process` makes the launch path async. The `/proc` reads are procfs and fast. Proposed: leave it for v1, make `launch` async in v1.1 |
| The TUI checks `path.exists()` before an editor hand-off (`event_loop.rs:228`) | CODING_STANDARDS.md Architecture, judgement | **Question for you.** The check avoids a screen flicker; the core already answers `TerminalReturned { error }`. Moving it to the core costs a flicker on a missing path |
| Completion popup keys matched directly (`complete.rs:203-209`) | One keymap table, judgement | **Open.** Same pattern as typing in the chat bar; needs a seam when the keymap becomes TOML (v1.1) |
| `MapOutlineBuffer::expect` reads like a panic | Mysterious Name | **Fixed**: renamed `await_map` |
| Sticky selection (`selected` id plus `last_index`) copied into seven buffers | Duplicated Code | Open: a generic `Cursor<Id>` would replace them |
| Four ways to run `git` (std `Command` in three files, async with timeout, the upgrade `Exec` trait) | Duplicated Code | Open |
| `revert_coordinator: Option<(Option<String>, Option<PathBuf>)>`; `closing_result(text, interrupted: bool, last: bool)` | Primitive Obsession, Data Clumps | Open: the same fix as wave 2's `StatusInputs` |
| `takeover.rs` (~750 lines) mixes `/proc` scanning, launching and transcript catch-up; `validate.rs` ~930 lines | Divergent Change, soft | Open |

## Spec

Most ticked boxes for M11–M15 and the ADR 0009–0014 behaviour check out against the code. Open boxes (M14 real round trip, M15 real agent, kitty canvas, mouse only in MAP mode) are correctly unticked.

| Finding | Kind | Outcome |
| --- | --- | --- |
| Plan's M15 progress still said "(c), (d) and (e) are unblocked" | Stale plan | **Fixed**: records PRs #39, #38, #42 |
| `list_tasks`, `get_task` (PR #20) and `update_task` (PR #25) not in the plan or status | Gap in the record | **Fixed**: row in the daily-driver table and `v1-status.md` |
| `update_task` refuses to move a task into Now; only starting a slice does | Unrecorded decision | **Question for you:** confirm, then it gets an ADR |
| Board `Tab`/`Shift-Tab` beyond M12's key list | Scope | No action: documented in `ui-session-brief.md` |
| Morning coordinator: `fresh_day` compares the newest journal's day with today, not with the day the coordinator was last used. Used Monday without `:eod`, newest journal Friday → Tuesday starts from Friday's handoff; Monday is reachable only through "Keep yesterday's coordinator" | Matches ADR 0012's wording, but loses a day | **Question for you:** add `latest journal >= last used` as a condition (changes ADR 0012) |
| `<leader>y` copies only the last assistant text block, so an answer split by tool calls is cut, and mid-turn it copies partial text | ADR 0011 "the side chat's last answer" | **Question for you:** gather every assistant entry since the last user message? Small fix once you say what "answer" means |
| Side chats are "read-only" but Bash follows the permission mode, so under auto or bypass a side chat can write with `sed -i` | Matches ADR 0011; the label overstates it | **Question for you:** force `default` mode for side chats, disallow Bash, or rename "read-only" |

## Questions for you

1. Blocking `systemctl` on the launch path: leave it for v1?
2. Editor hand-off path check: keep it in the TUI?
3. `update_task` never into Now: confirm, so it gets an ADR?
4. Morning coordinator: also start fresh only when the newest journal is at least as new as the last use?
5. `<leader>y`: the whole answer since your last message, or the last block?
6. Side chats: enforce read-only, or change the wording?

## Retro

To be filled in with `/retro`, then tag `review-wave-4`.

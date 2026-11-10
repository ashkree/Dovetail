# End-of-day handoff and a fresh coordinator each morning, in v1

Refines Decision 17 (daily review cycle) and Decision 22 (context fill and fresh starts), both v1.1 in the roadmap. This pulls their smallest useful part into v1: the end-of-day record with the coordinator's handoff, "Close the day?" on quit, and a fresh coordinator on the first open of a new day.

Why: the coordinator's context grows over a day of work, and the user shuts the machine down each evening (no sleep). Starting each morning from a written handoff instead of resuming yesterday's long conversation keeps the coordinator sharp. Approved by the user in the session of 2026-11-10, the day before the first use.

**Decision:**

- **The journal file.** `~/.local/share/dovetail/projects/<id>/journal/YYYY-MM-DD.md`, local date (Decision 17b's location; no custom folder yet). Free-form markdown: a file the coordinator wrote by hand counts as a handoff, all of it. A block Dovetail writes starts with the marker `<!-- dovetail: end of day -->` and holds two sections:
  - **Handoff**: the coordinator's reply to Dovetail's request (open threads, what it was about to do, decisions in flight and why, anything the user asked it to remember, what matters first tomorrow; under a page). Dovetail saves the reply; the coordinator does not write the file itself, so no write outside the project needs a permission.
  - **Facts**: from Dovetail's own data: open tasks by bucket, slices and their status, what is left in Needs you.
  A journal file is never replaced: a second close on the same day appends its block below a rule, and the last block's Handoff is the one read.
- **`:eod`** asks the coordinator for its handoff and writes the file when that turn ends. If the coordinator cannot answer (it never ran, its session fails, the turn is interrupted), the file is written with the facts and says why.
- **"Close the day?" on quit** when today has no journal file yet and the folder is a registered project: `y` closes the day as `:eod` does and quits once the file is written; `n` quits now (still asking if a turn runs); `Esc` (or any other key) is "later" and cancels the quit.
- **A fresh coordinator on the first open of a new day:** when the newest journal file is from an earlier day and the coordinator's conversation was last used before today, the coordinator gets a new Claude Code session (a new session id, not `--resume`). Its first prompt holds that file's handoff and the open tasks. Yesterday's session id and transcript are kept, not deleted.
- **Keep yesterday's coordinator** is one key: a Needs-you item "Keep yesterday's coordinator" raised with the fresh start. `y` stops the fresh session and resumes yesterday's; `d` keeps the fresh one. At a restart the item expires with an Activity note, like other permission items.

**Stays v1.1:** the morning review buffer (`:morning`, `y`/`n`/`A` per proposed change, re-prioritising), the user's own notes in the record, the facts beyond tasks, slices and Needs you (finished and moved tasks, PRs, decisions, cost), journal search and indexing, a custom journal folder, the 80 % context-fill fresh starts for the coordinator, and slices' handoffs.

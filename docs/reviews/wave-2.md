# Wave 2 review and retro

Review of `review-wave-1...wave-2` (M6–M9 session core, live permissions, slice flow, tool handlers, ADR 0005), first with `/code-review` (two Opus reviewers, Standards and Spec), then a verification review of the fixes. Date: 2026-11-06.

**Result:** `scripts/check.sh` passes (fmt, clippy `-D warnings`, tests, String-error check). No real Claude Code session was run; live behaviour waits for `scripts/live-checks.sh`.

## Standards

| Finding | Kind | Outcome |
| --- | --- | --- |
| About ten `Result<_, String>` in library code | AGENTS.md "typed errors in libraries", hard | **Fixed**; `scripts/check.sh` now fails on it |
| `dovetail` → `dovetail-driver` edge and `tempfile` dev-dependency not explained | How to work 6 | **Fixed** in commit c12e971 |
| `PermissionMode` and other enums mapped to text in three places | Repeated Switches | **Fixed**: serde names |
| `HashMap<String, (String, String)>` sends; four positional bools in `status` | Primitive Obsession | **Fixed**: `PendingSend`, `StatusInputs` |
| `service.rs` grew by about 1,000 lines with unrelated jobs | Divergent Change | **Fixed**: `registration.rs`, `shutdown.rs` |

## Spec

| Finding | Kind | Outcome |
| --- | --- | --- |
| Project key not updated on registration: wrong recipe, worktree dir, to-do id, hook path | Wrong | **Fixed**, with a regression test |
| Archive confirmation asked twice | Wrong | **Fixed** |
| "Always allow" silently session-only | Unasked UX decision | **Decided** by the user: session-only |
| `start_slice` held the call in manual mode | Against ADR 0004 | **Fixed**: returns `pending`; ADR 0004 updated |
| Slice queue in memory only | Gap | **Fixed**: stored (migration 0004) |
| Slices refused before the folder is a project | Against ADR 0005's intent | **Fixed**; ADR 0005 updated |
| Ready signals waited for the coordinator's next turn | Gap | **Fixed**: sent at once, wakes an idle coordinator |
| Activity feed mostly empty | Partial | **Moved to M16** by the user |
| Brief sent on the hook marker, not "after init" | Deviation | Kept; noted in M8 |

## Fix review

| Finding | Outcome |
| --- | --- |
| Overlapping inputs ended a turn at the first `result` | **Fixed**: inputs counted against results, with a test |
| A setup signal could resume the coordinator during quit or after a crash | **Fixed**: notes held until the next start |
| `hook.json` left under the temporary key | **Fixed** |
| Registration rename broke the to-do list id of sessions that had run | **Fixed**: those keep their id |

**Known limits:** slices started before registration get no recipe setup; held coordinator notes are lost on quit (M10 should store them).

## Retro

| Finding | Category | Outcome |
| --- | --- | --- |
| A known mistake (String errors) came back; only review caught it | Automated checks | **Done:** `scripts/check.sh` is the gate, with a grep for String errors |
| A run died mid-wave with work uncommitted; four milestones landed in one 4,900-line commit | Process | **Done:** Waves rule: commit every milestone and step, one milestone per commit |
| A user-visible change was not listed in the report | Coding standards | **Done:** report lists every departure from a spec line |
| The project bug came from one concept held in two fields | Code | **Fixed** in the fix list |

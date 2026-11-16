# Dovetail sets task status from what it observes

**Status: Accepted and built.** Approved by the user in the session of 2026-11-15; built 2026-11-16, except subagent tracking.

Refines Decision 11 as `docs/design-v2.md` restates it ("Task status, set by Dovetail"). This ADR names the event in the core that drives each arrow, and what counts as the user's approval.

## Decision

Task status gains one value, `review`. Dovetail moves a task along; agents cannot.

```
          agent starts            PR open, or            PR merged, or
          on the task             agent reports done     you mark it done
 todo ───────────────► in_progress ───────────────► review ───────────────► done
                            ▲                         │
                            └──── PR closed unmerged, ┘
                                  or you send it back
```

| Arrow | Driven by | Where in the core |
| --- | --- | --- |
| todo → in_progress | A slice starts on the task. | `start_slice` in `slice.rs` (exists today) |
| in_progress → review | The slice's branch has an **open pull request**, or the task's agent calls `core_report` with `state: done` (ADR 0022). The second covers work with no PR, such as research. | `on_worktree_gathered` in `worktree_info.rs`; `Core::tool` |
| review → done | The pull request is **merged**, or the user marks the task done. | `on_worktree_gathered`; `Command::UpdateTask` |
| review → in_progress | The pull request is closed without merging, or the user sends the task back. | same |

**Why "merged" counts as your approval.** Dovetail's launch settings already force `gh pr merge` to ask (`permissions.ask` in `session/launch.rs`), so a merge by the coordinator has passed through your inbox. A merge you make on GitHub is your approval too. Watching the merge itself, not the inbox answer, covers both and cannot be fooled by an approval that then fails.

**To see a PR in time,** the core looks up the pull request after each finished slice turn. Today it does that only when the worktree view asks; after a turn it refreshes local git state alone. One `gh pr list` call per finished turn, already off the loop.

**Agents lose the status field.** `tasks_update` refuses `status` with "Dovetail sets task status". The coordinator's role prompt loses "mark its task done after merging". Archiving stays a separate tool. The user can always set any status by hand; that is never refused.

**A task with several slices** follows the one that is furthest behind: it is in review only when none of its slices is still without an open PR.

**Subagents.** Claude starts subagents itself, so Dovetail cannot make the call name a task. A slice's subagent counts under the slice's task. A coordinator's subagent counts under the task of the open research round if there is one; otherwise it is **untracked**, shown under the effort that the turn's chat message was tagged with. Untracked work moves no task.

Example. The user asks for "Web: guest checkout form" (task 14, todo). The coordinator starts slice `web-form`: **in_progress**. The slice opens PR #42 and its turn ends; the lookup finds an open PR: **review**. The coordinator asks to merge; the request appears in Needs you; the user allows it; the next lookup finds PR #42 merged: **done**.

## Alternatives rejected

- **Let agents set status through `tasks_update`, as today.** It is what the design set out to remove: status then says what an agent claimed, not what happened.
- **Move to done when the user answers the merge request in the inbox.** The merge can still fail after the answer, and a merge made on GitHub would never be seen.
- **Move to review when the slice's turn ends.** Turns end many times before the work is ready.
- **A webhook or a timer watching GitHub.** A new moving part and idle work; the end of a turn is when a PR appears.
- **Require every subagent call to name a task.** Not enforceable: the call is Claude Code's own tool.

## Consequences

- `TaskStatus::Review` is a new value in the API and in `tasks.status`. No migration: the column is text. The frozen TUI must show it as in progress.
- A merged PR on a task whose slice is already archived is not seen. Archiving a slice does a last lookup first.
- Size: about a week with tests. It was left out of the effort data layer because it changes what live sessions are told and adds a network call per turn.

## As built (2026-11-16)

- The report tool is `coordination_report` (ADR 0022, "As built").
- A `done` report moves the task to review at once only when the reporting slice is the task's only slice. With several, their pull requests decide.
- The pull request is looked up after each finished slice turn, when a report names one or says done, and as a slice is archived.
- Dovetail never moves a task the user set to to do or done by hand: only in progress and review are followed.
- **Not built:** the "untracked" grouping for subagents with no task. It needs the tree in the GUI to show it.

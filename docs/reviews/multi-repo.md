# Multi-repo projects review (ADR 0021, PR #82)

Review of `claude/multi-repo-projects` against `origin/main`, 2026-11-16, unattended. One pass of `/code-review` with two Sonnet reviewers. The Standards reviewer worked from `CODING_STANDARDS.md` and the hard constraints and architecture rules in `AGENTS.md`. The Spec reviewer worked from ADR 0021, `docs/design-v2.md` and `docs/gui-layout-and-keys.md`. I checked each finding against the code before acting on it.

**Result:** after the fixes, `scripts/check.sh` passes. A throwaway program drove Home and a project's core through `CoreLink`, as the GUI shell does, with the fake driver on scratch folders:

1. It created a project with two scratch repos.
2. It added a third.
3. It started a slice on a named repo, which ran in that repo with `DOVETAIL_REPO` set. A slice with no repo named was refused.

Not run: the Tauri window and the new forms. No real Claude session ran.

## Standards

| Finding | Kind | Outcome |
| --- | --- | --- |
| `project_op` in the core ran `git rev-parse` and registry waits on the core loop | AGENTS.md: process and IO through tokio in the core | **Fixed**: `ProjectOp::start` runs on `spawn_blocking` |
| Forced orphan-worktree removal fell back to an empty path when the worktree's repo could not be found | CODING_STANDARDS "Destructive paths fail loudly" | **Fixed**: the first repo is used only for a plain folder; otherwise the `main_checkout` error is returned |
| `project_op` in Home and in the core repeat the same flow (start, wait, starter recipe, summary) | Duplicated Code, judgement | Open: the two differ in sync and async waiting; the shared part is already `ProjectOp` |
| `main_checkout(..).unwrap_or(fallback)` repeated in `restore.rs` | Duplicated Code, judgement | Partly fixed with the removal fix; the inspect path keeps its read-only fallback |
| Repo id 0 marks an unregistered folder (`Some(repo.id).filter(..)`) | Primitive Obsession, judgement | Open: confined to the frozen TUI's unregistered folder |
| The five project commands are listed in `service.rs`, `ProjectOp::matches` and `ProjectOp::parse` | Repeated Switches, judgement | Open |
| `(name, brief, repo, by_coordinator)` copied between `SliceRequest` and `QueuedSlice` | Data Clumps, judgement | Open: it was already that way before `repo` joined it |
| `pick_repo` and the remove check walk `ProjectState`'s repos | Feature Envy, judgement | Open |

Checked and holding:

- `counts.rs` runs on the DB thread.
- There is no new `unwrap()` outside tests.
- New public API types have doc comments.
- The TUI gains only `repo: None`.

## Spec

| Finding | Kind | Outcome |
| --- | --- | --- |
| A repo of a project that is not open could be removed while a slice still worked in it ("A repo with a live slice cannot be removed") | Bug | **Fixed**: `RemoveRepo` reads that project's `state.db` for open slices in the repo (slices with no repo stored count for the first repo). This applies in Home and in the core. Test in `home_creates_projects_and_changes_their_repos` |
| Two repo changes in one coordinator turn kept only the second note ("tells it 'repo … attached'") | Bug | **Fixed**: notes are appended |
| The moved-folder fresh start applied to every session, including slices, whose folders did not move | Risk | **Fixed**: only sessions in the coordinator's folder (coordinator, side chats) |
| The account falls back to the recipe's `profile` | Checked | Holds: `project_claude_home` |
| `moved_coordinator` writes its own seed rather than reusing `fresh_coordinator` | Judgement | Kept. `:fresh` needs a handoff file and offers `y` to go back, and that cannot work across folders. The seed has the same parts (handoff if any, open tasks, "start no work until they answer") |
| The slice's first prompt names its repo when there are several | Beyond the brief | Kept; in the PR's Questions |
| `UNIQUE (project_id, path)` on `repos` | Beyond the ADR | Kept: it stops the same folder being added twice under two names |
| New slice from the chat zone has no task | Judgement | Kept: the task comes from the tree selection; the coordinator's tool always names one |

## Questions for you

The PR's "Questions" section lists the choices beyond the brief:

- The fake driver reports the folder a session started in.
- Research projects and side chats move to `home/`.
- The old coordinator session is reopened by hand, not from the inbox.
- The form layouts (`gui/DECISIONS.md` 13).

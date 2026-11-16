# Plan: multi-repo projects (ADR 0021)

Builds ADR 0021 end to end: a project is a name with zero or more repos and one account; the coordinator runs in `projects/<id>/home/` with every repo attached by `--add-dir`; a slice picks its repo by name. Written 2026-11-16 before the code; the order of work at the end is the order it was built in.

## Keywords

| Word | Meaning here | Where |
| --- | --- | --- |
| **Repo** | One row of the new `repos` table: a git checkout (or, for an old folder project, a plain folder) that belongs to one project, with a kebab-case name. | `registry.db`, `projects::Repo` |
| **Home folder** | `~/.local/share/dovetail/projects/<id>/home/`: the coordinator's working folder. Empty; Dovetail creates it. | `Paths::project_home` |
| **Coordinator folder** | Where the coordinator, side chats and Ask run: the home folder for a registered project; the open folder itself for one not registered yet (TUI only). | `ProjectState::coordinator_folder` |
| **Agent's repo** | The repo a slice works in: `agents.repo_id`, or the project's first repo when it is empty (slices from before the migration). | `Core::repo_of` |
| **Account** | `projects.account`: a profile name from `config.toml`. Empty: the recipe's `profile`, then the default Claude home. | ADR 0009, ADR 0021 point 1 |

## 1. Registry migration 0004

```
 BEFORE (0003)                                   AFTER (0004)
 projects                                        projects
   id, name, kind CHECK(code|research),            id, name, account, last_opened_ms
   repo_path, git_common_dir UNIQUE,             repos
   last_opened_ms                                  id INTEGER PK, project_id → projects ON DELETE CASCADE,
   CHECK(kind = research OR repo_path)             name, path, git_common_dir UNIQUE, added_ms
                                                   UNIQUE(project_id, name), UNIQUE(project_id, path)
```

- Every `projects` row is copied; `account` starts NULL (the recipe's `profile` keeps working).
- A row with a `repo_path` gets one `repos` row: name = the project id (a slug, so already kebab case), path = `repo_path`, `git_common_dir` copied (NULL for a folder project without git, which ADR 0005 allowed), `added_ms` = `last_opened_ms` or 0.
- `kind` goes. `ProjectKind` stays in the API, computed: no repos is `Research`.
- Tests (`db.rs`): from version 3 with a code project, a folder project without git and a research project; from empty; reopening changes nothing.

`state.db` migration **0018** adds `agents.repo_id INTEGER` (NULL for every existing agent). No other state change.

## 2. API changes (`dovetail-api`)

| Type | Change |
| --- | --- |
| `RepoId(i64)` | new |
| `RepoSummary { id, name, path }` | new |
| `ProjectSummary` | gains `repos: Vec<RepoSummary>`, `account: Option<String>`, `last_opened_ms: Option<i64>`, `counts: ProjectCounts`. `repo_path` and `kind` stay for the frozen TUI: `repo_path` is the first repo's path (the state folder when there is none), `kind` is `Research` when there are no repos. |
| `ProjectCounts { efforts, running, needs_you }` | new; read from each project's `state.db` (read-only) by Home and by the core's snapshot. `running`: open efforts with a live (not archived) slice on one of their tasks. `needs_you`: open inbox items plus claimed criteria. |
| `Command::CreateProject { name, account, repo }` | new. `repo`: an optional first repo by path. |
| `Command::RenameProject { id, name }` | new |
| `Command::SetProjectAccount { id, account }` | new; `None` clears it. |
| `Command::AddRepo { project, path, name }` | new; `name` defaults to the slug of the folder's name. |
| `Command::RemoveRepo { project, repo }` | new; refused while a slice of that repo is open. |
| `Command::NewSlice` | gains `repo: Option<String>` (serde default). |
| `tools::StartSlice` | gains `repo: Option<String>`; schema and description in `mcp.rs`. |
| `Event::ProjectUpserted(ProjectSummary)` | new; `apply` replaces or appends it in `projects`, and replaces `project` when it is the open one. |

Errors are typed: `ProjectError` grows `RepoTaken`, `RepoNameTaken`, `NoRepo`, `RepoInUse`, `NotAGitRepo` where needed.

## 3. Call sites of `Project.repo_path` and `ProjectState::folder`

Grouped by what each becomes.

| Becomes | Sites |
| --- | --- |
| **Coordinator folder** (`coordinator_folder`) | `session/mod.rs` `ensure_live` (coordinator), `takeover.rs` `session_cwd` default arm, `side_chat.rs` start, `session/mod.rs` init fallback cwd |
| **This agent's repo** (`repo_of(agent)`) | `slice.rs` `start_slice` (repo by name from the request), `slice.rs` worktree removal, `worktree_info.rs` gather (per target, not one repo for all), `restore.rs` sweep (per agent), `session/subagent.rs` subagent worktree cleanup (its parent's repo), `service.rs` `list_paths` fallback (agent's repo, else coordinator folder) |
| **The repo a path belongs to** (`git rev-parse` on the worktree, else first repo) | `restore.rs` orphan worktrees and their removal |
| **First repo** (until a repo can be named there) | `research::root_of` (`research_dir` relative to the repo root), `browse.rs` and `service.rs` `docs_repo` (`:docs`), `upgrade/flow.rs` build repo, `dovetail/src/upgrade.rs`, `dovetail/src/tui.rs` |
| **Has no repos** (`is_research`) | every `is_research()` check stays, now computed |
| **Registration** | `registration.rs` (`:project add`), `tools.rs` `project_add`, `projects::register`: a project plus one repo |
| **Lookup** | `projects::locate`: the folder's git dir (or, without git, its path) looked up in `repos` |

## 4. Core behaviour

- **Launch.** `SessionSpec` gains `add_dirs: Vec<PathBuf>`, one `--add-dir` each. The coordinator and side chats (which run in the coordinator folder) get every repo of a registered project.
- **Moved folder.** `launch` starts a fresh session when a recorded transcript was filed under another folder than the one it now starts in (Claude Code files transcripts by cwd). For the coordinator, `start_coordinator` does this first, seeded the way `:fresh` seeds one (the newest handoff if a journal exists, and the open tasks), with an inbox note naming the old session and transcript so it can still be resumed from its old folder by hand.
- **Repo added or removed.** The registry changes at once. The open project's core reloads it, emits `ProjectUpserted`, and, if the coordinator runs and is idle, restarts it (resume, new flags) and tells it "repo `<name>` attached at `<path>`" (or detached). A working coordinator is restarted when its turn ends. Removing a repo with an open slice is refused.
- **Slices.** `repo` by name; may be left out with one repo; required with several; refused with none. Stored as `agents.repo_id`. The slice gets `DOVETAIL_REPO=<name>` in its environment, which the worktree hook reads.
- **Recipe.** `[repos.<name>]` with `setup` and `env`: `Recipe::for_repo(name)` replaces `setup` when given and merges `env` (the repo's keys win). Existing files unchanged.
- **Account.** `project_claude_home`: `projects.account` when set, else the recipe's `profile`.
- **Home** handles the five project commands itself (registry only); the core handles them too, for any project, and reacts when the open one changes.

## 5. GUI changes (`gui/`)

- `view.ts`: Home's repos, account and counts from `ProjectSummary`; `pending.projects` and its use go. The open project keeps live counts.
- `n` on Home: a real form (name, account, first repo path), sent as `create_project`. Where `docs/gui-layout-and-keys.md` is silent (the form's look), it reuses the existing modal and prompt styles.
- `r` on Home and `Space p r` in a workspace: a prompt for the repo's path (and optional name after a space-free path), sent as `add_repo` for the selected or open project.
- Top bar repos: from the core's `repos`.
- New slice: asks for the repo with the picker when there are several.
- Types regenerated; `apply.ts` gets `project_upserted`; a shared apply case in `crates/dovetail-api/tests/fixtures/apply`.

## 6. Frozen TUI

Builds and passes. `dovetail` in a repo's folder still opens its project, through `repos`. `ProjectSummary.repo_path` and `kind` keep their meaning for one-repo projects, so nothing it shows changes.

## 7. Tests

| Test | Where |
| --- | --- |
| Migration 0004 from version 3 (code, folder, research); from empty; registry ops (create, add, remove, rename, account, find by git dir and by folder) | `dovetail-core/src/db.rs` |
| Recipe `[repos.<name>]` merge | `recipe.rs` |
| Slice repo choice: default with one, required with several, refused with none, stored and used as cwd | `tests/sessions.rs` |
| Coordinator runs in `home/` with `--add-dir` per repo; adding a repo restarts an idle coordinator with the new flag | `tests/sessions.rs` |
| Removing a repo with a live slice is refused | `tests/sessions.rs` |
| A coordinator recorded in the repo starts fresh in `home/`, seeded | `tests/sessions.rs` |
| Home creates a project, adds and removes repos, renames, sets the account | `home.rs` |
| `ProjectUpserted` apply case shared by Rust and TypeScript | `tests/fixtures/apply` |
| GUI: view of a project's repos and counts | `gui/src/core/view.test.ts` |

## 8. Order of work

1. Registry migration, `Repo`, registry queries, `Project` reshaped; tests.
2. API types, commands, event, apply; state migration 0018 and `AgentRow.repo_id`.
3. Core: `ProjectState` methods, every call site in §3, `--add-dir`, moved-folder fresh start, account.
4. Project commands in the core and in Home; restart-on-idle; recipe overrides and `DOVETAIL_REPO`.
5. Slice `repo` (command and tool); role prompt; bootstrap if a path changed.
6. TUI compile and `locate`.
7. GUI.
8. `scripts/check.sh`; run the GUI against scratch folders with the fake driver.

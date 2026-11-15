# A project is a name with repos; the coordinator gets its own folder

**Status: Proposed: awaiting the user's approval.** Nothing is built on it yet.

Supersedes ADR 0005 (a project is a folder, found from the current folder) and Decision 12's "coordinator in the main checkout". Refines ADR 0009 (the profile becomes the project's account) and ADR 0015 (the project kind). Settles open points 2, 5 and 8 of `docs/design-v2.md`, and proposes one change to it: the coordinator's folder.

`docs/design-v2.md` decided what a project is. This ADR decides how the registry stores it, where the coordinator runs, how a slice picks its repo, and how existing projects move over.

## Decision

**1. Registry shape.** One new table and one rebuild of `projects` (registry migration `0004`):

```
 TODAY                                    PROPOSED
 projects                                 projects
   id, name, kind,                          id, name, account, last_opened_ms
   repo_path, git_common_dir,             repos
   last_opened_ms                           id, project_id → projects, name, path,
                                            git_common_dir UNIQUE, added_ms
                                            UNIQUE (project_id, name)
```

- A project's `id` stays the slug of its name, so state folders, sockets and recipes keep their paths.
- `account` is the name of a profile in `config.toml` (ADR 0009's mechanism, unchanged). Empty means the default Claude home. While it is empty the recipe's `profile` key is still read, so nothing breaks on the day of the migration.
- `repos.name` is lower kebab case and is what agents and the user call the repo.
- The table rebuild is needed because `projects` carries `CHECK` rules that force a code project to have a repo. Registry migrations 0002 and 0003 rebuilt it the same way; every row is copied.

**2. The project kind goes.** A project with no repos *is* the research-only project: it cannot start slices, and its research folder defaults to its state folder. `ProjectSummary.kind` stays in the API for the frozen TUI, computed from "has no repos". Tangents is a built-in project with no repos.

**3. The coordinator runs in `projects/<id>/home/`, not in the state folder itself.** Each repo is attached with one `--add-dir <path>` launch flag.

```
 ~/.local/share/dovetail/projects/shopfront/
 ├── state.db
 ├── worktrees/api-guest/        ← slice checkouts (full copies of the code)
 ├── journal/
 └── home/                       ← coordinator cwd (proposed)
 attached: ~/code/shopfront-api, ~/code/shopfront-web
```

The design says "in the project's state folder". That folder also holds `state.db` and every slice worktree, so a search from there would hit each file once per slice, and the database would sit in the coordinator's working folder. An empty subfolder avoids both. This is the one place this ADR departs from the agreed design.

**4. A repo added or removed while the coordinator runs** takes effect at its next start. When the coordinator is idle Dovetail restarts it at once (a resume: same session, same folder, new flags) and tells it "repo `shopfront-web` attached at `<path>`". A repo with a live slice cannot be removed.

**5. A slice picks its repo by name.** `project_start_slice` and `Command::NewSlice` gain `repo`. With one repo it may be left out; with several it is required; with none the call is refused. The choice is stored as `agents.repo_id` (the repo's registry id, so a rename does not break it). The slice starts in that repo's main checkout with `--worktree`, as today. Worktree and branch names stay `worktrees/<slice>` and `dovetail/<slice>`: slice names are already unique in a project.

Example: `project_start_slice { task: 12, name: "api-guest", repo: "shopfront-api" }`.

**6. One recipe file per project, with per-repo overrides.** `~/.config/dovetail/projects/<id>.toml` stays. Its top-level keys apply to every repo; a `[repos.<name>]` table overrides `setup`, `env` and (later) the preview command for one repo. Existing recipe files work unchanged.

**7. Opening.** The GUI opens a project by id from Home and never looks at the current folder. The frozen TUI keeps `dovetail` in a folder working: `projects::locate` looks the folder's git dir up in `repos`. Running from memory in a folder that is not a project (ADR 0005) stays a TUI-only path.

**8. Migration of existing projects.** In registry migration `0004`, each project with a `repo_path` gets one `repos` row named after the project id. Nothing in `state.db` changes. On the first v2 open the coordinator starts a **fresh session** in `home/`, because Claude Code files transcripts by working folder and cannot resume one from another folder; it is seeded as `:fresh` seeds one today (ADR 0012). The old session stays resumable from the inbox item that offer already raises.

## Alternatives rejected

- **Keep `repo_path` on `projects` and add `repos` for the extra ones.** Two places to look for a repo, forever.
- **Coordinator in the first repo's checkout.** Makes one repo special and puts the coordinator's files where slices branch from. The design already rejected it.
- **Symlink the repos into the coordinator's folder.** Claude Code resolves symlinks inconsistently across tools; `--add-dir` is the supported way.
- **Keep the `code` / `research` kind as a stored column.** It would have to be kept in step with the repo count by hand.
- **One recipe file per repo.** More files for the common one-repo case, and the account would still need a project-level home.
- **Store the repo on the agent by name or path.** A rename or a moved checkout would orphan the slice.

## Consequences

- `Project.repo_path` and `ProjectState::folder` are read in more than fifty places in the core. Each becomes either "the coordinator's folder" or "this agent's repo". That is the bulk of the work.
- Docs reading (`:docs`) and `@` path completion need a repo to be named when there are several.
- The role prompt for the coordinator changes: it no longer runs in a checkout, and it names a repo when it starts a slice.

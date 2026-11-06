# Projects are registered only on request, and any folder can be one

Supersedes plan item B25 (offer to register an unknown repo; outside any git repo, open the last project) and wave 1's silent registration in `projects::resolve`. Refines the project model, which assumed every project is a git repo: Dovetail is also meant for work that is not coding.

**Registration only on request:**

- Opening `dovetail` in a folder that is not a project registers nothing and creates no files. The app shows the folder as "not a Dovetail project yet".
- A folder becomes a project only when the user asks: by command (`:project add`) or by telling the coordinator, at any point in the chat. For the second, the coordinator gets a tool `add_project`, which registers the folder it runs in and shows that it did.
- Until then the coordinator session, its tasks and its inbox live in memory only; they are written to the new project's `state.db` when it is registered, and are lost on quit if it never is.
- `dovetail add` in a folder that is not a project fails and says how to add it.
- Slices may start before registration too (decided 2026-11-06). Their state is persisted with everything else on registration; worktrees made before it stay where they are (their paths are recorded), and new ones go under the real project id.

**Which folder is the project:** inside a git repo, the repo (identified by its git common dir, so worktrees resolve to it); otherwise the nearest registered ancestor folder; otherwise the folder itself, as a candidate.

**Folders without git:**

- They work as projects: tasks, inbox, decisions, chat and slices.
- Slices run in the project folder itself, shared with the other slices, with no worktree, branch, setup, merge or cleanup. The slice's launch prompt says the folder is shared. The coordinator decides who works on what, to keep slices off each other's files.
- If the folder later gets a git repo, new slices get worktrees as usual.

Approved by the user in the session of 2026-11-05, at the wave-1 gate.

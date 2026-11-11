# Tangents, and one side panel for side chats and grilling

Refines Decision 18 (side-chat helper, v1.1) and [ADR 0011](0011-side-chats.md) (side chats, v1). Relates to [ADR 0012](0012-end-of-day-handoff-in-v1.md) and Decisions 17 and 22 (daily review cycle), whose morning review buffer is still v1.1.

Why: the user asks questions that do not belong to the current project and loses the answers. Claude Code's `/btw` is close but has no real tools, is not saved and cites nothing.

**What exists.** ADR 0011 side chats shipped in v1: a `SideChat` agent kind, started with `<leader>sq` or `:ask`, **per project**, read-only, in the project's main checkout, no task, answers copied with `<leader>y`. Decision 18's grill auto-start and `<leader>?` stayed v1.1.

**Decision:**

- **A Tangent is a question that does not belong to the current project.** It runs in a **global Tangents space** outside every project, like `/btw` but with real tools, saved, with sources. A side chat stays project-scoped; a tangent is global. They share the session kind, the role prompt (`config/roles/side-chat.md`) and the panel.
- **One side panel** replaces the separate "grilling side chat toggle" of Decision 18. It holds side chats, grilling helpers and tangents. The key that opens it is **open** (see the conflict list in the PR: `<leader>b` is the Board). `<leader>?` stays as an alias if that costs nothing; it is unbound in code today.
- **Every answer is saved with its sources** (the same capture as research, `docs/research.md`). Destinations for an answer: `<leader>y` into the user's reply, as now; attach to the current task as a quick finding; or, by default, Tangents.
- **The morning review shows new tangents.** For each one: file it into a project, start a task from it, or drop it. This sits in the v1.1 morning review buffer (`:morning`) built on ADR 0012; until that buffer exists, tangents are listed in the side panel.
- **Decision 18's auto-start** on a `grill*` Skill call stays v1.1 and now opens the same panel.

**Consequences and open points:**

- **ADR 0011's `<leader>sq`/`:ask` change behaviour** if the panel replaces the main-region conversation they open today. Proposed: keep them, opening the panel instead; the user decides.
- **Global state vs per-project state.** Everything lives under `~/.local/share/dovetail/projects/<id>/` and `state.db`; `registry.db` is global but holds only projects. Tangents need a home: a `tangents` table in `registry.db` (new migration) plus files under `~/.local/share/dovetail/tangents/`. The session needs a cwd; proposed: that folder.
- Tangents are not tracked in the agent tree of any project. How they appear in the switcher when no project is open is undecided.

Approved by the user in the grilling session of 2026-11-10 (claude.ai).

**Amended 2026-11-11 (grill with the user).** Settles the open points above; the side panel is built (PR #52), tangents are **not built yet** (M22). Changes:

- **The key is `<leader>?`.** The Board keeps `<leader>b`; the brief's `<leader>b` is dropped. The panel takes the task tree's region in the side column, one side chat at a time (`Tab`/`Shift-Tab` switch). `<leader>sq` and `:ask` keep working and open the new side chat in the panel (ADR 0011).
- **`<leader>y` is removed** (ADR 0011), so it is no destination. The destinations of an answer are: attach to the current task as a finding, or, by default, Tangents.
- **Tangents is a built-in research project**, not a separate store. It is always present, pinned last in the project switcher with its own marker, cannot be archived, and uses ADR 0015's research-only project kind (PR #54). Each tangent is a task in its Unsorted list, with the same sources, findings and files as any research task. This replaces "a `tangents` table in `registry.db` plus files under `<data>/tangents/`" and any `tangents.db`: the files live under `<data>/projects/tangents/` with the other projects, and no new registry table is needed beyond the project row.
- **The morning review** lists new tangents as one "new tangents" item to triage (file into a project, start a task from it, or drop it). Until `:morning` exists they wait in Tangents' Unsorted list.
- **No project open** stops being a case: Tangents is itself a project, so a tangent started from there is filed in it. Decision 18's grill auto-start stays v1.1.
- The reading-mode library shows no Tangents section until tangents exist (ADR 0018).

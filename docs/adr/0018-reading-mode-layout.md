# Reading mode: a second layout preset

Supersedes Decision 15's single fixed layout **for this one preset**. Decision 15 (Layout B, placement per [ADR 0006](0006-chat-left-layout.md), stacking per [ADR 0008](0008-inbox-over-tree.md)) stays the default and the only layout otherwise. Nearest precedent: [ADR 0013](0013-focus-mode-keeps-the-background.md) and focus mode (`<leader>z`, Decision 16), which already hides the side column; reading mode goes further and replaces the whole arrangement. Decision 6a ("fixed layout in v1, splits later") is not reopened: reading mode is a preset over the existing buffers, and becomes a saved split once splits exist.

Why: reading long cited reports in a chat-shaped column is hard. The user wants a library, a clean reading column and a place to ask about what is open.

**Decision:**

- **Reading mode** is a full-screen layout, toggled with `<leader>r`; `Esc` also leaves it. Three columns:
  - **Library** (left): projects, research tasks, then report / rounds / findings; plus Decisions, Tangents, and Docs · ADRs (repo docs, read-only).
  - **Reading** (middle): the 120-column reading column with the solid background on. Citations render as coloured `[n]` markers.
  - **Ask** (right): `i` opens a read-only subagent grounded in the open report and its sources; its answers cite with the same `[n]` markers.
- **Keys** (reading mode only): `]s` / `[s` next and previous citation, `gd` open the cited finding or source, `gx` open the source in a browser, plus the existing reading keys (`]]`, `[[`, `zc`, `zo`, `/`).
- **HTML artifacts are not rendered as pages.** They are converted to markdown for reading; `gx` opens the real page. Images and maps use kitty graphics where available, with the existing outline as the fallback (hard constraint: any terminal).
- It is built from the existing buffer abstraction (`Region`, `Buffer`): the three columns are three regions showing buffers, not a new rendering path.

**Open points (see the conflict list in the PR):** `Esc` leaves reading mode, but the UI brief says `Esc` belongs to mode switching (it returns to Normal); the proposal scopes it so that `Esc` in Normal mode inside reading mode leaves, and in Insert mode (the Ask column) it only returns to Normal. `i` is `EnterInsert` globally; in the reading column it means "ask", scoped to that buffer. `<leader>b` and `<leader>r` bindings for the side panel and this mode are the user's call.

Approved by the user in the grilling session of 2026-11-10 (claude.ai).

## Decided 2026-11-11 (user), and how M21 built it

These win over the proposal above where they differ.

- **`<leader>r` toggles reading mode**, a second layout preset over `Region`/`Buffer`. Layout B stays the default and is restored exactly (buffers and focus) when reading mode ends.
- **`Esc`:** in Normal mode inside reading mode it leaves reading mode. In Insert mode (typing in Ask) it only returns to Normal; a second `Esc` leaves.
- **`i`:** in the Reading column it opens Ask about the open report and puts you in Insert mode in Ask's chat bar. In the Library it does nothing.
- **Reading column:** 120 columns, solid background on automatically while in reading mode (not tied to `<leader>B`, which is going away), citations pink (`Theme::cite`). `]s`/`[s` next/previous citation, `gd` opens the cited finding or source, `gx` opens a web source in the browser through `Command::OpenExternal`: the core launches the browser, the TUI never spawns a process.
- **Library:** projects (code and research, `◇` for research) → research tasks → report, rounds (brief, findings), quick findings, sources; then **Decisions** (the decision log, with evidence links) and **Docs · ADRs** (the repo's `docs/` markdown, read-only; left out for a research project, which has no repo). **No Tangents section** until tangents exist (ADR 0017, M22): nothing half-built shows.
- **Ask:** a read-only session grounded in the open report and the sources it cites, not a subagent of the coordinator, so nothing lands in the coordinator's context. Its answers cite with the same `[F n]`/`[S n]` markers.
- **HTML** documents are converted to markdown for reading; **images** use kitty graphics where the map canvas's detection says the terminal has them, outline text otherwise.

Built as (implementation choices, listed in the M21 PR):

- **Ask is its own agent kind, `AgentKind::Ask`, with role `ask`** (`config/roles/ask.md`), one per research task, named `ask-<task>`. It launches with the side chats' read-only lists (`dontAsk`, `Read`/`Grep`/`Glob`, read-only Bash, no `Edit`/`Write`), cwd = the task's research folder, and **no Dovetail tool suites**: it reads the files instead of calling `research_get`, so it cannot write to the task list, decision log or research. Its first message carries the report and an index of finding and source ids → files; the user sees only their question. It shows only in the Ask column (not the tree, not the side panel).
- **Lifetime:** closing reading mode keeps it. `dd` in the Ask column closes it; a new report for the task closes it (the core does this when `research_write_report` stores one), so the next question starts fresh on the new report. Over a restart it resumes like a side chat.
- `Command::OpenExternal` opens only `http`/`https` URLs, with `$BROWSER` if set, else `xdg-open`.
- Ask answers raise no "finished turn" item in Needs you (the answer is in the column you asked in), Ask is not listed in the task tree, the switcher or the side panel, and it cannot be taken over.
- Extra keys: `Ctrl-o` goes back to the document read before `gd`; `:read` toggles like `<leader>r`; in Ask, `]s`/`[s`/`gd`/`gx` work on the latest answer. An action that shows something in Layout B (the board, the inbox, an agent picked in the switcher) leaves reading mode first.

## Decided 2026-11-11 (later): transparent column, `<leader>fr`

Task #37 (decisions #63–#65 of its log). These win over the sections above where they differ.

- **No solid background, anywhere.** Reading views (reading mode's reading column and a research document opened in the task detail) paint no background; `Theme::reading_bg` and `reading_column::paint_background` are gone. The rule for the whole app: **no solid backgrounds on areas (panes, columns, views); the selection highlight and badges are the only exceptions.**
- **Padding and width:** reading views set text at most **96 columns** wide, with padding `{x: 3, y: 1}` inside the column (`theme::READING_PADDING`), the column (102 wide) centred in its pane: `render::reading_column::reading_text`. This replaces the 120-column, solid-background column above. The conversation's reading column is unchanged.
- **`<leader>fr` browses another project's library** (reading mode only: bound in the library, reading column and Ask). It opens the project picker with the same rows and markers as `<leader>pp` (`◇` research, `open ·` the open one). Picking a project re-roots the library to it: its research tasks (report, rounds, findings, sources), decision log and `docs/` (code projects only), **read-only, with no app switch**. Picking the open project returns to it.
- **`Enter` on a project row** in the library browses that project the same way; it no longer offers to switch. `<leader>pp` stays the only way to switch projects.
- **Library title** while browsing: `library · <name> · not open`; the browsed project's row is highlighted.
- **Ask is off while browsing** another project (or reading one of its documents): `i` says "Ask works in the open project; switch with <leader>pp to ask about this."
- **Loaded on pick, no live refresh** (picking it again reloads). **Not remembered:** entering reading mode always starts on the open project.

Built as: `Command::BrowseProject { id }` → `Event::ProjectBrowsed(BrowsedProject)` (its tasks, decisions, research summaries, listings and docs) and `Command::LoadBrowsed { project, what: BrowseWhat }` → `Event::BrowsedLoaded` for one document; both change nothing in the snapshot. The core reads that project's `state.db` on the DB thread over a connection opened read-only for the request (WAL makes this safe while that project's own Dovetail runs; a `state.db` of an older schema is refused with "open that project once"), and its research files and docs, and writes nothing. In the TUI a browsed document is `ReadTarget::Elsewhere(project, target)`, so its task ids never mix with the open project's.

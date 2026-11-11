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
- **Library:** projects (code and research, `◇` for research) → research tasks → report, rounds (brief, findings), quick findings, sources; then **Decisions** (the decision log, with evidence links) and **Docs · ADRs** (the repo's `docs/` markdown, read-only; left out for a research project, which has no repo). **Tangents** (built in M22, ADR 0017): every tangent, whatever project is open, folding to its answers, findings and sources; its documents resolve `gd`/`gx` against its own sources. It is left out when Tangents is the open project, whose tangents are its research tasks.
- **Ask:** a read-only session grounded in the open report and the sources it cites, not a subagent of the coordinator, so nothing lands in the coordinator's context. Its answers cite with the same `[F n]`/`[S n]` markers.
- **HTML** documents are converted to markdown for reading; **images** use kitty graphics where the map canvas's detection says the terminal has them, outline text otherwise.

Built as (implementation choices, listed in the M21 PR):

- **Ask is its own agent kind, `AgentKind::Ask`, with role `ask`** (`config/roles/ask.md`), one per research task, named `ask-<task>`. It launches with the side chats' read-only lists (`dontAsk`, `Read`/`Grep`/`Glob`, read-only Bash, no `Edit`/`Write`), cwd = the task's research folder, and **no Dovetail tool suites**: it reads the files instead of calling `research_get`, so it cannot write to the task list, decision log or research. Its first message carries the report and an index of finding and source ids → files; the user sees only their question. It shows only in the Ask column (not the tree, not the side panel).
- **Lifetime:** closing reading mode keeps it. `dd` in the Ask column closes it; a new report for the task closes it (the core does this when `research_write_report` stores one), so the next question starts fresh on the new report. Over a restart it resumes like a side chat.
- `Command::OpenExternal` opens only `http`/`https` URLs, with `$BROWSER` if set, else `xdg-open`.
- Ask answers raise no "finished turn" item in Needs you (the answer is in the column you asked in), Ask is not listed in the task tree, the switcher or the side panel, and it cannot be taken over.
- Extra keys: `Ctrl-o` goes back to the document read before `gd`; `:read` toggles like `<leader>r`; in Ask, `]s`/`[s`/`gd`/`gx` work on the latest answer. An action that shows something in Layout B (the board, the inbox, an agent picked in the switcher) leaves reading mode first.

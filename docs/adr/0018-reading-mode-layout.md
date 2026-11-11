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

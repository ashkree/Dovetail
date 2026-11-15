# Dovetail v2 — Design

Agreed with the user in the grilling session of 2026-11-15. This is the new baseline. `docs/design.md` stays as the frozen v1 baseline and is never edited; where the two differ, this file wins. ADRs written after this file win over it on the decision they name.

Anything this file does not decide is listed under [Open points](#open-points). Do not fill those in from guesswork.

---

## 1. Goal

By **mid-December 2026** the user's daily brainstorming, coding and web design run in the Dovetail GUI, including building Dovetail itself.

- **TUI v1 is frozen as built.** No new features. The user decides later whether it catches up. SSH and tmux use are parked with it.
- **Open v1 checks:** only those that test the core stay open: worktree cleanup, out-of-memory recovery, subagent cost. M16's working week and the 100 MB RSS target are dropped.

## 2. App

| | v2 |
| --- | --- |
| Shell | Tauri 2, web frontend in React + TypeScript |
| Canvas | React Flow |
| Core | Hosted **in-process** by the GUI process. A full daemon stays for later |
| Input | Keyboard-first, modal (Normal / Insert, Space leader), one `Action` table. Mouse optional |
| Memory budget | 500 MB for the app window plus one preview page, measured in a first spike |

This replaces ADR 0019's egui Companion and its rule that the GUI never holds the only way to do something: in v2 the GUI is the app.

```
GUI process (Tauri 2)
├── web frontend (React + TS, React Flow)   ── Commands in / Snapshot + events out
└── core (Rust, in-process)
      ├── SQLite on one DB thread
      ├── SessionDriver ── headless Claude Code sessions (NDJSON)
      └── dovetail mcp  ◄── MCP tools from the sessions
```

## 3. Structure

### Terms

- **Home**: the first screen. It lists all projects, like the Google Docs file list. Entering a project opens its workspace. A project can open in a second window.
- **Project**: a named workspace. It holds any number of repos (including none) and one Claude account. It is not tied to a folder and is never found from the current folder.
- **Repo**: a git repository attached to a project.
- **Account**: the Claude Code account a project's sessions run under (v1's "profile", ADR 0009).
- **Effort**: one line of work in a project. It has a kind (feature, fix, cleanup, research, experiment), a defining question or goal, and acceptance criteria.
- **Task**: every task belongs to an effort.
- **Slice**: works in exactly one repo, with one worktree. An effort may have slices in several repos.
- **Coordinator**: one per project. It runs in the project's state folder with every repo attached as an extra directory.
- **Inbox**: per project. A cross-project or per-account inbox is later.

Tree: **project → efforts → tasks → agents**.

### Example: a project with two repos

```
Home
├── Shopfront                       account: work
│   repos: shopfront-api, shopfront-web
│   coordinator  (cwd: Shopfront's state folder; both repos attached)
│   │
│   ├── Effort  "Guest checkout"    kind: feature
│   │   goal: a buyer can pay without an account
│   │   criteria: [ ] pay as guest end to end   [ ] order email sent
│   │   ├── Task  API: guest order endpoint
│   │   │     └── slice  (repo shopfront-api, own worktree)
│   │   └── Task  Web: guest checkout form
│   │         └── slice  (repo shopfront-web, own worktree)
│   │
│   └── Effort  "Which payment provider?"   kind: research
│       └── Task  Compare three providers
│             └── subagents (under the coordinator)
│
├── Market notes                    account: personal
│   repos: none                     (research only)
│
└── Dovetail                        account: personal
    repos: dovetail
```

### Migration from v1

- Each existing project becomes a one-repo project (its repo is the v1 project's repo).
- Existing tasks move into an effort called **Unsorted**, which the user sorts by hand.

## 4. Staying on track

The user's drift is context switching: they start new lines of work while agents run and lose the thread on return. Everything in this section serves that.

### One chat per project, every message tagged

- One coordinator chat per project, not one per effort.
- Every message carries an **effort tag**.
  - The user's messages get the **focused effort**. One key changes the tag before sending.
  - Slice reports are tagged by Dovetail from the slice's task. They show as one collapsed line.
- A filter shows one effort's messages as that effort's thread.

Example (one project chat, two efforts):

```
[Guest checkout]   you          Start the web form once the endpoint is merged.
[Guest checkout]   coordinator  Will do. The API slice is on the endpoint now.
[Payment provider] you          Add Paddle to the comparison.
[Guest checkout]   ▸ slice api-guest-order: PR #41 open, tests pass      (collapsed)
[Payment provider] coordinator  Added. Paddle's fee card is in incoming.

Filter "Guest checkout" →
[Guest checkout]   you          Start the web form once the endpoint is merged.
[Guest checkout]   coordinator  Will do. The API slice is on the endpoint now.
[Guest checkout]   ▸ slice api-guest-order: PR #41 open, tests pass
```

### Unique session names, reports through Dovetail

- Sessions get unique names that include the project and the effort.
- Slice reports go through Dovetail's own tools, not native messaging. Why: today a slice in project A sometimes messages project B's session, because both are named "coordinator".

### "Where was I" card

Each effort keeps one card:

```
┌ Guest checkout · where was I ─────────────────────┐
│ What it is   Pay without an account               │
│ Last done    API endpoint merged (PR #41)         │
│ Waits on you Approve merge of PR #42              │
│ Next step    Order email after the form lands     │
└───────────────────────────────────────────────────┘
```

- The agent updates it every turn.
- It shows first when the user switches back to the effort.
- A fresh coordinator gets all cards as its full handoff.

### Running efforts

A soft count of running efforts against a per-machine limit set in config. It shows; it never blocks.

### Task status, set by Dovetail

```
 To do ──agent starts on it──► In progress ──agent finishes / opens PR──► Review ──user approves merge──► Done
```

- Starting an agent requires naming a task.
- Subagent work without a task shows as **untracked** under its effort.

Example: the user asks the coordinator to start "Web: guest checkout form". Dovetail starts a slice for it and the task goes **To do → In progress**. The slice opens PR #42: **→ Review**. The merge approval appears in the inbox; the user approves: **→ Done**.

### Acceptance criteria

```
agent claims "criterion met" + evidence ──► user approves ──► coordinator ticks it
```

- An effort never closes without the user's review.
- A task that serves no criterion is flagged.
- No dates, deadlines or estimates anywhere.

## 5. Coding

- A **code landscape** tool gives the coordinator the module dependency graph, the files each running or planned task touches, and where they overlap, so it can judge order and parallel work.
- Built cheaply first: build-tool dependencies, worktree diffs, and files a task declares up front. Later reused for the code map.
- The coordinator proposes merge order; the user approves each merge in the inbox.

## 6. Canvas

- Any number of canvases per effort. Each starts from a framework:

```
Question tree                  Options compare                 Free board
                               
      Q: which provider?              Stripe  Paddle  Adyen      (cards anywhere)
     ┌──────┴──────┐           fees    [card]  [card]  [ ]
  Q: fees?     Q: EU tax?      EU tax  [card]  [ ]     [card]
   [finding]    (open)         setup   [ ]     [card]  [ ]
```

  - **Question tree**: the question at the root, sub-questions as branches, findings attached to the branch they answer. Unanswered branches stay visibly open.
  - **Options compare**: options as columns, criteria as rows, evidence cards in the cells.
  - **Free board**.
- **Card kinds:** finding, source, idea, decision, question, map.
- Research output arrives as cards in an **incoming** strip. The agent adds and links cards but never moves the user's.
- Each card has a button that inserts its identifier into the chat and attaches its content. Any chat reply can become a card.
- Long reports keep a reading view. Findings are cards only.
- A structured map (Decision 23, `maps_draw`) can sit on a canvas as a card. The canvas model and the map model stay separate.

## 7. Viewer

For web UI previews.

- The project recipe gets a **preview command**, started per slice on a free port. The viewer follows the focused slice.
- A native **variant panel** with three levels:
  - global (e.g. theme, layout),
  - component (e.g. header, footer),
  - content (e.g. one line of text).
- Variants mix and match. The panel is detachable and closable.
- "Keep this combination" tells the agent to make it permanent and delete the rest.
- The agent writes variants in the slice's code and declares them in a list.
- No in-app screenshot key. Agent self-capture may come later.

## 8. Release split

Order of work: the GUI shell for the daily loop first, project-management data and tools in the core in parallel, then the viewer, then the canvas. The canvas is the first thing to move out if the date slips.

| | |
| --- | --- |
| **First GUI release** | Chat, sidebar tree, inbox with approvals, reading view; then viewer; then canvas |
| **Later** (still usable in the frozen TUI meanwhile) | Board, maps, daily review, tangents, side chats, take-over, worktree view, quick capture, cross-project inbox, Home counts |
| **Dropped** | Focus mode, message timeline |
| **Not built** | Provisions for messages from outside agents |

## 9. What v2 replaces, refines and keeps

### Replaced or refined

| v1 item | Was | In v2 |
| --- | --- | --- |
| Decision 4b, process ownership | Single TUI app; daemon later | GUI process hosts the core in-process; daemon still later |
| Decision 6a, pane model | Buffers, fixed layout, splits later | Three zones (sidebar, main view, chat docked right); exact layout open |
| Decision 6b, project focus | One active project | Home lists all projects; a project can open in a second window |
| Decision 11, tasks | User tasks with agents under them; In progress when a slice starts, Done when the user marks it | Every task belongs to an effort; Dovetail sets status (To do → In progress → Review → Done); starting an agent needs a task |
| Decision 12, worktrees | Coordinator in the main checkout | Coordinator in the project's state folder with every repo attached; a slice works in exactly one repo |
| Decision 15, layout B (+ ADR 0006, ADR 0008) | Conversation left, inbox over tree right | Sidebar (inbox over efforts tree), main view, coordinator chat docked right; not settled |
| Decision 16, extra views | Board, worktree view, focus mode | Board and worktree view later; focus mode dropped |
| Decision 16a–c, readability | Terminal cells, 120-col column, transparent | Superseded by the GUI; details with the layout |
| Decision 17, daily cycle | Morning review, end-of-day | Later |
| Decision 21, quick capture | `<leader>t`, `dovetail add`, Unsorted | Later; "Unsorted" is now an effort |
| Decision 22, context fill / handoff | Coordinator writes a handoff | A fresh coordinator gets every effort's "where was I" card as its handoff |
| Decision 23, visual maps | Kitty image canvas + outline | Later; a map can sit on a canvas as a card; maps and canvas stay separate models |
| Message timeline (Decisions 3, 8; digest §8) | Planned buffer | Dropped |
| ADR 0005, explicit projects, any folder | A project is a folder, found from the current folder | Project is a named workspace, never found from a folder; holds 0..n repos |
| ADR 0009, profiles | A profile per project | Same mechanism, now called the project's **account** |
| ADR 0011, side chats; ADR 0017, tangents | Built in the TUI | Later |
| ADR 0012, end-of-day handoff | | Later (daily review) |
| ADR 0013, focus mode | | Dropped |
| ADR 0015, research capture | Research-only project kind; finding = raw report | Research output arrives as canvas cards; long reports keep a reading view; project kind see Open points |
| ADR 0018, reading mode | TUI layout preset | A reading view in the main view |
| ADR 0019, Companion (egui) | Optional window; never the only way | Replaced: the Tauri GUI is the app |
| ADR 0020, GUI main client | egui GUI, TUI remote kit, `dovetail serve` | Tauri + React; TUI frozen, not a remote kit; core in the GUI process |
| AGENTS.md: `ratatui` + `crossterm` | The UI toolkit | Bind only the frozen TUI |
| AGENTS.md: "any terminal", kitty graphics | Hard constraint | Applies only to the frozen TUI |
| AGENTS.md: TUI redraw rules, 100 MB RSS target | Hard constraint | TUI target dropped; GUI budget 500 MB (window + one preview page) |
| AGENTS.md: transparent terminals | Hard constraint | Applies only to the frozen TUI |
| v1 open checks | All of them | Only core checks stay: worktree cleanup, out-of-memory recovery, subagent cost. M16 working week dropped |

### Kept

- Core and UI talk only by message-passing: `Snapshot` and events out, `Command`s in, `apply()`.
- Claude Code behind the `SessionDriver` trait; protocol JSON parsed in `dovetail-driver`; the stream rules (digest §3–§6).
- Worktree rules: one worktree per slice, created by Dovetail's hook outside the repo, cleaned up by Dovetail.
- Nothing in user repos (with ADR 0015's opt-in research folder exception). Hooks, settings, MCP config and role prompts are launch flags. Never bare mode.
- One DB thread; `rusqlite` bundled, WAL.
- MCP tool handlers in the core; tool suites (ADR 0016).
- Headless sessions, `--permission-mode auto`, approvals over the control protocol.
- Keys and commands resolve to one `Action` enum through one keymap table.
- Per-session cgroup memory caps.

## 10. Database (proposed shape)

Approved by the user as the direction. Column names and types are for the implementation plan to finalise. Migrations only add: nothing existing is deleted or rewritten.

### Today (checked against `crates/dovetail-core/src/db/migrations/`)

```
registry.db
└── projects (id, name, kind code|research, repo_path, git_common_dir, last_opened_ms)
              └─ one repo baked into the project row (registry 0001–0003)
    (the profile is not here: it is set in the project's recipe TOML, ADR 0009)

state.db (per project)
tasks ◄── agents.task_id              (nullable)
      ◄── research_rounds.task_id
      ◄── findings.task_id            (+ round_id)
      ◄── reports.task_id
      ◄── decisions.task_id
      ◄── maps.task_id                (nullable)
conversation_entries (agent_id, …)    ─┐ only agent_id: a chat line does not
inbox_items          (agent_id, …)    ─┘ know which line of work it belongs to
```

### v2

```
registry.db
├── projects (id, name, …, account)           + account
└── repos    (id, project_id, name, path)     NEW, 0..n per project

state.db
efforts (id, title, kind, question_or_goal, status, where_was_i)          NEW
├── criteria (id, effort_id, text, state open|claimed|met, evidence)      NEW
├── tasks    + effort_id, + criterion_id
│   ├── agents + repo_id
│   └── research_rounds / findings / reports / decisions   (unchanged, still on task_id)
├── conversation_entries + effort_id                          (the effort tag)
└── canvases (id, effort_id, title, framework)                            NEW
    └── cards (id, canvas_id, kind, x, y, ref_id, text)                   NEW
        └── card_links (from_card, to_card, label)                        NEW
```

### What each addition is for

| Addition | For |
| --- | --- |
| `repos` | A project holds zero or more repos. The repo columns leave `projects` (moved into `repos`) |
| `projects.account` | The project's Claude account, recorded on the project |
| `efforts` | One line of work; `where_was_i` holds the "where was I" card |
| `criteria` | Acceptance criteria. `claimed` = the agent says it is met and it waits for the user's approval |
| `tasks.effort_id` | Every task belongs to an effort |
| `tasks.criterion_id` | Which criterion a task serves; NULL is the drift flag |
| `conversation_entries.effort_id` | The effort tag, so the chat filter is a lookup |
| `canvases`, `cards`, `card_links` | Canvases per effort, their cards and the links between cards |
| `agents.repo_id` | Which repo a slice's worktree belongs to |

### Example rows

| Table | Row |
| --- | --- |
| `efforts` | 7 · "Payment providers" · research · "Which provider fits EU cards?" |
| `criteria` | 3 · effort 7 · "Fees compared for 3 providers" · claimed · evidence: finding 41 |
| `tasks` | 52 · "Compare Stripe, Adyen, Mollie" · effort 7 · criterion 3 |
| `findings` | 41 · task 52 (as today) |
| `canvases` | 4 · effort 7 · "Providers" · options compare |
| `cards` | 90 · canvas 4 · finding · ref 41 · text NULL |
| `conversation_entries` | coordinator line "Fees are in; Mollie is cheapest for EU cards." · effort 7 |

```
effort 7 ──► criterion 3 (claimed, evidence finding 41)
   │              ▲
   └──► task 52 ──┘ serves
          └──► finding 41 ◄── card 90 (canvas 4, ref 41)
chat line ── effort_id 7 ──► shows in the "Payment providers" thread
```

### Design choices

1. **Cards point, they do not copy.** A card points at a row (finding, source, decision, map, task) through `ref_id`. Only ideas and questions, which exist nowhere else, keep their own `text`.
2. **Research stays attached to tasks.** A research effort holds one or more question tasks; rounds, findings and reports hang off those as today. No research data moves.
3. **Migration adds, never converts.** Each project gets one "Unsorted" effort and every existing task gets its id. Existing chat lines get no tag and show under "All".

Note for the plan: SQLite cannot drop or relax columns in place, so moving the repo columns out of `projects` needs the table rebuild that registry migrations 0002 and 0003 already used. `inbox_items` gains no `effort_id` in this shape.

## Open points

Not decided yet. Do not build on a guess.

1. **Layout and keymap.** The three zones (sidebar with inbox over efforts tree, main view for canvas / viewer / reading, coordinator chat docked right) and the keymap are being designed with the user in a separate clickable-mock session.
2. **Multi-repo coordinator launch.** How the coordinator is started in the state folder with every repo attached as an extra directory (flags, what happens when a repo is added or removed while it runs, role prompt wording).
3. **How variants are declared.** The format and place of the agent's variant list, and how the panel switches variants in the running preview.
4. **Session naming scheme.** Names must be unique and include project and effort; the exact format is open.
5. **Project kind.** ADR 0015's `code` / `research` project kind against "a project holds any number of repos, including none": whether the kind stays.
6. **Research findings vs finding cards.** How ADR 0015's stored findings, sources and reports map onto finding and source cards and the reading view.
7. **P-levels and buckets** (Decision 17e). Not mentioned in v2; whether they stay alongside efforts.
8. **Preview command format** in the project recipe, and where the recipe lives now that a project can hold several repos.
9. **Running-effort limit**: what counts as a "running" effort, and the config key.
10. **Code landscape tool**: its MCP name, suite and output shape.
11. **Memory spike**: procedure and pass/fail for the 500 MB budget.

### Proposed: brainstorming on the canvas (not decided)

The user wants brainstorming in Dovetail to work like the grilling session that produced this design, with diagrams central to following a design. These are proposals, not decisions. Items 1 and 5 go beyond what was agreed: a new framework and a new card kind.

1. **"Decision rounds" framework** (new, a fourth framework). Question cards, each with the agent's recommended answer and a state: open, settled, needs explanation, parked. Questions are asked in rounds. A partial answer moves a question to the next round without blocking the rest. Answers may reshape later questions.
2. **Settled column.** It fills as questions close, and doubles as the effort's "where was I" card.
3. **"Explain" on a question card.** The agent attaches a diagram and one worked example to that card.
4. **Glossary card.** Pinned on every canvas, created at the start, holding the agreed keywords.
5. **Diagram card kind** (new). Shows today vs proposed; where the subject is code, the "today" side is drawn from the code landscape tool.
6. **"Write this up" on the settled column.** Starts a cheaper session with the settled summary as its brief.

```
round 2                                         settled
┌ Q: one chat per effort? ────────┐             ┌──────────────────────────┐
│ recommend: one per project      │             │ ✓ project holds 0..n repos│
│ state: needs explanation  [explain]           │ ✓ inbox per project       │
└─────────────────────────────────┘             │ ...                       │
┌ Q: task status by hand? ────────┐             └── [write this up] ────────┘
│ recommend: Dovetail sets it     │
│ state: open                     │             glossary (pinned)
└─────────────────────────────────┘             effort · repo · slice · ...
```

### Edits other docs will need

Not made in this change.

**`AGENTS.md`**

- "What this is": a Tauri GUI for projects with efforts, not "a keyboard-driven terminal UI … on one feature".
- Reading order: add `docs/design-v2.md` above `docs/design.md`.
- "Known conflicts": replace the ADR 0020 egui note with v2's Tauri + React; layout per the mock session.
- Hard constraints: `ratatui` + `crossterm`, "any terminal", kitty graphics, redraw rules and transparent terminals bind the frozen TUI only; add Tauri 2, React + TypeScript, React Flow; GUI memory budget 500 MB.
- Architecture rules: crate list gains the Tauri app crate and the web frontend; core hosted in the GUI process.
- Dogfooding: role prompts must describe efforts, effort tags, "where was I" cards and the multi-repo coordinator.
- Scope: target is v2's first GUI release, not v1.

**`docs/design-digest.md`**

- Header: point to `docs/design-v2.md` as the baseline that wins.
- §1 decision log: mark the replaced decisions from the table above.
- §2 architecture diagram: GUI process hosting the core.
- §7 data: project not tied to a folder; projects with 0..n repos; efforts; a project is no longer identified by a folder or git common dir (a repo may still be).
- §8 coordination: slice reports through Dovetail's tools; message timeline dropped.
- §10 UI essentials: Layout B replaced (pending the mock).
- §12 roadmap: v2 release split.

# Research feature

Spec for v1.1's first milestone. Decisions: [ADR 0015](adr/0015-research-capture-rounds-reports.md) (capture, rounds, reports, files), [0016](adr/0016-tool-suites.md) (tool suites), [0017](adr/0017-tangents-and-side-panel.md) (tangents, side panel), [0018](adr/0018-reading-mode-layout.md) (reading mode). Order of work: [`docs/plan/research-sessions.md`](plan/research-sessions.md). Where this file and an ADR differ, the ADR wins. Items marked **open** wait for the user's call.

**Status (2026-11-11):** R1 to R5 are built (PRs #49, #51, #50, #55, #56), with the follow-ups of PRs #52 to #54, #57 and #58; only R6 (tangents) and R7 are left. The user's decisions of 2026-11-11 are in the ADRs' "Amended" paragraphs; where this file still describes the pre-decision design (marked below) the ADR wins.

## 1. Purpose

Most of the user's work is research: business opportunities, whole features, specific questions. Subagents go out with criteria, report to the coordinator, and the user reads what the coordinator surfaces. Full findings, sources, ideas and decisions get lost on the way. The goal: **make decisions backed by sources, at macro and micro scale, without losing anything.**

## 2. Kinds

| Kind | What it is | Home | Written by |
| --- | --- | --- | --- |
| Source | A page or file an agent actually fetched or read | Cached copy, listed with each finding | Dovetail, automatically |
| Finding | One subagent's full raw report | `findings/<n>-<agent>.md` | Dovetail, automatically |
| Report | The curated synthesis the user reads | `report.md`, citing findings and sources by id | Coordinator |
| Idea | "We could also…" | A round, or a task in Unsorted marked "follow-up of" | Coordinator proposes, user accepts |
| Decision | "We're going with X" | Task decision log; architectural ones become an ADR | User, or an agent with the user's OK |

Ideas and decisions never live inside reports. Reports link to them.

## 3. Data model additions

All in the project's `state.db`, except `tangents` (global, `registry.db`). Migrations continue after `0012_maps`: state `0013_research`, `0014_decision_evidence`; registry gets its next number for `tangents` (check `db/migrations/registry/`). Written in the style of implementation-plan §5; Rust types go in `dovetail_api` (plain data, serde) so the daemon split stays possible.

```sql
-- 0013_research
CREATE TABLE research_rounds (
  id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id),
  n INTEGER NOT NULL,                       -- 1-based per task
  opened_ms INTEGER NOT NULL, closed_ms INTEGER,
  brief TEXT NOT NULL,                      -- the user's criteria, also written to brief.md
  dir TEXT NOT NULL,                        -- rounds/01-2026-11-10, relative to the research dir
  UNIQUE(task_id, n));

CREATE TABLE findings (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES tasks(id),
  round_id INTEGER REFERENCES research_rounds(id),   -- NULL: quick finding, no round
  n INTEGER NOT NULL,                                -- 1-based within the round (or the task)
  agent_id TEXT NOT NULL,                            -- agents.id of the subagent
  label TEXT NOT NULL,                               -- slug used in the file name
  description TEXT,                                  -- the Agent call's description
  path TEXT NOT NULL,                                -- rounds/01-…/findings/01-<label>.md, relative to the task folder
  body TEXT NOT NULL,                                -- the full output; also on disk
  body_from TEXT NOT NULL,                           -- which stream field the body came from (§5.4)
  subagent_type TEXT, started_ms INTEGER, tokens INTEGER,
  created_ms INTEGER NOT NULL);

CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,                       -- 'web' | 'file' | 'search' (a WebSearch itself)
  locator TEXT NOT NULL,                    -- URL, absolute file path, or search query
  tool TEXT NOT NULL,                       -- 'WebFetch' | 'WebSearch' | 'Read' | 'Grep' | 'Glob'
  query TEXT,                               -- WebFetch's prompt, WebSearch's query
  content TEXT NOT NULL,                    -- what the tool returned, so the cache can be rebuilt
  cached INTEGER NOT NULL,                  -- written to sources/<id>.md (Grep/Glob: no)
  is_error INTEGER NOT NULL DEFAULT 0,
  content_hash TEXT NOT NULL,               -- dedups the same page fetched twice
  fetched_ms INTEGER NOT NULL,
  UNIQUE(locator, content_hash));

CREATE TABLE finding_sources (
  finding_id INTEGER NOT NULL REFERENCES findings(id),
  source_id INTEGER NOT NULL REFERENCES sources(id),
  tool_use_id TEXT NOT NULL,                -- the stream's tool_use id, for audit
  PRIMARY KEY(finding_id, source_id, tool_use_id));

CREATE TABLE reports (
  id INTEGER PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id),
  round_id INTEGER REFERENCES research_rounds(id),   -- the round it was written after
  at_ms INTEGER NOT NULL, body TEXT NOT NULL,
  uncited INTEGER NOT NULL DEFAULT 0,       -- validation results kept with the version
  unlabelled_figures INTEGER NOT NULL DEFAULT 0);

ALTER TABLE tasks ADD COLUMN research_kind TEXT;     -- NULL | 'question' | 'decision'
ALTER TABLE tasks ADD COLUMN altitude TEXT;          -- NULL | 'macro' | 'micro'

-- research-only project (ADR 0015 open point, decided 2026-11-11): registry migration
-- 0003_project_kind adds projects.kind ('code' | 'research') and makes repo_path nullable
-- state.db config: research_dir overrides the default (read from the recipe, not stored)

-- 0014_decision_evidence
CREATE TABLE decision_evidence (
  decision_id INTEGER NOT NULL REFERENCES decisions(id),
  finding_id INTEGER REFERENCES findings(id),
  source_id INTEGER REFERENCES sources(id),
  CHECK (finding_id IS NOT NULL OR source_id IS NOT NULL));

-- tangents: SUPERSEDED (2026-11-11, ADR 0017). Tangents is a built-in research-only project, so a
-- tangent is a task in its Unsorted list with ordinary findings and sources; there is no tangents
-- table and no tangents.db. Not built (R6).

-- unfiled coordinator findings (built, PR #53): own table `unfiled_findings` (no task), filed or
-- dropped from Needs you; filing gives the finding a new F id
```

A tangent's sources reuse `sources`/`finding_sources` of the Tangents project's `state.db` (decided 2026-11-11). v1.1 columns still to come: FTS rows for findings and reports (search, Decision 20).

`Snapshot` carries `research: Vec<ResearchSummary>` (per task: rounds, finding and source counts, report time); content loads through `Command::LoadResearch { task, what }` answered by `Event::ResearchLoaded`, as maps do. `Event::ResearchUpserted(ResearchSummary)` keeps the library current. Built in R1 (`0013_research`, which also holds `reports` and the two `tasks` columns); the file copy of a cached source is always `sources/<id>.md`, so no `cache_path` column is kept, and the content is stored so a missing file can be rebuilt.

## 4. Folder layout

```
research/<task-id>-<slug>/
  report.md
  rounds/
    01-2026-11-10/
      brief.md
      findings/01-<agent>.md
    02-2026-11-11/
      …
  sources/<source-id>.md          # cached copies, by id
```

- **Default root:** `~/.local/share/dovetail/projects/<project-id>/research/`. (The brief writes `~/.local/share/dovetail/<project>/research/`; the existing layout puts projects under `projects/<id>/`, so the same place is used.)
- **Opt-in root:** `research_dir = "docs/research"` in the project's recipe, `~/.config/dovetail/projects/<id>.toml`, relative to the repo root, or absolute. Dovetail's own `config/projects/dovetail.toml` sets it to `docs/research/` (ADR 0015). Slices and the coordinator never write there themselves; Dovetail does. Quick findings (no round) go to `research/<task-id>-<slug>/findings/` directly.
- **Tangents:** the built-in research project's folder, `~/.local/share/dovetail/projects/tangents/research/<task-id>-<slug>/` (ADR 0017, amended 2026-11-11; not built). A research-only project's root is `~/.local/share/dovetail/projects/<id>/research/` (built, PR #54).
- Files are written by the core after the database row, atomically (temp file and rename). The database is the source of truth; the files are a readable copy and can be rebuilt.

## 5. Capture pipeline

Nothing here is new protocol. It uses events the driver already parses (`dovetail-driver/src/event.rs`) and the core's session state (`session/state.rs`).

1. **Round open.** `research_start_round(task, brief)` (R2; in R1 `Command::StartRound`, `:round <task> <brief>`) creates a `research_rounds` row and `brief.md`; the core remembers "round R is open for the coordinator". At most one round is open at a time; `:round close` (`Command::CloseRound`) closes it, and subagents already launched in it are still filed under it.
2. **Which subagents belong.** A `DriverEvent` assistant `ContentBlock::ToolUse { name: "Agent" | "Task", .. }` from the coordinator while a round is open starts a candidate; its `tool_use_id` is the key. `TaskEvent { phase: Started, tool_use_id, subagent_type, description }` confirms a background subagent and names it. Every later event of that subagent carries `parent_tool_use_id` equal to that id (digest §3).
3. **Sources.** For each `ToolUse` tagged to the subagent with `name` in `WebFetch`, `WebSearch`, `Read` (and `Grep`/`Glob` listed but not cached), the core records the input (`url` and `prompt`, `query`, `file_path`); the matching `DriverEvent` `ToolResult { tool_use_id, content, is_error }` gives what the tool returned. Error results are kept but marked. Content is cached to `sources/<id>.md`, deduplicated by locator and hash. A `Source` is created from the **tool call**, never from links in prose. Per spike S7: a **`WebFetch` result is a model's answer to the agent's own `prompt`, not the page**, so the cache stores it with that prompt and says "what the agent saw"; a **`WebSearch` result** gives one source per URL of its `Links: [...]` JSON array, and the rest of the result (a model's synthesis that can carry third-party text) is kept whole as one `search` source marked untrusted.
4. **Finding body.** In priority order (S7, Claude Code 2.1.316, where every subagent ends with a `SubagentHandback` call): (a) `task_notification.handback_report.text`; (b) for a foreground call, the `Agent` result's `tool_use_result.handbackReport.text`; (c) the `SubagentHandback` call's `message`; (d) the `Agent` result content, only when the subagent used no handback (with a handback it is a boilerplate "delivered to you as a message" line); (e) the subagent's assistant text; (f) `task_notification.summary`, only when `handback` is not `send` (otherwise it is the same boilerplate); (g) last resort, the `assistant` lines of `output_file`, a symlink to the subagent's transcript. **Never copy `output_file` wholesale**: it also holds the request context, the user's email and connector lists included. The row records which one was used (`body_from`).
5. **File the finding.** On subagent completion (`task_updated` with a final status, `task_notification`, or the foreground `Agent` `ToolResult`) — **never on a `result`**: a background subagent keeps streaming after the coordinator's turn ends — insert the `findings` and `finding_sources` rows, then write `findings/<n>-<label>.md` with front matter (finding id, task, round, agent, subagent type, description, started and finished time, tokens, `body_from`, `sources: [ids]`), a Sources section listing each source with its locator, tool and a link to its cached copy, and the report. A better body arriving later (the notification after `task_updated`) rewrites the row and file.
6. **Context isolation.** The coordinator's context is whatever Claude Code already gave it (the `Agent` tool result, the summary). Dovetail sends **nothing** to the coordinator about findings and does not add them to its prompt. The coordinator reads findings only by choice, through the `research_get` read (below) or a file path it is told.
7. **No round open.** *(Amended 2026-11-11, PR #53: a research-shaped subagent of the **coordinator** with no round open is stored with no task as an unfiled finding, a Needs-you item filed into a task with `y`/Enter or dropped with `d`. A second round requested while one is open is queued and opens in order when the open one closes.)* A research-shaped subagent (`WebFetch`/`WebSearch` use) of a session that works on a task (a slice) becomes a quick finding with `round_id NULL`, filed under `findings/` of the task folder; the coordinator has no task of its own, so with no round open its subagents are not filed. The user can promote a quick finding to a round (`:round` on the finding), which creates the round row and moves the file: **not built in R1** (it needs R3's listing to point at a finding).
8. **What the stream cannot give.** `WebFetch` results are a model's answer about the page, not the raw bytes; the cache is "what the agent saw", and raw pages exist only if Dovetail fetched the URL itself, which it does not. Not verified by S7: `Read` truncation of long files, a subagent that ends without a handback, a failing `WebFetch`'s error shape. Whether the research suite's own tools work inside background subagents matters only if subagents themselves call `research_*`, which they do not in this design: only the coordinator does (S7 (e): a subagent's `mcp__dovetail__*` call streams like any tool).

Fake-driver coverage: S7's `research_round_foreground.ndjson` and `research_round_background.ndjson` (plus `research_websearch.ndjson`, `research_mcp_subagent.ndjson`) in `crates/dovetail-driver/tests/fixtures/`, replayed through the real core in `crates/dovetail-core/tests/research.rs`.

## 6. Tools

Served by `dovetail mcp` under the Research suite, coordinator only (ADR 0016). Handlers live in `dovetail-core/src/tools/research.rs`. Errors are specific strings in `isError` results, as `maps_draw` does.

### `research_start_round`

```json
{"type": "object", "properties": {
  "task":  {"type": "integer", "description": "Id of an existing task (the question)."},
  "brief": {"type": "string", "description": "The user's criteria for this round, verbatim where possible."}
}, "required": ["task", "brief"]}
```

Returns `{round: n, dir: "rounds/01-2026-11-10", status: "open"}`. Errors: unknown task; **a round is already open for this task** (close it first, or `research_write_report` closes it); empty `brief`; the task is archived.

### `research_write_report`

```json
{"type": "object", "properties": {
  "task": {"type": "integer"},
  "body": {"type": "string", "description": "The whole report in markdown, citing findings as [F3] and sources as [S12]. Rewrites the previous report."}
}, "required": ["task", "body"]}
```

Validation (section 7) runs before anything is written. Returns `{written: true, uncited: [..], unlabelled_figures: [..], citations_resolved: n}`; flagged problems do not refuse the write unless a citation does not resolve. Errors: **no open round** and no earlier round (nothing to report on); **a citation id that does not exist** (`[F9] does not resolve; findings in this task: F1..F4`); **a decision-template task missing a required section**; body empty. On success the open round is closed.

### `research_propose`

```json
{"type": "object", "properties": {
  "task":     {"type": "integer"},
  "kind":     {"type": "string", "enum": ["idea", "decision"]},
  "text":     {"type": "string", "description": "The idea, or the decision in one line."},
  "evidence": {"type": "array", "items": {"type": "string"}, "description": "Finding and source ids, e.g. [\"F3\", \"S12\"]. Required for a decision."},
  "why":      {"type": "string"},
  "alternatives": {"type": "array", "items": {"type": "object", "properties": {"option": {"type": "string"}, "why_not": {"type": "string"}}, "required": ["option"]}}
}, "required": ["task", "kind", "text"]}
```

Never writes directly: creates an inbox proposal item (new `InboxKind::Proposal { kind, text, evidence }`, Needs you). `y` accepts: an idea becomes a follow-up task in Unsorted or a new round brief (the item offers both, routing rule in ADR 0015); a decision is written through the existing `core_log_decision` path with `decision_evidence` rows. `d` drops it. The call returns at once, `pending`, as `project_start_slice` does; the answer reaches the coordinator as a short user message. Errors: **kind `decision` with no `evidence`**; an evidence id that does not resolve; unknown task; no `why` is allowed.

Read helpers (Research suite, coordinator only; the Ask session has no suites and reads the files, PR #56): `research_get(task, what)` where `what` is `report`, `round:N`, `finding:N`, `source:N`. Needed so the coordinator can cite without keeping content in context; kept small.

## 7. Report template and validation

Plain-question tasks get a normal report. Tasks with `research_kind = 'decision'` use:

```markdown
# <the question>

> What changed in round N: <two lines, not on round 1>

**Recommendation:** …
**Confidence:** high | medium | low — <one sentence>        <!-- stated by the coordinator -->
**Evidence:** 7 sources, 4 independent                      <!-- appended by Dovetail -->

## Key figures
| Figure | Value | Basis |
| --- | --- | --- |
| Market size | $4.2B | **Sourced** [S3] |
| Per-seat revenue | $31 | **Derived**: 4.2B ÷ 135M seats [S3][S9] |
| Churn | 6 % | **Estimate**: assumed from comparable SaaS; reasoning … |

## Options
### A — …
For: … [F2][S5]   Against: … [F4]

## What would change this
- … *(each line can become the next round's brief with one key)*

## Open questions
- … *(each becomes an idea, routed by the rule in ADR 0015)*
```

Dovetail checks, in order:
1. **Citations resolve.** Every `[Fn]` and `[Sn]` names a finding or source of this task. Failure refuses the write.
2. **Uncited claims.** A paragraph or table row in a body section with factual shape (a number, a named entity, "according to") and no citation is flagged: listed in the tool result and marked with a margin note in the reading buffer. Heuristic, never refusing.
3. **Figures.** Every number in Key figures has a label (**Sourced**, **Derived**, **Estimate**) and, for Sourced and Derived, a citation; Derived must show a calculation. Missing label or citation is flagged.
4. **Sections.** A decision template with a missing heading is refused.
5. **Source counts.** "Evidence: N sources, M independent" is computed by Dovetail and rewritten on save; independent means distinct registrable domains (files: distinct paths). The coordinator's own count is ignored.

## 8. Keybindings

Checked against `crates/dovetail-tui/src/keymap.rs`. **open** means a clash or a decision for the user; nothing here is bound until resolved.

| Key | Meaning | Scope | Status |
| --- | --- | --- | --- |
| `<leader>b` | Board (the brief's "side panel" use is dropped) | global | decided 2026-11-11: the Board keeps it |
| `<leader>?` | Side panel | global | bound (PR #52, `Action::ToggleSidePanel`): decided 2026-11-11 |
| `<leader>r` | Reading mode | global | bound (M21, `Action::ToggleReadingMode`; also `:read`) |
| `Esc` | Leave reading mode | reading mode | bound (M21): Normal mode in any reading-mode column; in Ask's chat bar it only returns to Normal (decided 2026-11-11) |
| `i` | Ask about the open report | reading column, Ask | bound (M21, `Action::AskAboutReport`); in the library `i` does nothing (`Action::Ignore`) |
| `]s` / `[s` | Next / previous citation | reading column, Ask | bound (M21) |
| `gd` | Open cited finding or source | reading column, Ask | bound (M21) |
| `gx` | Open the source in a browser | reading column, Ask | bound (M21, `Command::OpenExternal`); the keymap test now uses `gq` as its unbound example |
| `Ctrl-o` | Back to the document read before `gd` | reading column | bound (M21) |
| `<leader>sq`, `:ask` | Side chat, opened in the side panel | global | bound (PR #52); they open in the panel, not the main region |
| `<leader>y` | (was: copy a side chat's answer to the coordinator) | global | removed (PR #52) |
| `R` (proposed) | Run next round from the "What would change this" line under the cursor | report buffer | proposed, not built; the user has not decided |
| `y` / `d` | Accept / drop a proposal | inbox | existing keys |

Inbox keys, the leader and which-key are unchanged.

## 9. Reading mode layout

```
┌ library ─────────────┬ reading ─────────────────────────────┬ ask ───────────────┐
│ ▾ business           │                                      │ you                │
│   ▾ #1231 pricing    │   Should we price per seat?          │ Why 6 % churn?     │
│     ● report.md      │                                      │                    │
│     ▸ rounds (2)     │   Recommendation: per seat [1][3]    │ answer             │
│       01 …           │   Confidence: medium                 │ From [3], a 2025   │
│       02 …           │   Evidence: 7 sources, 4 independent │ survey, …    [3]   │
│     ▸ findings (6)   │                                      │                    │
│ ▸ Decisions          │   Key figures                        │                    │
│ ▸ Tangents (2)       │   Market size  $4.2B  Sourced [3]    │ › i to ask         │
│ ▸ Docs · ADRs        │                                      │                    │
├──────────────────────┴──────────────────────────────────────┴────────────────────┤
│ READING  #1231 pricing · round 2 · 7 sources · [3] ◂ ]s/[s · gd open · gx browser │
└───────────────────────────────────────────────────────────────────────────────────┘
```

As built in M21 (ADR 0018, "Decided 2026-11-11"): no Tangents section until tangents exist (R6); Ask is a read-only session (`AgentKind::Ask`), not a subagent.

120-column centred reading column, solid background on, citations as coloured `[n]` markers. Ask is a read-only subagent started on `i`, its context the open report and the sources it cites (loaded by the core from the research folder), answers citing with the same markers. Layout rules: ADR 0018.

## 10. Milestones

Each ends with `scripts/check.sh` passing. "Done when" boxes are ticked by the integrator after checking. Session prompts are in `docs/plan/research-sessions.md`.

- **R1 Capture (core). Done, PR #49.** Round, finding and source storage; automatic capture from the stream; files on disk; research dir from the recipe. Done when: a fake-driver replay of a subagent with `WebFetch` and `Read` produces `findings/01-<agent>.md` with its sources listed; the same source fetched twice is one `sources` row; nothing is sent to the coordinator's session (asserted on the `FakeDriver` command log); a restart keeps rounds and findings; the opt-in research dir writes under `docs/research/` of a temp repo and nothing else in it.
- **R2 Research suite. Done, PR #51.** `research_start_round`, `research_write_report` with validation, `research_propose` → inbox proposal, `research_get`, decision evidence in the decision log, coordinator role-prompt lines. Done when: the fake-driver flow writes `brief.md` and `report.md`; every `[n]` resolves; an unresolved citation, a missing round, and a `decision` without evidence each return the specified error; an accepted decision shows evidence links in the task detail.
- **R3 Rough reading. Done, PR #50.** Reports, rounds and findings open in existing buffers (task detail lists them; `Enter` opens one in the conversation-style reader or `e` hands it to nvim). Done when: each stored artifact is reachable from the task detail and renders with the reading column; `[n]` markers are coloured; no new layout.
- **R4 Tool suites and rename (ADR 0016). Done, PR #55.** Done when: roles list suites; old names answer with a deprecation for one release; role prompts, `live-checks.sh` and tests updated; `allowed_tools` derived from suites.
- **R5 Reading mode. Done, PR #56.** Three-column layout, citations, `]s` `[s` `gd` `gx`, Ask column. Done when: `<leader>r` toggles and `Esc` leaves; `gx` opens a source (browser launched by the core); `gd` opens the cited finding; the Ask subagent's answers carry markers that resolve; HTML converts to markdown; layout snapshots at 100, 160 and 220 columns; the side panel key decision is applied.
- **R6 Tangents and the side panel.** The side panel is done (PR #52). Left (not built): Tangents as a built-in research project (always present, last in the switcher with its own marker, cannot be archived, tangents as tasks in Unsorted), answer destinations (attach to the current task as a finding, or Tangents), the "new tangents" morning item (file into a project, start a task, drop). Done when: a tangent is saved in Tangents with its sources and shows in the next morning's "new tangents" item; filing it into a project creates the finding or task; Tangents cannot be archived. (`<leader>y` was removed.)
- **R7 Role-prompt reliability.** *(Not run; the user's.)* Extends S6: does the coordinator, given a short role prompt, call the research suite (start a round before fanning out, write a cited report, propose instead of deciding)? Done when: 3 of 3 runs per case in a scratch repo with tiny prompts, recorded in `docs/spikes/`.

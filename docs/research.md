# Research feature

Spec for v1.1's first milestone. Decisions: [ADR 0015](adr/0015-research-capture-rounds-reports.md) (capture, rounds, reports, files), [0016](adr/0016-tool-suites.md) (tool suites), [0017](adr/0017-tangents-and-side-panel.md) (tangents, side panel), [0018](adr/0018-reading-mode-layout.md) (reading mode). Order of work: [`docs/plan/research-sessions.md`](plan/research-sessions.md). Where this file and an ADR differ, the ADR wins. Items marked **open** wait for the user's call (PR body, "Questions for you").

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
  path TEXT NOT NULL,                                -- findings/01-<label>.md, relative
  body TEXT NOT NULL,                                -- the full output; also on disk
  created_ms INTEGER NOT NULL);

CREATE TABLE sources (
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,                       -- 'web' | 'file'
  locator TEXT NOT NULL,                    -- URL or absolute file path
  tool TEXT NOT NULL,                       -- 'WebFetch' | 'WebSearch' | 'Read' | ...
  cache_path TEXT,                          -- sources/<id>.md, relative; what the tool returned
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

-- research-only project, if the user approves (ADR 0015 open point)
-- registry.db: ALTER TABLE projects ADD COLUMN kind TEXT NOT NULL DEFAULT 'code';  -- 'code' | 'research'
-- state.db config: research_dir overrides the default (read from the recipe, not stored)

-- 0014_decision_evidence
CREATE TABLE decision_evidence (
  decision_id INTEGER NOT NULL REFERENCES decisions(id),
  finding_id INTEGER REFERENCES findings(id),
  source_id INTEGER REFERENCES sources(id),
  CHECK (finding_id IS NOT NULL OR source_id IS NOT NULL));

-- registry.db (tangents are global, ADR 0017)
CREATE TABLE tangents (
  id INTEGER PRIMARY KEY, question TEXT NOT NULL, answer TEXT, dir TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  state TEXT NOT NULL DEFAULT 'new',        -- 'new' | 'filed' | 'task' | 'dropped'
  filed_project TEXT, filed_task INTEGER);
```

Sources of tangents reuse `sources`/`finding_sources` in a `tangents.db`, or `tangents` carries its own rows: **open**, decided in R6. v1.1 columns still to come: FTS rows for findings and reports (search, Decision 20).

`Snapshot` carries `research: Vec<ResearchSummary>` (per task: rounds, finding and source counts, report time); content loads through `Command::LoadResearch { task, what }` answered by `Event::ResearchLoaded`, as maps do. `Event::ResearchUpserted(ResearchSummary)` keeps the library current.

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
- **Tangents:** `~/.local/share/dovetail/tangents/<id>-<slug>/{answer.md,sources/}`.
- Files are written by the core after the database row, atomically (temp file and rename). The database is the source of truth; the files are a readable copy and can be rebuilt.

## 5. Capture pipeline

Nothing here is new protocol. It uses events the driver already parses (`dovetail-driver/src/event.rs`) and the core's session state (`session/state.rs`).

1. **Round open.** `research_start_round(task, brief)` creates a `research_rounds` row and `brief.md`; the core remembers "round R is open for coordinator session S".
2. **Which subagents belong.** A `DriverEvent` assistant `ContentBlock::ToolUse { name: "Agent" | "Task", .. }` from the coordinator while a round is open starts a candidate; its `tool_use_id` is the key. `TaskEvent { phase: Started, tool_use_id, subagent_type, description }` confirms a background subagent and names it. Every later event of that subagent carries `parent_tool_use_id` equal to that id (digest §3).
3. **Sources.** For each `ToolUse` tagged to the subagent with `name` in `WebFetch`, `WebSearch`, `Read` (and `Grep`/`Glob` listed but not cached), the core records the input (`url`, `query`, `file_path`); the matching `DriverEvent` `ToolResult { tool_use_id, content, is_error }` gives what the tool returned. Error results are kept but marked. Content is cached to `sources/<id>.md`, deduplicated by hash. A `Source` is created from the **tool call**, never from links in prose.
4. **Finding body.** In priority order: (a) the subagent's final report as the text of its handback — in the recorded fixture `subagent_background.ndjson` the subagent ends with a `SubagentHandback` tool call whose `message` is the report, and a foreground `Agent` call's `ToolResult` carries it; (b) `TaskEvent` `Notification { output_file, summary }` — `output_file` is the full transcript output on disk, `summary` is only a summary; (c) the concatenated assistant text of the subagent. The best available full text is the finding; the others are kept in the row if they differ.
5. **File the finding.** On subagent completion (`Notification`/`Updated` with a final status, or the foreground `Agent` `ToolResult`), write `findings/<n>-<label>.md` with front matter (agent, subagent type, task id, round, started and finished time, tokens, `sources: [ids]`) and the body, then insert `findings` and `finding_sources` rows. The file's header lists its sources with locators.
6. **Context isolation.** The coordinator's context is whatever Claude Code already gave it (the `Agent` tool result, the summary). Dovetail sends **nothing** to the coordinator about findings and does not add them to its prompt. The coordinator reads findings only by choice, through the `get_finding` read (below) or a file path it is told.
7. **No round open.** A research-shaped subagent (`WebFetch`/`WebSearch` use) on a task becomes a quick finding with `round_id NULL`; the user can promote it to a round (`:round` on the finding) which creates the round row and moves the file.
8. **What the stream cannot give.** `WebFetch` results are a model's rendering of the page, not the raw bytes; the cache is "what the agent saw". Subagents' `WebSearch` results are lists of titles and URLs. Whether `tool_result` bodies for these tools survive the stream unmodified, and whether `Read` results are truncated, is spike **S7**. Whether the research suite's own tools work inside background subagents (digest §13 still lists it open; `docs/spikes/mcp.md` shows a pass) matters only if subagents themselves call `research_*`, which they do not in this design: only the coordinator does.

Fake-driver coverage: a fixture `research_round.ndjson` (recorded in S7) replayed through `FakeDriver`.

## 6. Tools

Served by `dovetail mcp` under the Research suite, coordinator only (ADR 0016). Handlers live in `dovetail-core/src/tools/research.rs`. Errors are specific strings in `isError` results, as `draw_map` does.

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

Never writes directly: creates an inbox proposal item (new `InboxKind::Proposal { kind, text, evidence }`, Needs you). `y` accepts: an idea becomes a follow-up task in Unsorted or a new round brief (the item offers both, routing rule in ADR 0015); a decision is written through the existing `log_decision` path with `decision_evidence` rows. `d` drops it. The call returns at once, `pending`, as `start_slice` does; the answer reaches the coordinator as a short user message. Errors: **kind `decision` with no `evidence`**; an evidence id that does not resolve; unknown task; no `why` is allowed.

Read helpers (Research suite, coordinator and the Ask column's subagent): `research_get(task, what)` where `what` is `report`, `round:N`, `finding:N`, `source:N`. Needed so the coordinator can cite without keeping content in context; kept small.

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
| `<leader>b` | Side panel (brief) | global | **open**: bound to the Board (`Action::ShowBoard`, digest 16, UI brief line 132) |
| `<leader>?` | Alias for the side panel | global | **open**: free in code; documented as Decision 18's grilling toggle (README, digest). Alias is free |
| `<leader>r` | Reading mode | global | free in code (`<leader>pr` is RegisterProject, `r` is bound only in inbox scopes). Not open as a key; the mode is gated on R5 |
| `Esc` | Leave reading mode | reading mode | **open**: the UI brief reserves `Esc` for mode switching; scoped to Normal mode in reading mode |
| `i` | Ask about the open report | Ask column | **open**: `i` is `EnterInsert` globally; scoped to the reading buffer, resolves to Ask there |
| `]s` / `[s` | Next / previous citation | reading buffer | free (`]]`/`[[` exist, `]s` does not) |
| `gd` | Open cited finding or source | reading buffer | free (`gg` is the only `g` binding; `gx` appears in a keymap test as an *unbound* example, `an_unbound_key_is_unmatched…`, which must change when `gx` is bound) |
| `gx` | Open the source in a browser | reading buffer | free; the test above changes |
| `<leader>sq`, `:ask` | Side chat in the main region (ADR 0011) | global | **open**: behaviour changes if the panel replaces the main-region conversation |
| `R` (proposed) | Run next round from the "What would change this" line under the cursor | report buffer | proposed, not in the brief |
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

120-column centred reading column, solid background on, citations as coloured `[n]` markers. Ask is a read-only subagent started on `i`, its context the open report and the sources it cites (loaded by the core from the research folder), answers citing with the same markers. Layout rules: ADR 0018.

## 10. Milestones

Each ends with `scripts/check.sh` passing. "Done when" boxes are ticked by the integrator after checking. Session prompts are in `docs/plan/research-sessions.md`.

- **R1 Capture (core).** Round, finding and source storage; automatic capture from the stream; files on disk; research dir from the recipe. Done when: a fake-driver replay of a subagent with `WebFetch` and `Read` produces `findings/01-<agent>.md` with its sources listed; the same source fetched twice is one `sources` row; nothing is sent to the coordinator's session (asserted on the `FakeDriver` command log); a restart keeps rounds and findings; the opt-in research dir writes under `docs/research/` of a temp repo and nothing else in it.
- **R2 Research suite.** `research_start_round`, `research_write_report` with validation, `research_propose` → inbox proposal, `research_get`, decision evidence in the decision log, coordinator role-prompt lines. Done when: the fake-driver flow writes `brief.md` and `report.md`; every `[n]` resolves; an unresolved citation, a missing round, and a `decision` without evidence each return the specified error; an accepted decision shows evidence links in the task detail.
- **R3 Rough reading.** Reports, rounds and findings open in existing buffers (task detail lists them; `Enter` opens one in the conversation-style reader or `e` hands it to nvim). Done when: each stored artifact is reachable from the task detail and renders with the reading column; `[n]` markers are coloured; no new layout.
- **R4 Tool suites and rename (ADR 0016).** Done when: roles list suites; old names answer with a deprecation for one release; role prompts, `live-checks.sh` and tests updated; `allowed_tools` derived from suites.
- **R5 Reading mode.** Three-column layout, citations, `]s` `[s` `gd` `gx`, Ask column. Done when: `<leader>r` toggles and `Esc` leaves; `gx` opens a source (browser launched by the core); `gd` opens the cited finding; the Ask subagent's answers carry markers that resolve; HTML converts to markdown; layout snapshots at 100, 160 and 220 columns; the side panel key decision is applied.
- **R6 Tangents and the side panel.** Global Tangents space, one panel, destinations for an answer, morning-review listing. Done when: a tangent started with no project open is saved with sources and shows in the next morning review; filing it into a project creates the finding or task; `<leader>y` still works.
- **R7 Role-prompt reliability.** Extends S6: does the coordinator, given a short role prompt, call the research suite (start a round before fanning out, write a cited report, propose instead of deciding)? Done when: 3 of 3 runs per case in a scratch repo with tiny prompts, recorded in `docs/spikes/`.

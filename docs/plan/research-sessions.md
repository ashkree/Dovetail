# Research feature: where the build stands, and the sessions to build it

Written 2026-11-11 against `main` at `cf15fe2`. Spec: [`docs/research.md`](../research.md); decisions: ADRs [0015](../adr/0015-research-capture-rounds-reports.md) to [0018](../adr/0018-reading-mode-layout.md). Source of truth for v1 order of work stays `docs/implementation-plan.md`; `docs/plan/v1-status.md` is the user-facing summary of the same ground and was written the evening before this file.

## 1. Status report

Evidence is the "done when" boxes and "Merged:/Progress:" lines of `docs/implementation-plan.md` §6, its Daily-driver fixes table, `git log --merges`, and the files on `main`. A milestone with any unticked box is **partial**. `git branch -a --no-merged main` lists nothing, so there are no unmerged branches: `dovetail/m15-outline` merged as PR #42 and `dovetail/upgrade` as PR #41.

| Milestone | State | Evidence | What's next |
| --- | --- | --- | --- |
| S0 terms of use | done | `docs/spikes/terms-of-use.md`, passed 2026-11-01 | — |
| S1 driver crate | done | `docs/spikes/driver.md`; hand-written NDJSON layer; 15 fixtures in `crates/dovetail-driver/tests/fixtures/` | — |
| S2 inbound messaging | done | `docs/spikes/messaging.md`; no relay needed | — |
| S3 MCP behaviour | done | `docs/spikes/mcp.md`: all four checks pass, including a background subagent calling an MCP tool (the digest's §13 row "MCP tools in background subagents" is stale) | — |
| S4 cgroup | done | `docs/spikes/cgroup.md`; M13 OOM test run on the user's machine 2026-11-09 | — |
| S5 merman | partial | PR #33, four machine checks pass; **the look in kitty is the user's to judge** | user judges the look |
| S6 `ask_user` role prompt | not started | plan §1: "open"; box unticked under M9 | folded into R7 below for the research suite; the `ask_user` half is still open |
| M1 workspace, API, mock | done | all boxes ticked; `crates/dovetail-api`, `dovetail-mock` | — |
| M2 TUI shell | done | all boxes ticked; `docs/reviews/m2.md` | — |
| M3 conversation | done | all boxes ticked (one waived by the user) | — |
| M4 command mode, picker, capture | done | all boxes ticked; `crates/dovetail-tui/tests/m4.rs` | — |
| M5 storage, tracker | done | all boxes ticked; migrations 0001 to 0004; wave 1 track A | — |
| M6 driver, coordinator | partial | 4 boxes unticked: real streaming chat, interrupt, non-`auto` warning, child reaping. Code merged (waves 1 and 2) and used daily | user runs `scripts/live-checks.sh` |
| M7 live permissions | partial | 3 unticked: real manual-mode request answered, a held request not blocking another, mode change without restart | live checks |
| M8 slices, worktree hooks | partial | 7 unticked (real slice in a hook worktree, failed setup, long setup, `gh pr merge` ask rule, messaging, archive, "only branches") | live checks |
| M9 MCP bridge, tools | partial | 5 unticked incl. S6; tools exist: `ask_user`, `report_status`, `create_task`, `log_decision`, `start_slice`, `archive_slice`, `archive_task`, `add_project`, `list_tasks`, `get_task`, `update_task`, `draw_map` | live checks; S6 |
| M10 resume, install — self-hosting point | partial | rehearsal done 2026-11-06; one box unticked: "repeated once on this repo". Dovetail has built itself since 2026-11-09 (`v1-status.md`) | user ticks it |
| M11 subagents, to-dos | done | boxes ticked on fake-driver replays, **not checked live**; PRs #12, #13 | live check; R1 builds on this routing |
| M12 board, worktree view | done | all ticked; PR #28 and task #22 | — |
| M13 caps, context fill | done | all ticked; PRs #29 to #31 | one path (no user systemd) untried |
| M14 take-over | partial | PR #34; the real round trip is unticked | user tries it |
| M15 maps stage 1 | partial | (a) PR #32, (b) PR #35, (c) renderer PR #39, (d) swimlane PR #38, (e) shelf and outline PR #42 are merged; **(f) kitty canvas and MAP mode not merged** (scheduled overnight per `v1-status.md`); all 3 boxes unticked | (f); then a real agent draws both map types |
| M16 hardening | not started | all 3 boxes unticked. Pieces already in: focus mode (`<leader>z`, ADR 0013) | activity events, startup warnings, RSS (scheduled overnight); then the user's week |

### Built but not on the list

| Work | Evidence |
| --- | --- |
| Non-blocking `ask_user` | ADR 0007, PR #6 |
| Decision log and `log_decision` | ADR 0003 (migration `0001_init` `decisions`) |
| `create_task`, `start_slice`, `archive_slice`, `archive_task`, `add_project`, `list_tasks`, `get_task`, `update_task` | ADRs 0002, 0004, 0005; PR #11; `crates/dovetail/src/mcp.rs` |
| Explicit projects, any folder | ADR 0005 |
| Inbox over tree, conversation left | ADRs 0006, 0008, PR #14 |
| Profiles (a Claude account per project) | ADR 0009, PR #26, migration 0010 |
| Chat bar keys, completion, `:skills` | ADR 0010; `docs/spikes/slash-and-at.md`; commits 1a36dc1, 007af85 |
| Side chats | ADR 0011; commit 5f1abb7 |
| End of day and a fresh morning coordinator | ADR 0012; PR #36 |
| Focus mode | ADR 0013; commit 6b009ab |
| `:upgrade` / `dovetail upgrade` | ADR 0014; **built and merged**, PR #41 |
| Usage limits in the header | daily-driver fix #16, PR #17 |
| Daily-driver fixes #1 to #17 | implementation-plan table; PRs #4 to #18 |
| Review-wave-4 cleanup | PR #40 |

Note: the plan's own header still says "waves 1 and 2 in progress"; it is stale.

## 2. v1 prerequisites for research

| Needed | State | Needed by |
| --- | --- | --- |
| MCP bridge (`dovetail mcp`, tool handlers in core) | exists | R2 |
| Decision log (ADR 0003) | exists | R2 |
| Subagent routing from `parent_tool_use_id` and `task_*` (M11a) | built, checked on fake replays only | R1 |
| `ToolUse` and `ToolResult` events carry origin, input, content | exist (`dovetail-driver/src/event.rs`) | R1 |
| Task notes (`tasks/<id>.md`, v1.1) | **not built** | not needed by R1 or R2. R3 lists research from the task detail, which exists. Task notes can follow without rework |
| Live confidence in M11 | unchecked live | S7 covers the research-relevant part |

## 3. Schedule

Order: S7, M17, M18, M19, S8, M20, M21, M22. Research starts now, in parallel with the rest of M15 (f) and ahead of M16's user week; the only shared files are `dovetail-api` types and migration numbers (next state migration is `0013`), so merge in order. Gates that need the user: **G1** (before M20) placement of the tools the brief does not name (ADR 0016); **G2** (before M21) the side-panel key and the `Esc`/`i` scoping; **G3** (before M22) the registry and tangent-storage questions.

Every prompt below assumes the standing rules of `AGENTS.md`: read it and `CODING_STANDARDS.md` first, work in a worktree on branch `dovetail/<name>`, tests only for things that break and for the "done when" items named, `scripts/check.sh` before reporting, commit in steps ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` (or the model that did the work), open a PR, do not merge, list every departure from the spec as a question. No real Claude sessions against real projects; scratch repos under `/tmp` only.

### S7 — Capture fidelity spike
- **Model:** Sonnet — scripted, well-specified, small
- **Depends on:** none
- **Prompt (copy-paste):**
  > Read `AGENTS.md`, `docs/research.md` §5, `docs/spikes/driver.md`, `crates/dovetail-driver/src/{event,parse}.rs`. Write `spikes/research-capture/` (outside the workspace) and run it against a scratch git repo in `/tmp/dovetail-spikes/research/` with tiny prompts on Haiku or Sonnet. Launch a coordinator exactly as Dovetail does (`-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-mode auto`, `WebFetch` and `WebSearch` allowed) and have it spawn (1) a foreground subagent and (2) a background subagent, each told to fetch one stable public page with WebFetch, read one local file with Read, and report. Capture both raw streams. Answer in `docs/spikes/research-capture.md`: (a) are the subagents' `WebFetch` and `Read` `tool_use` inputs in the stream, tagged `parent_tool_use_id`? (b) is the `tool_result` body present, complete or truncated, and is a `WebFetch` result raw page text or a model summary? (c) where does the subagent's full final report appear for foreground and for background (`SubagentHandback` input, `Agent` result, `task_notification.output_file`) and is the output file complete? (d) do `WebSearch` results list URLs? (e) does a subagent that calls `mcp__dovetail__*` appear the same way? Save sanitised fixtures as `crates/dovetail-driver/tests/fixtures/research_round_{foreground,background}.ndjson`. Pass: every one of (a), (c) answerable yes for both; fallbacks recorded where not (for example read the output file when the stream lacks the report).
- **Validation:**
  - [ ] `docs/spikes/research-capture.md` answers (a) to (e) with evidence lines
  - [ ] Two sanitised fixtures committed, replayable by `FakeDriver`
  - [ ] A one-paragraph conclusion: which source each finding body comes from, and what "cached copy" really is
  - [ ] No process left running; cost recorded

### M17 — R1 Capture (core)
- **Model:** Opus — touches driver events, the core session state, storage and a new snapshot surface; the capture rules are the heart of the feature
- **Depends on:** S7, v1 M11 (subagent routing)
- **Prompt (copy-paste):**
  > Implement milestone R1 of `docs/research.md` §10, following §3 (migration `0013_research` with `research_rounds`, `findings`, `sources`, `finding_sources`; `reports` and the `tasks` columns can land here too), §4 (folder layout, default root and the recipe's `research_dir`) and §5 (capture pipeline), and `docs/spikes/research-capture.md` for which stream events carry what. Put plain types in `dovetail-api` (`ResearchSummary`, `Event::ResearchUpserted`, `Command::LoadResearch` and `Event::ResearchLoaded`), the pipeline in `dovetail-core/src/research/{mod,capture,files}.rs`, the recipe field in `recipe.rs`, and add `research_dir = "docs/research"` to `config/projects/dovetail.toml` with a bootstrap check. No MCP tools yet: a round is opened by a `Command::StartRound` (and `:round <task> <brief>`) so it is testable. Nothing is ever sent to the coordinator's session. Quick findings (no round) follow §5.7. Update `docs/implementation-plan.md` §5 (migration list) and §6 with an R1 entry. Tests: the two S7 fixtures through `FakeDriver`, dedup of a source fetched twice, restart keeps rows, opt-in dir writes only under `docs/research/`.
- **Validation:**
  - [x] Fake-driver replay of a subagent with WebFetch and Read yields `findings/01-<agent>.md` listing its sources (`dovetail-core/tests/research.rs`: `a_foreground_subagent_in_a_round_is_filed_with_its_sources`, `a_background_subagent_is_filed_on_its_notification_not_the_turn_result`; files `findings/01-fetch-page-and-read-notes.md` and `01-fetch-page-read-notes-report.md`)
  - [x] Same page fetched twice is one `sources` row, two `finding_sources` (`the_same_page_fetched_twice_is_one_source_row_linked_by_each_fetch`, counted in `state.db`)
  - [x] The `FakeDriver` command log shows no input sent to the coordinator because of capture (`fake.sent()` is exactly the user's one prompt in both S7 replays)
  - [x] Restart keeps rounds, findings and sources; files are rebuilt if missing (`a_restart_keeps_rounds_findings_and_sources_and_rebuilds_missing_files`: the task folder is deleted, the reopened app rewrites the same four files byte for byte)
  - [x] Opt-in `research_dir` writes only under that folder of a temp repo (`an_opt_in_research_dir_writes_only_under_that_folder_of_the_repo`)
  - [x] `scripts/check.sh` passes

### M18 — R2 Research suite tools
- **Model:** Sonnet — well specified by §6 and §7 once R1 exists
- **Depends on:** M17
- **Prompt (copy-paste):**
  > Implement milestone R2 of `docs/research.md` §10. Add `research_start_round`, `research_write_report`, `research_propose` and `research_get` to `crates/dovetail/src/mcp.rs` and `dovetail-api/src/tools.rs` (schemas exactly as §6, coordinator role only, with the `research_` names as written), handlers in `dovetail-core/src/tools/research.rs`. Report validation per §7 (citations must resolve and refuse otherwise; uncited claims and unlabelled figures flag; Dovetail rewrites the "Evidence: N sources, M independent" line). `research_propose` creates a new `InboxKind::Proposal` in Needs you; `y` accepts (idea: follow-up task in Unsorted or a new round, per ADR 0015; decision: through the existing `log_decision` path plus `decision_evidence`, migration `0014_decision_evidence`), `d` drops, the call returns `pending` at once and the answer arrives as a user message (as `start_slice` does). A `decision` without `evidence` is an error. Add short research lines to `config/roles/coordinator.md` (keep the file under ~20 lines; trim elsewhere if needed) and say `scripts/bootstrap.sh --force` is needed. Show evidence links in the task detail's decision log. Tests: the three error cases, citation resolution, the accept and drop flows.
- **Validation:**
  - [x] Fake-driver flow writes `brief.md` and `report.md`; every `[n]` resolves (`research_tools.rs`: `a_round_by_tool_and_a_report_write_brief_and_report_and_every_citation_resolves`)
  - [x] No open round, an unresolved citation, and a decision without evidence each return the specified error (`the_specified_errors_are_returned_and_nothing_is_written`)
  - [x] An accepted decision shows its finding and source links in the task detail (stored in `decision_evidence`, in `Decision::evidence`, `get_task`, and an `evidence: [F3] [S12]` line in the TUI task detail; `an_accepted_decision_is_logged_with_its_evidence_and_never_before`)
  - [x] `research_propose` never writes without `y` (`an_accepted_decision_...` checks nothing is logged before `y`; `a_dropped_proposal_writes_nothing_...` for `d`)
  - [x] Coordinator prompt still under ~20 lines; `bootstrap.sh` copies it (18 lines; the script copies `config/roles/*.md`; `--force` is needed to replace an installed copy)
  - [x] `scripts/check.sh` passes

### M19 — R3 Rough reading
- **Model:** Sonnet — views over existing buffers
- **Depends on:** M17 (M18 for reports)
- **Prompt (copy-paste):**
  > Implement milestone R3 of `docs/research.md` §10. In `dovetail-tui` (depends only on `dovetail-api`) list a task's report, rounds, findings and sources in the task detail, open one with `Enter` in a read-only buffer using the existing reading column and markdown renderer (`render/{markdown,reading_column}.rs`), colour `[Fn]`/`[Sn]` citation markers, `e` hands the file to nvim through the existing editor hand-off (core side). No new layout, no new keys beyond the task-detail scope. Also add `:research` to list research tasks. Add the actions to the keymap table (`keymap.rs`) in the `TaskDetail` scope only; do not touch `<leader>` bindings. Snapshot tests at 80, 120, 200 columns.
- **Validation:**
  - [x] Every stored artifact (report, brief, finding, source) opens from the task detail and renders in the reading column (`dovetail-tui/tests/research.rs`: `every_row_opens_its_document_in_the_reading_view` covers all four kinds; snapshots at 80, 120, 200; the mock round trip in `dovetail-mock/tests/mock.rs`). Report: shown as "no report yet" when `report_ms` is `None`; not yet tried against M18's real report
  - [ ] `[n]` markers are coloured (done: `the_reading_view_colours_citation_markers_and_nothing_else`, `render::citations` tests); `e` opens the real file in nvim and returns cleanly (the UI hands over `Command::OpenEditor { path }` with the document's absolute path, tested in `e_hands_the_documents_file_to_the_editor…`, through the existing, already tested lend-and-reclaim of the terminal; an interactive run in a real terminal is left to the user: `dovetail --mock` has no editor)
  - [x] No change to Layout B, no new global keys (the keymap gains one row, `<CR>` in the `DETAIL` scope; `:research` is a command; no `<leader>` binding touched)
  - [x] `scripts/check.sh` passes

### S8 — Research role-prompt reliability (extends S6)
- **Model:** Sonnet — runs trials; the user reviews
- **Depends on:** M18
- **Prompt (copy-paste):**
  > Read `docs/implementation-plan.md` S6 and `docs/research.md` §6. In a scratch repo run three trials each of two coordinators: tool descriptions only, and with the installed role prompt including the research lines. Task: "research whether X; I want a decision." Pass for the prompt variant: 3 of 3 runs call `research_start_round` before fanning out, call `research_write_report` with resolving citations, and use `research_propose` rather than deciding in prose. Record in `docs/spikes/research-role-prompt.md` with the raw call sequences; propose stronger wording or a skill if it fails. Tiny prompts, Haiku or Sonnet.
- **Validation:**
  - [ ] Six runs recorded with call sequences
  - [ ] Pass or fail stated per case, with the fallback if failed
  - [ ] The user reviews the result (not an agent task)

### M20 — R4 Tool suites and rename (ADR 0016)
- **Model:** Opus — repo-wide rename with migration, role config, persisted conversations
- **Depends on:** M18, **G1** (the user's placement of the tools the brief does not name)
- **Prompt (copy-paste):**
  > Implement ADR 0016 using the user's G1 decisions recorded in its "Open" notes (update the ADR first if they differ). Add suite membership to the tool table in `dovetail-api/src/tools.rs`, role config listing suites (`config.toml` and the role defaults), `allowed_tools` and the bridge tool list derived from suites, prefixed names, and for one release old names served as aliases by the same handler with a `tracing::warn!` and a one-line deprecation note in the tool result. Update `config/roles/*.md`, `scripts/live-checks.sh`, the hand-written `mcp__dovetail__` strings in tests and `dovetail-driver/src/claude.rs`, and the docs that describe current behaviour (not historical ADRs/spikes). Note `claim`/`release` are v1.1 Claims and not built; the Coordination suite is empty until then. Check that a resumed session whose transcript used the old name still works. Update `bootstrap.sh` notes and say `--force` is needed.
- **Validation:**
  - [ ] Roles get exactly their suites (assert the lists in a test)
  - [ ] Old names still answer, with a logged deprecation
  - [ ] No role prompt, script or test references a removed name
  - [ ] A resumed v1 conversation using an old name works
  - [ ] `scripts/check.sh` passes

### M21 — R5 Reading mode (ADR 0018)
- **Model:** Opus — a second layout preset against the single-layout assumption in TUI code; kitty graphics fallback
- **Depends on:** M19, M20, **G2** (side-panel key; `Esc`/`i` scoping)
- **Prompt (copy-paste):**
  > Implement milestone R5 of `docs/research.md` §10 and ADR 0018. Add a layout preset `Reading` (library, reading, ask) built from the existing `Region`/`Buffer` abstractions, toggled by `<leader>r`, left by `Esc` (Normal mode only). Library: projects, research tasks, report/rounds/findings, Decisions, Tangents (empty until R6), Docs · ADRs (read-only repo docs). Reading: 120-column column, solid background on, coloured `[n]` markers; `]s`/`[s` next/previous citation, `gd` open the cited finding or source, `gx` open the source in a browser through a core `Command::OpenExternal` (the TUI never spawns processes). Ask: `i` starts a read-only subagent grounded in the open report and its sources via the core; answers carry resolving `[n]` markers. Convert HTML artifacts to markdown. Images use kitty graphics where available, outline text otherwise. Change the keymap test that uses `gx` as an unbound example. Apply the G2 decisions to the side-panel key. Snapshot tests at 100, 160, 220 columns; no idle redraws.
- **Validation:**
  - [ ] `<leader>r` toggles; `Esc` leaves; Layout B unchanged otherwise
  - [ ] `]s`, `[s`, `gd`, `gx` work as specified; `gx` is core-launched
  - [ ] Ask answers cite with markers that resolve
  - [ ] HTML shown as converted markdown; `gx` opens the real page
  - [ ] Works in a terminal without kitty graphics
  - [ ] `scripts/check.sh` passes

### M22 — R6 Tangents and the side panel (ADR 0017)
- **Model:** Opus — global state outside the per-project model, one panel merging three session uses
- **Depends on:** M21, **G3** (registry and tangent storage; research-only project folder question)
- **Prompt (copy-paste):**
  > Implement milestone R6 of `docs/research.md` §10 and ADR 0017 with the user's G3 decisions. A global Tangents space (`tangents` table in `registry.db` and files under `~/.local/share/dovetail/tangents/`), a tangent session that runs there with the side-chat role prompt and read tools, captured sources as in R1, one side panel for side chats, grilling helper (auto-start still v1.1) and tangents, destinations per answer (`<leader>y`, attach to the current task as a quick finding, or Tangents by default), and the morning-review listing (file into a project, start a task, drop). Keep `<leader>sq`/`:ask` working as G2 decided. Research-only projects per G3. Update `docs/implementation-plan.md` and the glossary if names change.
- **Validation:**
  - [ ] A tangent asked with no project open is saved with its sources
  - [ ] The next morning review lists it; filing creates the finding or task in the chosen project
  - [ ] `<leader>y` still copies the last answer into the reply
  - [ ] State is restored after restart
  - [ ] `scripts/check.sh` passes

## 4. Earliest usable point

**After M17 (R1), about two sessions from now (S7, then M17).** What the rough form looks like day to day: you run research the way you do now. Every subagent the coordinator spawns while you have opened a round (`:round <task> <brief>`) has its full output saved as `findings/NN-<agent>.md` under `~/.local/share/dovetail/projects/<project>/research/<task-id>-<slug>/rounds/NN-<date>/`, with the pages and files it actually fetched listed in its header and cached beside it. The coordinator's context is untouched. You read the files in nvim (or `cat`). There is no `report.md` tool yet: the coordinator still writes its synthesis in chat, and you ask it to save that into the folder by hand.

**After M17 + M18 + M19 (R1 + R2 + R3)** it is the complete rough form: the coordinator calls `research_start_round`, writes a cited `report.md` that Dovetail validates, proposes ideas and decisions into your inbox with evidence, and you read reports, rounds and findings in the task detail with coloured `[n]` markers or in nvim. No reading mode, no tangents, tools unprefixed (the `research_` names are the exception), `<leader>b` untouched.

**Zero-code stopgap (not part of the build, use it today if you must).** In your installed `~/.config/dovetail/roles/coordinator.md`, add one line: "When you fan out research subagents, tell each one to write its full report to `~/research/<task>/NN-<name>.md` before replying, with a Sources section listing every URL it fetched or file it read, and to reply with a five-line summary; cite those files in your report." Limits: the sources are the subagent's own claim, not taken from tool calls; the folder is outside Dovetail's database; auto mode may ask for a write permission per subagent; `scripts/bootstrap.sh` will not overwrite your edit but `--force` will. This is exactly the lossy behaviour R1 replaces.

# Tool suites: prefixed names and per-role suite lists

Refines Decision 8b/25 (the table of tools `dovetail mcp` serves and which role gets each), and the unprefixed tool names the v1 role prompts, tests and scripts use. Builds on [ADR 0002](0002-create-task-in-v1.md), [0003](0003-decision-log-in-v1.md), [0004](0004-start-slice-tool.md) and [0005](0005-explicit-projects-any-folder.md), which added tools to Decision 25's list.

Why: the tool list has grown from three to twelve and the research feature adds more. A flat unprefixed list does not say which role may call what, and a name in a log says nothing about where it belongs.

**Decision:**

- Tools are grouped into **suites**. A suite is a documentation grouping and what a role gets: the role config lists suites, and `allowed_tools` (`session/launch.rs`) and the bridge's tool list are built from them.
- Tool names carry a suite prefix: `research_start_round`, not `start_round`.
- Brief suites: **Core** (`ask_user`, `report_status`), **Coordination** (`claim`, `release`, `list_claims`, ordering rules; v1.1), **Research** (`research_start_round`, `research_write_report`, `research_propose`).
- The coordinator gets Core, Coordination and Research; slices get Core and Coordination. A side chat gets Core.
- v1 tools ship unprefixed; v1.1 renames them.

**Placement (the user's G1 decisions, 2026-11-11).** The brief named three suites; the user placed the rest:

| Suite | Tools | Coordinator | Slices | Side chats |
| --- | --- | --- | --- | --- |
| Core | `core_ask_user`, `core_report_status`, `core_log_decision` | ✓ | ✓ | ✓ |
| Tasks | `tasks_create`, `tasks_list`, `tasks_get`, `tasks_update`, `tasks_archive` | ✓ | | |
| Project | `project_start_slice`, `project_archive_slice`, `project_add` | ✓ | | |
| Coordination | `claim`, `release` (v1.1 Claims, not built: empty for now), ordering rules | ✓ | ✓ | |
| Research | `research_start_round`, `research_write_report`, `research_propose`, `research_get` | ✓ | | |
| Maps | `maps_draw` | ✓ | ✓ | |
| Canvas (ADR 0025) | `canvas_add_card`, `canvas_link_cards` | ✓ | | |

- `log_decision` is Core, so slices and side chats keep it without getting Tasks. Side chats keep their read-only launch lists (ADR 0011, PR #52) and get Core only, so they lose `draw_map`.
- `start_slice`, `archive_slice` and `add_project` are lifecycle, not PR ownership: their own Project suite, not Coordination.
- `research_get` stays coordinator-only (Research). Reading mode's Ask column (M21) runs a read-only subagent of the coordinator, which shares its parent's bridge and so reaches it.
- "v1 tools ship unprefixed; v1.1 renames them" above is superseded: the rename ships with research (M20).

**Naming scheme.** `<suite>_<verb>[_<object>]`, the suite as prefix (`core`, `tasks`, `project`, `maps`, `research`), and the object dropped when it is the suite's own noun (`tasks_create`, not `tasks_create_task`; `project_add`, not `project_add_project`). Research tools already follow it.

| Old name (alias for one release) | New name | Suite |
| --- | --- | --- |
| `ask_user` | `core_ask_user` | Core |
| `report_status` | `core_report_status` | Core |
| `log_decision` | `core_log_decision` | Core |
| `create_task` | `tasks_create` | Tasks |
| `list_tasks` | `tasks_list` | Tasks |
| `get_task` | `tasks_get` | Tasks |
| `update_task` | `tasks_update` | Tasks |
| `archive_task` | `tasks_archive` | Tasks |
| `start_slice` | `project_start_slice` | Project |
| `archive_slice` | `project_archive_slice` | Project |
| `add_project` | `project_add` | Project |
| `draw_map` | `maps_draw` | Maps |
| `research_start_round`, `research_write_report`, `research_propose`, `research_get` | unchanged | Research |

**Role config.** `config.toml` may set `[suites]` with `coordinator`, `slice` and `side_chat` lists of suite names; each list left out takes the role's default from the table above (`Role::default_suites`). Side chats (and Ask) always get only `core` (decided 2026-11-11, PR #55): a `side_chat` list that says otherwise is ignored, with a warning in the status bar and the log. The core passes the list to the bridge in `DOVETAIL_SUITES`; `--allowedTools`, the bridge's `tools/list` and the core's check on each call all follow it.

**Rename cost.** The names appear in 64 tracked files: 38 under `crates/` (the tool table `dovetail-api/src/tools.rs` 45 mentions, `dovetail/src/mcp.rs` 22, `dovetail-core/src/tools.rs` 20, `dovetail-core/tests/sessions.rs` 16, `service.rs`, `slice.rs`, `inbox.rs`, the `merged_inputs.ndjson` fixture, the mock), 3 role prompts in `config/roles/` (`coordinator.md` 9, `slice.md` 3, `side-chat.md` 1), `scripts/live-checks.sh` (5), and 21 docs (design, digest, plan, ADRs 0002 to 0011, spikes). `session/launch.rs:162` builds `mcp__dovetail__<name>` from the tool table and `session/state.rs:73` strips the prefix, so both follow a rename automatically; the hand-written `mcp__dovetail__ask_user` strings in tests and `dovetail-driver/src/claude.rs` do not. Conversations already stored mention old names, and a resumed session whose transcript used `ask_user` will keep trying it.

**Migration (decided 2026-11-11):**

1. Serve both names for one release. The old name is an alias handled by the same handler, answers normally and logs a deprecation (`tracing::warn!`, plus a one-line note in the tool result so the model learns the new name).
2. Update the role prompts, `scripts/live-checks.sh` and the code's own references in the same PR as the rename. Historical ADRs and spikes are not edited; the glossary and this ADR record the mapping.
3. Install the new prompts with `scripts/bootstrap.sh --force` (as ADR-era prompt changes already require).
4. Drop the old names one release later.

An alternative worth the user's thought: serve each suite as its own MCP server (`mcp__dovetail-research__start_round`), so a role's suite list is its `--mcp-config` and no tool is renamed. It costs one more bridge process per suite per session (memory matters here, plan §2) and contradicts the brief's "names carry a prefix", so it is listed, not chosen.

Approved by the user in the grilling session of 2026-11-10 (claude.ai); the placement of the tools the brief does not name, the naming of the renamed tools and the migration above were decided on 2026-11-11 (G1). The migration is as recommended: an alias is listed in `tools/list` and `--allowedTools` with a short deprecated description and an open schema, so a resumed conversation can still call it.

# Tool suites: prefixed names and per-role suite lists

Refines Decision 8b/25 (the table of tools `dovetail mcp` serves and which role gets each), and the unprefixed tool names the v1 role prompts, tests and scripts use. Builds on [ADR 0002](0002-create-task-in-v1.md), [0003](0003-decision-log-in-v1.md), [0004](0004-start-slice-tool.md) and [0005](0005-explicit-projects-any-folder.md), which added tools to Decision 25's list.

Why: the tool list has grown from three to twelve and the research feature adds more. A flat unprefixed list does not say which role may call what, and a name in a log says nothing about where it belongs.

**Decision:**

- Tools are grouped into **suites**. A suite is a documentation grouping and what a role gets: the role config lists suites, and `allowed_tools` (`session/launch.rs`) and the bridge's tool list are built from them.
- Tool names carry a suite prefix: `research_start_round`, not `start_round`.
- Brief suites: **Core** (`ask_user`, `report_status`), **Coordination** (`claim`, `release`, `list_claims`, ordering rules; v1.1), **Research** (`research_start_round`, `research_write_report`, `research_propose`).
- The coordinator gets Core, Coordination and Research; slices get Core and Coordination. A side chat gets Core.
- v1 tools ship unprefixed; v1.1 renames them.

**What the brief does not place.** Dovetail serves twelve tools today (`crates/dovetail/src/mcp.rs`). Proposed placement, for the user to confirm:

| Tool | Today | Proposed suite | Note |
| --- | --- | --- | --- |
| `ask_user`, `report_status` | all roles | Core | as in the brief |
| `draw_map` | coordinator, slices | Core, or its own `Maps` suite | not in the brief |
| `create_task`, `list_tasks`, `get_task`, `update_task`, `archive_task` | coordinator | **`Tasks` suite (unplaced in the brief)** | coordinator-only |
| `log_decision` | coordinator, slices | `Tasks` | slices call it too, so slices would need `Tasks` (read-only subset?) |
| `start_slice`, `archive_slice`, `add_project` | coordinator | **`Project` suite (unplaced)**, or Coordination | `claim`/`release` are about PR ownership; these are about lifecycle |
| `claim`, `release`, `list_claims` | not built | Coordination | v1.1 Claims; the suite is nearly empty until then |

Consequence of the brief's literal wording ("coordinator: Core, Coordination, Research"): without extra suites the coordinator would lose its task and slice tools. The ADR therefore needs the user to approve the `Tasks` and `Project` suites (or merge them into Coordination).

**Rename cost.** The names appear in 64 tracked files: 38 under `crates/` (the tool table `dovetail-api/src/tools.rs` 45 mentions, `dovetail/src/mcp.rs` 22, `dovetail-core/src/tools.rs` 20, `dovetail-core/tests/sessions.rs` 16, `service.rs`, `slice.rs`, `inbox.rs`, the `merged_inputs.ndjson` fixture, the mock), 3 role prompts in `config/roles/` (`coordinator.md` 9, `slice.md` 3, `side-chat.md` 1), `scripts/live-checks.sh` (5), and 21 docs (design, digest, plan, ADRs 0002 to 0011, spikes). `session/launch.rs:162` builds `mcp__dovetail__<name>` from the tool table and `session/state.rs:73` strips the prefix, so both follow a rename automatically; the hand-written `mcp__dovetail__ask_user` strings in tests and `dovetail-driver/src/claude.rs` do not. Conversations already stored mention old names, and a resumed session whose transcript used `ask_user` will keep trying it.

**Migration (recommended):**

1. Serve both names for one release. The old name is an alias handled by the same handler, answers normally and logs a deprecation (`tracing::warn!`, plus a one-line note in the tool result so the model learns the new name).
2. Update the role prompts, `scripts/live-checks.sh` and the code's own references in the same PR as the rename. Historical ADRs and spikes are not edited; the glossary and this ADR record the mapping.
3. Install the new prompts with `scripts/bootstrap.sh --force` (as ADR-era prompt changes already require).
4. Drop the old names one release later.

An alternative worth the user's thought: serve each suite as its own MCP server (`mcp__dovetail-research__start_round`), so a role's suite list is its `--mcp-config` and no tool is renamed. It costs one more bridge process per suite per session (memory matters here, plan §2) and contradicts the brief's "names carry a prefix", so it is listed, not chosen.

Approved by the user in the grilling session of 2026-11-10 (claude.ai), apart from the placement of the tools the brief does not name, which stays a question.

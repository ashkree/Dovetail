# Tool groups by workspace object, grants per tool

Refines [ADR 0016](0016-tool-suites.md). This ADR replaces two things from it:
- suites, both as the name prefix and as the unit a role is granted;
- the v1 aliases.

It also adds tools for agents to read and act on the inbox, efforts, slices, canvases, maps and the project. Decided with the user on 2026-11-17, after the tool review in that session.

Why:
- **Missing read tools.** No tool exposed effort or criterion ids, so `coordination_claim_criterion` and the canvas tools' `effort` could only be guessed. Canvas card ids were lost after a restart or compaction.
- **Prefixes named after Dovetail's internals.** `core_` was a grab-bag, and `coordination_` mixed a slice's report with an effort's criterion.
- **Tools listed but always refused.** The coordinator was listed `coordination_report` and `design_declare_variants`, and every call was refused.

## Decision

**Names follow what the user sees.** A tool's prefix is its **tool group**: the workspace object it acts on. The groups are `inbox`, `efforts`, `tasks`, `slices`, `session`, `canvas`, `maps`, `research`, `viewer` and `project`. Names keep ADR 0016's shape, `<group>_<verb>[_<object>]`.

**Grants are per tool.** Each row of the tool table lists the roles that may call it. A role gets exactly those tools, in `tools/list`, in `--allowedTools`, and in the core's check on each call. A group grants nothing.

**Config changes:**
- `config.toml`'s `[suites]` section is gone. If it is present, it is ignored, with a warning in the log and the status bar.
- `DOVETAIL_SUITES` is gone. The bridge derives the tool list from `DOVETAIL_ROLE`.
- Side chats, tangents and Ask keep their fixed lists (ADR 0011, 0017, 0018).

**What agents may do.** Agents read freely, write only to what they own or create, and propose what the user decides. Nothing here lets an agent do the following, which stay the user's:
- answer a permission request or a question;
- approve or return a criterion;
- move or delete a canvas card (ADR 0025 and 0027 stand);
- keep variants;
- change accounts or repos.

**Inbox (the user's answers of 2026-11-17):**
- **Dismissing items.** An agent may dismiss its own open question with `inbox_dismiss`.
- **Coordinator cleanup.** The coordinator may read the whole inbox (`inbox_list`) and also dismiss:
  - any agent's question; the asking agent gets a message with the coordinator's reason;
  - finished-turn items;
  - error items.
- **What the coordinator may not dismiss.** Anything that waits on the user's decision:
  - permission requests;
  - slice-start requests;
  - research proposals;
  - claimed criteria;
  - worktree removals, orphans, the keep-coordinator choice, take-over locks, upgrades;
  - unfiled findings;
  - new tangents.

  Dismissing a permission request would silently deny a waiting agent.

**Efforts (the user's answers):** the coordinator creates efforts and adds criteria, but only through an **effort proposal** in the chat, never straight away.
- **Proposing.** `efforts_propose` posts the proposal into the coordinator chat as a card in a fixed template, and returns its id. A proposal is either a new effort or new criteria for an existing effort. The template:

  ```
  Effort proposal P3 (revises P2)
  Name:      guest-checkout                    (kind: feature)
  Goal:      Shoppers can buy without making an account.
  Criteria:  1. A guest can pay with a card and gets an emailed receipt.
             2. A guest order shows in the admin list like any other.
  First tasks (optional):
             - Guest session and cart (next)
             - Receipt email (next)
  Reply yes, no, or tell me what to change.
  ```

  For new criteria the card names the effort and lists only the criteria.
- **The user answers in the chat:**
  - **yes:** the coordinator calls `efforts_accept` with the proposal id, which creates exactly what the card showed: the effort, its criteria, and its first tasks in that effort.
  - **edits:** the coordinator calls `efforts_propose` again with `revises`. The new card replaces the old one, which is marked superseded.
  - **no:** nothing is created and the proposal is dropped.
- **What the tool guarantees.** Dovetail can't read "yes" in free text, so it guards what it can check:
  - `efforts_accept` takes only a proposal id, so nothing the user did not see can be created;
  - it accepts only the latest version of a proposal;
  - it accepts only if the user has sent a message after the card was posted, so the coordinator can't propose and accept in one turn.

  The rest (a "yes" is a yes) is the role prompt's.
- **The "where was I" card.** The coordinator writes it directly (`efforts_set_where_was_i`). It is the coordinator's summary for the user's return, not a decision.

**Old names.** The v1 aliases (`ask_user`, …) are dropped, not served for another release. The ADR 0016 names (`core_ask_user`, …) are not aliased either. A call by any old name gets an unknown-tool error that names the new tool, so a resumed conversation learns it in one turn. Old names are not listed in `tools/list`.

## The table

| Group | Tool | Was | Coordinator | Slice | Side chat / tangent |
| --- | --- | --- | --- | --- | --- |
| inbox | `inbox_ask` | `core_ask_user` | ✓ | ✓ | ✓ |
| | `inbox_list` | new | ✓ | | |
| | `inbox_dismiss` | new | ✓ (cleanup) | own questions | own questions |
| efforts | `efforts_list` | new | ✓ | ✓ | |
| | `efforts_get` | new | ✓ | ✓ | |
| | `efforts_propose` | new | ✓ | | |
| | `efforts_accept` | new | ✓ (after the user's yes) | | |
| | `efforts_set_where_was_i` | new | ✓ | | |
| | `efforts_claim_criterion` | `coordination_claim_criterion` | ✓ | ✓ | |
| tasks | `tasks_create`, `tasks_list`, `tasks_update`, `tasks_archive` | unchanged | ✓ | | |
| | `tasks_get` | unchanged | ✓ | ✓ | |
| | `tasks_log_decision` | `core_log_decision` | ✓ | ✓ | ✓ |
| slices | `slices_start` | `project_start_slice` | ✓ | | |
| | `slices_archive` | `project_archive_slice` | ✓ | | |
| | `slices_list` | new | ✓ | | |
| | `slices_report` | `coordination_report` | | ✓ | |
| session | `session_set_status` | `core_report_status` | ✓ | ✓ | ✓ |
| canvas | `canvas_add_block`, `canvas_add_card`, `canvas_link_cards` | unchanged | ✓ | | |
| | `canvas_get` | new | ✓ | | |
| maps | `maps_draw` | unchanged | ✓ | ✓ | |
| | `maps_list`, `maps_get` | new | ✓ | ✓ | |
| research | `research_start_round`, `research_write_report`, `research_propose`, `research_get` | unchanged | ✓ | | |
| viewer | `viewer_declare_variants` | `design_declare_variants` | | ✓ | |
| | `viewer_restart_preview` | new | | ✓ | |
| project | `project_add` | unchanged | ✓ | | |
| | `project_get` | new | ✓ | ✓ | |

Ask keeps no tools. In total the coordinator gets 32 tools, a slice 15, and a side chat or tangent 4.

Smaller changes that come with it:
- `tasks_create` takes `effort` and `criterion`, and `tasks_get` shows them.
- `tasks_list` filters by `review` and by `effort`.
- `inbox_ask` returns the new item's id, which `inbox_dismiss` takes.

## Considered options

- **Keep suites as the grant unit, grouped by workspace.** Rejected: a slice needs one or two tools from five groups (it may log a decision but not create tasks, and report but not start slices). Partial grants defeat the unit.
- **Let the coordinator create efforts with no check, steered only by its prompt.** Rejected by the user: an effort is the user's goal, so they confirm it looks right first.
- **An effort proposal as an inbox item with `y`/`d`.** Rejected by the user: an effort is shaped by talking it over, and edits go back and forth in the chat. A `y`/`d` item can't take "change criterion 2". The inbox stays for what waits on the user while they are away.
- **Keep ADR 0016's aliases for one more release.** Rejected: renaming twice would chain aliases. An error naming the new tool costs a resumed conversation one turn and keeps every tool list clean.
- **UTCP instead of MCP** (discussed the same day). Not adopted: Claude Code reaches outside tools only through MCP or Bash. A Bash `cli` call loses per-tool permissions, side chats' read-only Bash, and schema-checked arguments. A UTCP manual generated from the tool table remains possible later for clients other than Claude Code.

## Consequences

- **Prompts and checks change in the same PR.** Every role prompt, `scripts/live-checks.sh`, the mock, and the hand-written tool names in tests and `dovetail-driver` change together. Installed prompts need `scripts/bootstrap.sh --force`.
- **Effort proposals live in the chat, not the inbox.** They don't count in Needs you and aren't kept across a restart. After a restart the card is still in the chat as text, but `efforts_accept` refuses it, and the coordinator proposes again.
- **Stored conversations keep the old names.** The core does not rewrite them; the error message is the migration.

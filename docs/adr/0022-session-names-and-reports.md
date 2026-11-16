# Session names carry the project; slices report through a Dovetail tool

**Status: Accepted and built.** Approved by the user in the session of 2026-11-15; built 2026-11-16.

Settles open point 4 of `docs/design-v2.md`. Proposes one simplification of it: the effort is not part of the name.

## The fault

Inside Dovetail an agent is known by its id, and ids only need to be unique in one project, because each open project has its own database and its own socket. The coordinator's id is the fixed text `coordinator` (`COORDINATOR` in `crates/dovetail-core/src/service.rs`), and that is fine there.

The fault is one step further out. Dovetail passes each session's display name to Claude Code as `--name` (`Launch.name` in `crates/dovetail-core/src/session/launch.rs`), and Claude Code's own messaging finds sessions on the whole machine by that name. Two open projects mean two sessions named `coordinator`.

```
 TODAY                                         PROPOSED
 project A   coordinator ◄─┐                   project A   a.coordinator ◄── core_report ── a.api-guest
 project B   coordinator ◄─┼─ SendMessage      project B   b.coordinator
             slice ────────┘  "coordinator"                (names differ; reports do not use names)
```

## Decision

**1. The name given to Claude Code is `<project-id>.<agent-name>`.** Examples: `shopfront.coordinator`, `shopfront.api-guest`. Agent ids, the tree, branch names and the database do not change; only the `--name` flag does. The core strips the prefix where it reads a name back from the stream (`agent_by_name`).

**2. The effort is not in the name.** The design says names include the project and the effort. Uniqueness needs only the project. An effort in the name would go stale when a task moves to another effort, and the coordinator belongs to no effort. The effort shows where it is useful: as the tag on the slice's reports (point 3) and in the tree.

**3. Slices report with a new tool, `core_report`.**

```
 core_report { state: "progress" | "done" | "blocked", summary: "PR #41 open, tests pass", pr: 41 }
```

The core handles it like any tool call (`Core::tool`). It adds one line to the coordinator's chat, tagged with the effort of the slice's task, shown collapsed; and it sends the text to the coordinator's session as a note (`send_note`), which starts a turn if the coordinator is idle. No session name is involved, so a report cannot reach another project.

Example chat line: `[guest-checkout] ▸ slice api-guest: PR #41 open, tests pass`.

**4. Coordinator to slice stays Claude Code's own messaging, by the new unique name.** With unique names it can no longer misroute, and it lets the coordinator hold a conversation with a slice. The slice's first prompt states the coordinator's full name.

**5. A session started before this change keeps resuming.** Whether Claude Code accepts a different `--name` on resume is not known. Check it with one live session before building: if it does not, existing sessions keep their old name until they are archived.

## Alternatives rejected

- **Rename the coordinator's agent id per project.** The id is in every project's database, in the to-do list ids and in the tests. It was never the part that clashed.
- **Route every message through Dovetail and turn native messaging off.** Simpler to reason about, but it removes subagent hand-backs, which arrive the same way, and costs a second tool and a delivery queue for no fault that unique names leave open. Revisit if misroutes continue.
- **A random suffix in the name.** Unique, but unreadable when an agent has to type it.
- **Reports as plain `SendMessage` to the unique name.** Works, but Dovetail then sees a report only as a message between agents: it cannot tag it with an effort, collapse it, or use "done" to move the task (ADR 0024).

## Consequences

- `config/roles/slice.md` changes: "report with `core_report`", and ask the coordinator by its full name. `config/roles/coordinator.md` changes: slice names are prefixed.
- `core_report` joins the Core suite for slices (ADR 0016's table in `dovetail_api::tools`).
- The agent-to-agent message feed keeps working: it already matches names through `agent_by_name`.

## As built (2026-11-16)

- **The tool is `coordination_report`, in the Coordination suite, not `core_report`.** Every tool's name starts with its suite's (ADR 0016), and the Core suite also goes to side chats, which have nothing to report to. Slices and the coordinator get Coordination; a call from anything but a slice is refused.
- **Point 5 is settled:** a resumed session takes the new name (`docs/spikes/session-name-on-resume.md`), so existing sessions are renamed on their next start.
- The report line is an agent-message entry in the coordinator's conversation, tagged with the effort of the slice's task. "Slice ready" now also gives the slice's session name.

# Dovetail

A keyboard-driven terminal UI for running many Claude Code agents on one feature.

## Language

### Agents

**Coordinator**:
The session the user works with directly, running in the main checkout and never stopped.
_Avoid_: lead, main agent, orchestrator

**Slice**:
A long-lived session that owns one piece of work and has exactly one worktree of its own.
_Avoid_: worker, worker session

**Side chat**:
A session the user opens for a question or some research: it cannot change files and runs only read-only commands, works in the main checkout, with no worktree or task, and shows in the side panel (ADR 0011; side panel decided 2026-11-11).
_Avoid_: scratch session, helper

**Side panel**:
The pane in the side column, in place of the task tree, that shows side chats and tangents (later also the grilling helper) beside the coordinator's conversation; `<leader>?` toggles it.
_Avoid_: drawer, sidebar

**Ask**:
Reading mode's read-only session about one research task's report, in its research folder and apart from the coordinator: it answers from the report and the files it cites, citing them as `[F n]` and `[S n]`, and shows only in the Ask column (ADR 0018). One per research task; a new report closes it.
_Avoid_: ask subagent, report chat, side chat (which is project-scoped and opened by hand)

**Subagent**:
An agent a session spawns, at Claude's choice, for quick self-contained work. It shares its parent's worktree unless Claude isolates it.
_Avoid_: agent (which also covers sessions), worker

**Foreground subagent**:
A subagent that shares its parent's to-do list and can have to-do items.
_Avoid_: blocking subagent, inline subagent

**Background subagent**:
A subagent that cannot have to-do items and appears as one node with its description, progress, summary and cost.
_Avoid_: background task

### Work

**Task**:
A piece of the user's own work tracked in Dovetail, with or without agents attached. It is done when marked done in Dovetail, whatever its to-do items say.
_Avoid_: subtask, ticket, to-do

**Related task**:
A task linked to another both ways, with no effect on either one's order or status. Quick capture relates the new task to the task in focus unless told not to.
_Avoid_: subtask, child task, linked task

**Follow-up**:
A task raised because of another, which it names as the task it follows up.
_Avoid_: subtask, related task

**Decision**:
An entry in a task's decision log: what was decided, optionally why, and the alternatives that lost. Entries are never edited; a later one supersedes an earlier one.
_Avoid_: note, ADR (the repo's own decision records)

**To-do item**:
An entry on Claude's native list, shown under the session or foreground subagent that created it. Claude's tools call these tasks (`TaskCreate`, `TaskUpdate`), but they are never Dovetail tasks.
_Avoid_: task, subtask, checklist item

**`task_*` event**:
A stream event reporting on a subagent run, such as `task_started` or `task_notification`. It describes neither a task nor a to-do item.
_Avoid_: task, task update

**P-level**:
How important a task is, from P0 to P3.
_Avoid_: priority, severity

**Bucket**:
When the user will do a task: Now, Next or Later. A task with no bucket sits in Unsorted.
_Avoid_: priority, status, column

**Journal**:
The project's end-of-day record, one markdown file per closed day: the coordinator's handoff and Dovetail's facts (ADR 0012).
_Avoid_: log, diary, daily notes

**Handoff**:
What an outgoing coordinator writes for the fresh one that replaces it: open threads, what it was about to do, decisions in flight, what matters first.
_Avoid_: summary, briefing

### Conversation

**Prompt**:
What the user sends an agent, starting a turn of its conversation.
_Avoid_: message, input

**Message**:
Text one agent sends another through Claude Code's native messaging. Messages appear in Activity and carry no authority.
_Avoid_: prompt, chat

### Attention

**Needs you**:
The inbox tier holding everything waiting on the user, including each agent's latest finished turn. A task shows as needs you while one of its agents has an item here.
_Avoid_: alerts, notifications

**Activity**:
The inbox tier holding what asks nothing of the user, such as PR pushes, CI status and messages between agents. It is a separate feed.
_Avoid_: inbox item, log, notifications

**Inbox item**:
An entry in Needs you, of one of four kinds: permission request, question, finished turn or error.
_Avoid_: alert, notification

**Permission request**:
An inbox item asking the user to allow or deny a tool use.
_Avoid_: approval (the answer, not the request), prompt

**Question**:
An inbox item from an agent's `core_ask_user` call. It never blocks the agent: `core_ask_user` returns at once, and the answer arrives later as a message (ADR 0007).
_Avoid_: blocker, prompt

**Finished turn**:
An inbox item showing a preview of the last message from an agent whose turn ended. An agent has at most one, and the newest replaces the last. It does not mean the agent or its task is done.
_Avoid_: completed, done, result

**Error**:
An inbox item reporting a driver error, a crash or a memory kill.
_Avoid_: crash, failure

### Research

**Round**:
One fan-out of research subagents on a research task, with the user's criteria kept as its `brief.md`. A task has many rounds across days (ADR 0015).
_Avoid_: run, iteration, session

**Finding**:
One research subagent's full raw report, saved automatically; never fed back into the coordinator's context.
_Avoid_: result, output, summary

**Source**:
A page or file an agent actually fetched or read, known from its tool calls rather than from links in its prose, with a cached copy of what the tool returned.
_Avoid_: link, reference, citation (the marker, not the thing)

**Report**:
The coordinator's curated synthesis for one research task, `report.md`, rewritten each round and citing findings and sources by id.
_Avoid_: summary, write-up

**Tangent**:
A question that does not belong to the current project, answered in Tangents, a built-in research-only project that is always there, with sources, and kept there until the user attaches it to a task, or triages it (the morning's "new tangents" item, or `:tangents`): files it into a project, starts a task from it or drops it (ADR 0017).
_Avoid_: side chat (which is project-scoped), aside

**Suite**:
A named group of MCP tools; a role's config lists the suites it gets, and each tool name carries its suite as a prefix (ADR 0016). The suites are Core, Tasks, Project, Coordination (empty until v1.1 Claims), Research and Maps; by default the coordinator gets all of them, a slice Core, Coordination and Maps, a side chat Core. Tool names read `<suite>_<verb>`: `core_ask_user`, `tasks_create`, `project_start_slice`, `maps_draw`.
_Avoid_: toolset, plugin, namespace

**Altitude**:
Whether research is macro (business, economics, direction) or micro (implementation). It decides where the research lives; size decides how it runs.
_Avoid_: scope, level, depth

**Reading mode**:
The second layout preset, `<leader>r`: the library on the left, the reading column in the middle, Ask on the right, in place of Layout B, which comes back as it was (ADR 0018).
_Avoid_: reader view, focus mode (which only hides the side column)

**Library**:
Reading mode's left column: projects, the open project's research tasks with their report, rounds, findings and sources, the decision log and the repo's docs.
_Avoid_: tree (the task tree), shelf (the map shelf)

**Research-only project**:
A project of the research kind: it exists for macro research and has no repo, no worktrees and no slices; its sessions run in Dovetail's own folder for it (ADR 0015).
_Avoid_: notes project, non-code project

**Companion**:
A GUI window the user can open beside the terminal UI, for views a terminal does badly (maps first); it shows the same app and never holds the only way to do something (ADR 0019).
_Avoid_: GUI, viewer, canvas window, desktop app

### Maps

**Map shelf**:
The folding list of a project's maps at the bottom of the task tree.
_Avoid_: maps section, maps panel

**Structure map**:
A map of what exists and how it connects: nodes nested in groups, joined by labelled connections.
_Avoid_: architecture diagram, flowchart

**Swimlane map**:
A map of what happens in order: one guided flow drawn as steps across horizontal lanes, time running left to right.
_Avoid_: flow map, sequence diagram

**Guided flow**:
A named, ordered walk through a map's nodes with a note per step, moved through one step at a time.
_Avoid_: tour, walkthrough, path

**Lane**:
One participant's horizontal band on a swimlane map, optionally linked to that participant's node on a structure map.
_Avoid_: swimlane (the whole map), row, track

**Map node kind**:
One of the nine fixed kinds a map node can be (Part, File, Endpoint, Data, External, Actor, Outcome, Logic, Concept); it sets the node's shape and colour.
_Avoid_: node type, category

**Map version**:
One drawing of a map. A map keeps its id and type; each `maps_draw` call with that id stores a new version, and earlier versions are kept.
_Avoid_: revision, snapshot (the core's snapshot is something else)

### Build

**Worktree recipe**:
A project's instructions for preparing a new worktree: files to symlink in, plus an optional setup command.
_Avoid_: setup script, template, worktree config

**Profile**:
A named Claude Code account, kept in its own config folder; each project picks one in its worktree recipe and every session of the project runs under it.
_Avoid_: account (for the setting), login, identity

**Self-hosting point**:
The earliest milestone after which the rest of Dovetail's build runs inside Dovetail.
_Avoid_: bootstrap, dogfooding

**Upgrade**:
Replacing the installed Dovetail with a newer build by restarting it at a quiet moment; the sessions resume (ADR 0014).
_Avoid_: update, hot reload, live edit

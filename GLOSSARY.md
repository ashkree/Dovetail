# Dovetail

A keyboard-first app for running many Claude Code agents across a project's lines of work. Terms marked **(v2)** are new in `docs/design-v2.md`; **(changed in v2)** means the v1 meaning no longer holds; **(v1 TUI)** means the term lives only in the frozen terminal UI.

## Language

### Projects

**Home** (v2):
The first screen, listing every project; entering one opens its workspace.
_Avoid_: dashboard, launcher, project switcher

**Project** (changed in v2):
A named workspace holding any number of repos, including none, and one account. It is not tied to a folder and is never found from the current folder.
_Avoid_: workspace (for the thing itself), repo, folder

**Repo** (v2):
A git repository attached to a project.
_Avoid_: project, checkout, codebase

**Account** (v2, was Profile):
The Claude Code account a project's sessions run under, one per project.
_Avoid_: profile, login, identity

**Effort** (v2):
One line of work in a project, with a kind (feature, fix, cleanup, research or experiment), a defining question or goal, and acceptance criteria. Every task belongs to one.
_Avoid_: epic, initiative, stream, thread

**Acceptance criterion** (v2):
A condition an effort must meet; it is ticked only after an agent claims it with evidence and the user approves. An effort never closes without the user's review.
_Avoid_: requirement, goal (the effort's defining aim), definition of done

**"Where was I" card** (v2):
An effort's summary of what it is, what was last done, what waits on the user and the next step, shown first when the user returns to the effort.
_Avoid_: status, summary, handoff (which is the whole set of cards a fresh coordinator gets)

**Running effort** (v2):
An effort that counts against the machine's soft limit on parallel lines of work; the count shows and never blocks.
_Avoid_: active effort, open effort

### Agents

**Coordinator** (changed in v2):
The one session per project the user works with directly, running in the project's state folder with every repo attached, and never stopped. In v1 it ran in the repo's main checkout.
_Avoid_: lead, main agent, orchestrator

**Project chat** (v2):
The coordinator's single conversation for a whole project, where every message carries an effort tag.
_Avoid_: effort chat, thread (a filtered view of one effort's messages), coordinator chat (accepted as a synonym)

**Effort tag** (v2):
The effort a chat message belongs to: the focused effort for the user's messages, the slice's task's effort for slice reports.
_Avoid_: label, channel, topic

**Slice** (changed in v2):
A long-lived session that owns one piece of work in exactly one repo, with exactly one worktree of its own. An effort may have slices in several repos.
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
An agent a session spawns, at Claude's choice, for quick self-contained work. It shares its parent's worktree unless Claude isolates it. Subagent work with no task shows as **untracked** under its effort (v2).
_Avoid_: agent (which also covers sessions), worker

**Foreground subagent**:
A subagent that shares its parent's to-do list and can have to-do items.
_Avoid_: blocking subagent, inline subagent

**Background subagent**:
A subagent that cannot have to-do items and appears as one node with its description, progress, summary and cost.
_Avoid_: background task

### Work

**Task** (changed in v2):
A piece of the user's own work in one effort, with or without agents attached. Dovetail sets its status: To do, In progress when an agent starts on it, Review when the agent finishes or opens a PR, Done when the user approves the merge. Its to-do items never decide it. A task that serves no acceptance criterion is flagged.
_Avoid_: subtask, ticket, to-do

**Unsorted** (v2):
The effort every project starts with, holding the tasks nobody has put in an effort yet. It cannot be renamed or closed.
_Avoid_: inbox, backlog, default effort

**Reasoning effort**:
How hard a session's model thinks, from low to max; a setting of the session, unrelated to efforts.
_Avoid_: effort

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

**Bucket** (v1 TUI):
When the user will do a task: Now, Next or Later. A task with no bucket sits in the Unsorted list, which is not the Unsorted effort. Whether buckets stay in v2 is open; v2's **Unsorted** is an effort holding migrated tasks.
_Avoid_: priority, status, column

**Journal**:
The project's end-of-day record, one markdown file per closed day: the coordinator's handoff and Dovetail's facts (ADR 0012).
_Avoid_: log, diary, daily notes

**Handoff** (changed in v2):
What a fresh coordinator starts from. In v2 it is every effort's "where was I" card; in v1 the outgoing coordinator wrote open threads, what it was about to do, decisions in flight and what matters first.
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

**Finding** (changed in v2):
A result of research. In v1, one research subagent's full raw report, saved automatically and never fed back into the coordinator's context; in v2 findings appear only as finding cards. How the two map is open.
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
A named group of MCP tools; a role's config lists the suites it gets, and each tool name carries its suite as a prefix (ADR 0016). The suites are Core, Tasks, Project, Coordination (`coordination_report`, ADR 0022; `coordination_claim_criterion`), Research, Maps and Canvas (`canvas_add_card`, `canvas_link_cards`; coordinator only, ADR 0025); by default the coordinator gets all of them, a slice Core, Coordination and Maps, a side chat Core. Tool names read `<suite>_<verb>`: `core_ask_user`, `tasks_create`, `project_start_slice`, `maps_draw`.
_Avoid_: toolset, plugin, namespace

**Altitude**:
Whether research is macro (business, economics, direction) or micro (implementation). It decides where the research lives; size decides how it runs.
_Avoid_: scope, level, depth

**Reading view** (v2):
The main-view display for a long report, kept because findings are cards but reports are prose.
_Avoid_: reading mode (the v1 TUI preset), reader

**Reading mode** (v1 TUI):
The second layout preset, `<leader>r`: the library on the left, the reading column in the middle, Ask on the right, in place of Layout B, which comes back as it was (ADR 0018).
_Avoid_: reader view, focus mode (which only hides the side column)

**Library**:
Reading mode's left column: projects, the open project's research tasks with their report, rounds, findings and sources, the decision log and the repo's docs.
_Avoid_: tree (the task tree), shelf (the map shelf)

**Research-only project** (changed in v2):
In v2, a project with no repos; whether the separate project kind stays is open. In v1, a project of the research kind: it exists for macro research and has no repo, no worktrees and no slices; its sessions run in Dovetail's own folder for it (ADR 0015).
_Avoid_: notes project, non-code project

**Companion** (replaced in v2):
A GUI window the user could open beside the terminal UI, which never held the only way to do something (ADR 0019). v2 replaces it with the GUI itself.
_Avoid_: GUI, viewer, canvas window, desktop app

### Canvas and viewer

**Canvas** (v2):
A board belonging to an effort, started from a framework: question tree, options compare or free board. An effort can have any number.
_Avoid_: board (the v1 kanban view), whiteboard, map

**Card** (v2):
One item on a canvas, of a fixed kind: finding, source, idea, decision, question or map. The agent adds and links cards but never moves the user's.
_Avoid_: node (a map's item), note, sticky

**Incoming strip** (v2):
The edge of a canvas where research output arrives as cards before the user places them.
_Avoid_: inbox (the attention list), queue

**Viewer** (v2):
The main-view display of a running web preview, started from the project's preview command per slice; it follows the focused slice.
_Avoid_: preview pane, browser, Companion

**Variant** (v2):
One alternative the agent writes into a slice's code and declares in a list, at one of three levels: global, component or content. Variants mix and match; keeping a combination makes it permanent and deletes the rest.
_Avoid_: option (an options-compare column), version, theme

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

**Profile** (replaced in v2 by Account):
v1's name for a named Claude Code account kept in its own config folder, picked per project in its worktree recipe.

**Self-hosting point**:
The earliest milestone after which the rest of Dovetail's build runs inside Dovetail.
_Avoid_: bootstrap, dogfooding

**Upgrade**:
Replacing the installed Dovetail with a newer build by restarting it at a quiet moment; the sessions resume (ADR 0014).
_Avoid_: update, hot reload, live edit

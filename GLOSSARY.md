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
An inbox item from an agent's `ask_user` call, which may block the agent until answered.
_Avoid_: blocker, prompt

**Finished turn**:
An inbox item showing a preview of the last message from an agent whose turn ended. An agent has at most one, and the newest replaces the last. It does not mean the agent or its task is done.
_Avoid_: completed, done, result

**Error**:
An inbox item reporting a driver error, a crash or a memory kill.
_Avoid_: crash, failure

### Maps

**Map shelf**:
The folding list of a project's maps at the bottom of the task tree.
_Avoid_: maps section, maps panel

### Build

**Worktree recipe**:
A project's instructions for preparing a new worktree: files to symlink in, plus an optional setup command.
_Avoid_: setup script, template, worktree config

**Self-hosting point**:
The earliest milestone after which the rest of Dovetail's build runs inside Dovetail.
_Avoid_: bootstrap, dogfooding

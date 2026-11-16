//! Subagents as tree nodes (M11a, decision 11, digest §3).
//!
//! A subagent runs inside its parent's process, so everything about it arrives on the parent's
//! stream:
//!
//! - the parent's `Agent` (or `Task`) tool call creates the node, keyed by its `tool_use_id`;
//! - the subagent's own messages carry that id as `parent_tool_use_id` and are routed to the node,
//!   never into the parent's conversation;
//! - `system/task_*` events (`task_type: local_agent`) drive status, progress (tool uses,
//!   tokens, duration), the final summary and the output file. `task_updated` carries only the
//!   task id, so the task id is mapped back to the tool use id when `task_started` names both;
//! - a foreground subagent also ends at the tool result of the call that launched it.
//!
//! The stream gives no per-subagent cost: the parent's `result` cost already includes its
//! subagents, so a subagent's `cost_usd` stays 0 and totals count nothing twice.
//!
//! When a subagent that Claude isolated in its own worktree (`agent-<id>`) finishes, the worktree
//! is removed if unchanged (digest §6: `WorktreeRemove` does not fire for subagents); a changed
//! one is left for the parent and noted in Activity.

use std::path::PathBuf;

use dovetail_api::{ActivityKind, Agent, AgentId, AgentKind, AgentStatus, Event, SubagentProgress};
use dovetail_driver::{ContentBlock, DriverEvent, Origin, TaskEvent, TaskPhase, ToolResult};
use serde_json::Value;

use crate::db::{AgentRow, now_ms};
use crate::service::{Core, Input, new_agent};
use crate::worktree::{self, SUBAGENT_PREFIX, SubagentCleanup};

/// Finished subagents kept in the tree per parent, newest first; older ones are closed.
const KEEP_FINISHED: usize = 1;

/// Longest status line taken from a summary.
const STATUS_MAX: usize = 120;

/// Tool names that launch a subagent.
pub(crate) fn launches_subagent(tool: &str) -> bool {
    matches!(tool, "Agent" | "Task")
}

/// Marks a subagent that can no longer report as stopped.
pub(crate) fn mark_stopped(agent: &mut Agent, why: &str) {
    agent.status = AgentStatus::Error;
    agent.status_line = why.to_owned();
}

/// Whether a subagent has finished, for good or not.
fn finished(agent: &Agent) -> bool {
    matches!(agent.status, AgentStatus::Done | AgentStatus::Error)
}

/// The first non-empty line of `text`, cut to [`STATUS_MAX`] characters.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > STATUS_MAX {
        let cut: String = line.chars().take(STATUS_MAX - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

/// A `task_*` status that means the subagent is over: `Some(true)` for success.
fn terminal(status: Option<&str>) -> Option<bool> {
    match status? {
        "completed" => Some(true),
        "failed" | "killed" | "stopped" | "cancelled" | "error" => Some(false),
        _ => None,
    }
}

/// The final report: the hand-back text when the notification points to it, else the summary.
fn summary_of(event: &TaskEvent) -> Option<String> {
    event
        .raw
        .get("handback_report")
        .and_then(|r| r.get("text"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| event.summary.clone())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

impl Core {
    /// The parent's `Agent` call `tool_use_id` with `input`: a new node under `parent`.
    pub(crate) fn subagent_launched(&mut self, parent: &AgentId, tool_use_id: &str, input: &Value) {
        let id = super::subagent_id(parent, tool_use_id);
        if self.agents.contains_key(&id) {
            return;
        }
        let field = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let description = field("description");
        let name = description
            .clone()
            .or_else(|| field("subagent_type"))
            .unwrap_or_else(|| "subagent".into());
        let background = input
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(parent_row) = self.agents.get(parent) else {
            return;
        };
        let mut agent = new_agent(
            id.clone(),
            name,
            AgentKind::Subagent { background },
            parent_row.agent.task,
        );
        agent.parent = Some(parent.clone());
        agent.permission_mode = parent_row.agent.permission_mode;
        agent.status = AgentStatus::Working;
        agent.status_line = field("subagent_type").unwrap_or_default();
        agent.progress = Some(SubagentProgress {
            description: description.unwrap_or_default(),
            ..SubagentProgress::default()
        });
        let row = AgentRow {
            agent,
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: now_ms(),
            tool_use_id: Some(tool_use_id.to_owned()),
            claude_home: parent_row.claude_home.clone(),
            takeover: None,
            model_choice: None,
            repo: None,
        };
        self.agents.insert(id.clone(), row);
        self.save_agent(&id);
    }

    /// An event of a subagent of `parent` (rule 1: routed by `parent_tool_use_id`).
    pub(crate) fn on_subagent_event(
        &mut self,
        parent: &AgentId,
        origin: &Origin,
        event: &DriverEvent,
    ) {
        let Some(tool_use_id) = origin.parent_tool_use_id.as_deref() else {
            return;
        };
        if let (Some(key), Some(live)) = (&origin.agent_id, self.live.get_mut(parent)) {
            live.subagent_keys
                .entry(tool_use_id.to_owned())
                .or_insert_with(|| key.clone());
        }
        let id = super::subagent_id(parent, tool_use_id);
        if !self.agents.contains_key(&id) {
            self.subagent_launched(parent, tool_use_id, &Value::Null);
        }
        let last_tool = match event {
            DriverEvent::Assistant(message) => message.blocks.iter().rev().find_map(|b| match b {
                ContentBlock::ToolUse { name, .. } => Some(name.clone()),
                _ => None,
            }),
            _ => None,
        };
        // Deltas are frequent and change nothing shown: only messages touch the node.
        if !matches!(
            event,
            DriverEvent::Assistant(_) | DriverEvent::ToolResult(_)
        ) {
            return;
        }
        self.update_agent(&id, |row| {
            row.agent.last_activity_ms = now_ms();
            if let (Some(tool), Some(p)) = (last_tool, row.agent.progress.as_mut()) {
                p.last_tool = Some(tool);
            }
        });
    }

    /// `system/task_*` on `parent`'s stream.
    pub(crate) fn on_task_event(&mut self, parent: &AgentId, event: TaskEvent) {
        // Background shell tasks (`local_bash`) are not subagents.
        if event
            .task_type
            .as_deref()
            .is_some_and(|t| t != "local_agent")
        {
            return;
        }
        let Some(live) = self.live.get_mut(parent) else {
            return;
        };
        // Only `task_started` names the task type; later events of a background shell task carry
        // none, so they are told apart by their task id.
        let tool_use_id = match (&event.phase, &event.task_type, &event.tool_use_id) {
            (TaskPhase::Started, Some(_), Some(t)) => {
                live.subagent_tasks.insert(event.task_id.clone(), t.clone());
                live.subagent_keys
                    .entry(t.clone())
                    .or_insert_with(|| event.task_id.clone());
                t.clone()
            }
            _ => match live.subagent_tasks.get(&event.task_id) {
                Some(t) => t.clone(),
                None => return, // not a subagent we saw start
            },
        };
        let id = super::subagent_id(parent, &tool_use_id);
        if !self.agents.contains_key(&id) {
            let input = serde_json::json!({
                "description": event.description,
                "subagent_type": event.subagent_type,
            });
            self.subagent_launched(parent, &tool_use_id, &input);
        }
        let background = event.raw.get("is_backgrounded").and_then(Value::as_bool);
        let summary = summary_of(&event);
        let output_file = event
            .output_file
            .as_deref()
            .filter(|f| !f.is_empty())
            .map(PathBuf::from);
        let before = self
            .agents
            .get(&id)
            .and_then(|r| r.agent.progress.as_ref())
            .map_or(0, |p| p.total_tokens);
        self.update_agent(&id, |row| {
            let a = &mut row.agent;
            a.last_activity_ms = now_ms();
            if let Some(background) = background {
                a.kind = AgentKind::Subagent { background };
            }
            let p = a.progress.get_or_insert_with(SubagentProgress::default);
            if p.description.is_empty()
                && let Some(d) = &event.description
                && event.phase == TaskPhase::Started
            {
                p.description = d.clone();
            }
            if let Some(n) = event.tool_uses {
                p.tool_uses = u32::try_from(n).unwrap_or(u32::MAX);
            }
            if let Some(n) = event.total_tokens {
                p.total_tokens = n;
            }
            if let Some(n) = event.duration_ms {
                p.duration_ms = n;
            }
            if let Some(tool) = &event.last_tool_name {
                p.last_tool = Some(tool.clone());
            }
            if output_file.is_some() {
                p.output_file = output_file;
            }
            // The notification may follow a `task_updated` that already ended the node.
            if event.phase == TaskPhase::Notification
                && let Some(s) = &summary
            {
                p.summary = Some(s.clone());
                if finished(a) {
                    a.status_line = first_line(s);
                }
            }
            // `task_progress` describes what it does right now.
            if event.phase == TaskPhase::Progress
                && !finished(a)
                && let Some(d) = &event.description
            {
                a.status_line = first_line(d);
            }
        });
        // The session's total gains what this subagent used since its last event; the count
        // stays with the session after the node is pruned.
        let added = event.total_tokens.map_or(0, |n| n.saturating_sub(before));
        if added > 0 {
            self.update_agent(parent, |row| {
                row.agent.subagent_tokens = row.agent.subagent_tokens.saturating_add(added);
            });
        }
        let ended = match event.phase {
            TaskPhase::Updated | TaskPhase::Notification => terminal(event.status.as_deref()),
            _ => None,
        };
        if let Some(ok) = ended {
            self.subagent_finished(parent, &id, ok);
        }
    }

    /// A tool result on the parent's own stream: the end of a foreground subagent.
    pub(crate) fn subagent_tool_result(&mut self, parent: &AgentId, result: &ToolResult) {
        let id = super::subagent_id(parent, &result.tool_use_id);
        let Some(row) = self.agents.get(&id) else {
            return;
        };
        // A background call's result only says it was launched.
        if row.agent.kind != (AgentKind::Subagent { background: false }) || finished(&row.agent) {
            return;
        }
        let text = result.content_text();
        self.update_agent(&id, |row| {
            if let Some(p) = row.agent.progress.as_mut()
                && p.summary.is_none()
                && !text.trim().is_empty()
            {
                p.summary = Some(text.trim().to_owned());
            }
        });
        self.subagent_finished(parent, &id, !result.is_error);
    }

    /// The subagent `id` is over: status, the summary's first line, worktree cleanup, pruning.
    fn subagent_finished(&mut self, parent: &AgentId, id: &AgentId, ok: bool) {
        let Some(row) = self.agents.get(id) else {
            return;
        };
        if finished(&row.agent) {
            return;
        }
        let tool_use_id = row.tool_use_id.clone();
        self.update_agent(id, |row| {
            let a = &mut row.agent;
            a.status = if ok {
                AgentStatus::Done
            } else {
                AgentStatus::Error
            };
            let summary = a.progress.as_ref().and_then(|p| p.summary.as_deref());
            a.status_line = match summary.map(first_line).filter(|s| !s.is_empty()) {
                Some(line) => line,
                None if ok => String::new(),
                None => "failed".into(),
            };
        });
        let key = tool_use_id.and_then(|t| {
            self.live
                .get(parent)
                .and_then(|l| l.subagent_keys.get(&t).cloned())
        });
        if let Some(key) = key {
            self.clean_subagent_worktree(id, format!("{SUBAGENT_PREFIX}{key}"));
        }
        self.prune_finished_subagents(parent);
    }

    /// Checks the subagent's worktree `name` on a blocking task and removes it if unchanged.
    fn clean_subagent_worktree(&mut self, id: &AgentId, name: String) {
        // A research project has no repo to have made a worktree in: no git calls.
        if self.project.is_research() || worktree::validate_name(&name).is_err() {
            return;
        }
        let paths = self.paths.clone();
        let key = self.project.key().to_owned();
        let Some(repo) = self.repo_of(id).map(|r| r.path) else {
            return;
        };
        let inputs = self.inputs.clone();
        let agent = id.clone();
        tokio::task::spawn_blocking(move || {
            let result = worktree::remove_if_unchanged(&paths, &key, &name, &repo);
            let _ = inputs.send(Input::SubagentWorktree {
                agent,
                name,
                result,
            });
        });
    }

    pub(crate) fn on_subagent_worktree(
        &mut self,
        agent: &AgentId,
        name: &str,
        result: Result<SubagentCleanup, crate::worktree::WorktreeError>,
    ) {
        let who = self.name_of(agent);
        match result {
            Ok(SubagentCleanup::Absent) => {}
            Ok(SubagentCleanup::Removed) => {
                tracing::info!(subagent = %agent.0, worktree = name, "removed unchanged worktree");
            }
            Ok(SubagentCleanup::Changed {
                uncommitted,
                commits,
            }) => {
                let mut parts = Vec::new();
                if uncommitted > 0 {
                    parts.push(format!("{uncommitted} uncommitted"));
                }
                if commits > 0 {
                    parts.push(format!("{commits} commits"));
                }
                self.activity(
                    ActivityKind::Lifecycle,
                    Some(agent),
                    format!(
                        "subagent {who} left work in worktree {name} ({}); kept for its parent",
                        parts.join(", ")
                    ),
                );
            }
            Err(e) => {
                tracing::warn!(subagent = %agent.0, worktree = name, "subagent cleanup: {e}");
                self.activity(
                    ActivityKind::Lifecycle,
                    Some(agent),
                    format!("subagent {who}: could not check worktree {name}: {e}"),
                );
            }
        }
    }

    /// Keeps the newest [`KEEP_FINISHED`] finished subagents of `parent`; closes the rest.
    fn prune_finished_subagents(&mut self, parent: &AgentId) {
        let mut done: Vec<(i64, AgentId)> = self
            .agents
            .values()
            .filter(|r| r.agent.parent.as_ref() == Some(parent) && finished(&r.agent))
            .map(|r| (r.created_ms, r.agent.id.clone()))
            .collect();
        done.sort();
        let excess = done.len().saturating_sub(KEEP_FINISHED);
        for (_, id) in done.into_iter().take(excess) {
            self.close_subagent(&id);
        }
    }

    fn close_subagent(&mut self, id: &AgentId) {
        if self.agents.remove(id).is_none() {
            return;
        }
        self.tails.remove(id);
        self.persist("a closed subagent", self.db.close_agent(id.clone()));
        self.reassign_todos_of(id);
        self.emit(Event::AgentRemoved(id.clone()));
    }

    /// The parent is gone for good: so are its subagent nodes.
    pub(crate) fn close_subagents_of(&mut self, parent: &AgentId) {
        let ids: Vec<AgentId> = self
            .agents
            .values()
            .filter(|r| r.agent.parent.as_ref() == Some(parent))
            .map(|r| r.agent.id.clone())
            .collect();
        for id in ids {
            self.close_subagent(&id);
        }
    }

    /// The parent's process exited: subagents still running stopped with it.
    pub(crate) fn subagents_stopped(&mut self, parent: &AgentId) {
        let ids: Vec<AgentId> = self
            .agents
            .values()
            .filter(|r| r.agent.parent.as_ref() == Some(parent) && !finished(&r.agent))
            .map(|r| r.agent.id.clone())
            .collect();
        for id in ids {
            self.update_agent(&id, |row| {
                mark_stopped(&mut row.agent, "stopped: its session ended");
            });
        }
    }
}

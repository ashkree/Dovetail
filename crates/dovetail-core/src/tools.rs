//! The MCP tools' handlers (M9, ADR 0002–0005). The bridge parses and validates each call, but
//! the core checks the role again: a slice must not reach a coordinator tool even through a
//! hand-written socket client.

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::tools::Tool;
use dovetail_api::{
    DecisionAuthor, EntryKind, Event, InboxKind, NewDecision, NewTask, PermissionMode,
};
use serde_json::json;
use tokio::sync::oneshot;

use crate::error::SliceError;
use crate::inbox::Waiter;
use crate::service::Core;
use crate::slice::{SliceRequest, SliceStart};
use crate::socket::ToolCall;

/// Most characters of a status line (B17).
const STATUS_MAX: usize = 120;

fn ok(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: false,
    }
}

fn error(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: true,
    }
}

/// `start_slice`'s answer: `started`, `queued` or `denied` first, then what to do next.
pub(crate) fn slice_reply(result: Result<SliceStart, SliceError>) -> SocketResponseBody {
    match result {
        Ok(SliceStart::Started(name)) => ok(format!(
            "started: slice {name} is setting up. Wait for \"slice {name} ready\" before you \
             message it."
        )),
        Ok(SliceStart::Queued(name)) => ok(format!(
            "queued: the maximum number of slices is running; {name} starts when one is \
             archived, and you will get \"slice {name} ready\" then."
        )),
        Err(reason) => error(format!("denied: {reason}")),
    }
}

impl Core {
    /// Answers one tool call, at once: `ask_user` too, whose answer arrives later as a message
    /// (ADR 0007).
    pub(crate) async fn tool(
        &mut self,
        call: ToolCall,
        reply: oneshot::Sender<SocketResponseBody>,
    ) {
        let ToolCall { agent, call, .. } = call;
        let Some(role) = self.role_of(&agent) else {
            let _ = reply.send(SocketResponseBody::Error {
                message: format!("unknown agent `{}`", agent.0),
            });
            return;
        };
        if !Tool::allowed(call.name(), role) {
            let _ = reply.send(error(format!(
                "`{}` is for the coordinator only",
                call.name()
            )));
            return;
        }
        if let Err(problem) = call.validate(role) {
            let _ = reply.send(error(problem.to_string()));
            return;
        }
        let answer = match call {
            Tool::AskUser(ask) => {
                let kind = InboxKind::Question {
                    text: ask.question.clone(),
                };
                // Shown in the conversation too, where it is easier to read than in Needs you.
                self.add_entry(
                    &agent,
                    EntryKind::Notice,
                    format!("Question for you (also in Needs you): {}", ask.question),
                );
                self.raise(
                    &agent,
                    kind,
                    Waiter::Question {
                        question: ask.question,
                    },
                );
                ok(
                    "Question sent; the answer will arrive as a new message. Carry on with \
                    anything that does not depend on it, or end your turn if nothing is left.",
                )
            }
            Tool::ReportStatus(report) => {
                let line: String = report
                    .status
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(STATUS_MAX)
                    .collect();
                self.update_agent(&agent, |row| row.agent.status_line = line);
                ok("Status set.")
            }
            Tool::CreateTask(task) => {
                let new = NewTask {
                    title: task.title,
                    p_level: task.p_level,
                    bucket: task.bucket,
                    notes: task.notes,
                    follow_up_of: task.follow_up_of,
                    pr: None,
                    related: task.related,
                };
                let bucket = match new.bucket {
                    Some(b) => format!("{b:?}"),
                    None => "Unsorted".into(),
                };
                // Coordinator-created, marked until the user has seen it (ADR 0002).
                match self.create_task(new, true).await {
                    Ok(id) => ok(format!("Created task #{} in {bucket}.", id.0)),
                    Err(e) => error(format!("could not create the task: {e}")),
                }
            }
            Tool::LogDecision(decision) => {
                let own_task = self.agents.get(&agent).and_then(|r| r.agent.task);
                match decision.task.or(own_task) {
                    None => error("you have no task of your own: name the `task`"),
                    Some(task) => {
                        let new = NewDecision {
                            task,
                            text: decision.decision,
                            why: decision.why,
                            alternatives: decision.alternatives,
                            supersedes: decision.supersedes,
                        };
                        match self
                            .db
                            .log_decision(new, DecisionAuthor::Agent(agent.clone()))
                            .await
                        {
                            Ok(logged) => {
                                let text = format!(
                                    "Logged decision #{} on task #{}.",
                                    logged.id.0, task.0
                                );
                                self.emit(Event::DecisionAdded(logged));
                                ok(text)
                            }
                            Err(e) => error(format!("could not log the decision: {e}")),
                        }
                    }
                }
            }
            Tool::StartSlice(start) => {
                let request = SliceRequest {
                    task: Some(start.task),
                    name: start.name.trim().to_owned(),
                    brief: start.brief,
                    by_coordinator: true,
                };
                if let Err(reason) = self.check_slice(&request).await {
                    let _ = reply.send(error(format!("denied: {reason}")));
                    return;
                }
                let mode = self
                    .agents
                    .get(&agent)
                    .map_or(PermissionMode::Manual, |r| r.agent.permission_mode);
                if mode != PermissionMode::Auto {
                    // Approval follows the coordinator's permission mode (ADR 0004).
                    let name = request.name.clone();
                    let input = json!({
                        "task": request.task.map(|t| t.0),
                        "name": request.name,
                        "brief": request.brief,
                    });
                    self.raise(
                        &agent,
                        InboxKind::Permission {
                            tool: dovetail_api::START_SLICE_PERMISSION.into(),
                            input,
                            reason: Some("the coordinator wants to start a slice".into()),
                            suggestions: Vec::new(),
                        },
                        Waiter::StartSlice { request },
                    );
                    // The call returns at once; the answer comes as a message (decided
                    // 2026-11-06, ADR 0004).
                    let _ = reply.send(ok(format!(
                        "pending: the user is asked to approve slice {name}. The answer arrives \
                         as a message: \"start_slice {name} approved\" (then wait for \"slice \
                         {name} ready\") or \"start_slice {name} denied\"."
                    )));
                    return;
                }
                let result = self.request_slice(request).await;
                slice_reply(result)
            }
            Tool::AddProject(_) => {
                let before = self.project.registered().map(|p| p.id.clone());
                match self.register_project().await {
                    Ok(project) if before.is_some() => ok(format!(
                        "This folder is already the Dovetail project `{}`.",
                        project.id
                    )),
                    Ok(project) => {
                        let text = format!(
                            "Added `{}` as the Dovetail project `{}`. Its tasks, inbox and this \
                             conversation are now kept.",
                            project.repo_path.display(),
                            project.id
                        );
                        self.add_entry(&agent, EntryKind::Notice, text.clone());
                        ok(text)
                    }
                    Err(e) => error(format!("could not add the project: {e}")),
                }
            }
        };
        let _ = reply.send(answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::tools::Role;

    #[test]
    fn start_slice_answers_lead_with_the_outcome() {
        let text = |body| match body {
            SocketResponseBody::ToolResult { text, .. } => text,
            other => panic!("{other:?}"),
        };
        assert!(text(slice_reply(Ok(SliceStart::Started("a".into())))).starts_with("started"));
        assert!(text(slice_reply(Ok(SliceStart::Queued("a".into())))).starts_with("queued"));
        assert!(text(slice_reply(Err(SliceError::Reserved))).starts_with("denied"));
    }

    #[test]
    fn roles_are_checked_with_the_shared_rule() {
        assert!(!Tool::allowed("start_slice", Role::Slice));
        assert!(!Tool::allowed("add_project", Role::Slice));
        assert!(Tool::allowed("ask_user", Role::Slice));
    }
}

//! The MCP tools' handlers (M9, ADR 0002–0005). The bridge parses and validates each call, but
//! the core checks the role again: a slice must not reach a coordinator tool even through a
//! hand-written socket client.

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::tools::archive_result::{ARCHIVED, WAITING_FOR_USER};
use dovetail_api::tools::{Tool, UpdateTask};
use dovetail_api::{
    Agent, AgentKind, DecisionAuthor, EntryKind, Event, InboxKind, NewDecision, NewTask,
    PermissionMode, TaskChange, TaskId, TaskStatus,
};
use serde_json::json;
use tokio::sync::oneshot;

use crate::db::ModelChoice;

/// The model a coordinator-started slice gets when it names none (task #42).
const DEFAULT_SLICE_MODEL: &str = "sonnet";
use crate::error::SliceError;
use crate::inbox::Waiter;
use crate::service::Core;
use crate::slice::{ArchiveCall, SliceRequest, SliceStart};
use crate::socket::ToolCall;

mod map;
mod read;
mod research;

/// Most characters of a status line (B17).
const STATUS_MAX: usize = 120;

pub(crate) fn ok(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: false,
    }
}

pub(crate) fn error(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: true,
    }
}

/// `project_start_slice`'s answer: `started`, `queued` or `denied` first, then what to do next.
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

/// Answers a settled `project_archive_slice` / `tasks_archive` call: `archived` or `waiting_for_user`
/// first, or an error.
pub(crate) fn answer_archive(call: ArchiveCall) {
    let _ = call.reply.send(archive_reply(
        call.task,
        &call.archived,
        &call.waiting,
        &call.failed,
    ));
}

fn archive_reply(
    task: Option<TaskId>,
    archived: &[String],
    waiting: &[String],
    failed: &[String],
) -> SocketResponseBody {
    let names = |list: &[String]| list.join(", ");
    let waiting_text = |list: &[String]| {
        format!(
            "{WAITING_FOR_USER}: slice {} has uncommitted or unpushed work. The user is asked \
             whether it may go; the answer arrives as a message, \"slice <name> archived\" or \
             \"slice <name> kept\".",
            names(list)
        )
    };
    match task {
        None => {
            if !failed.is_empty() {
                error(format!("could not archive slice {}", names(failed)))
            } else if !waiting.is_empty() {
                ok(waiting_text(waiting))
            } else {
                ok(format!(
                    "{ARCHIVED}: slice {} is gone: session stopped, worktree and branch \
                     removed, its place freed.",
                    names(archived)
                ))
            }
        }
        Some(id) => {
            let mut text = if !waiting.is_empty() {
                format!("{} Task #{} is archived.", waiting_text(waiting), id.0)
            } else {
                format!("{ARCHIVED}: task #{}.", id.0)
            };
            if !archived.is_empty() {
                text.push_str(&format!(" Archived slice {}.", names(archived)));
            }
            if failed.is_empty() {
                ok(text)
            } else {
                error(format!("{text} Could not archive slice {}.", names(failed)))
            }
        }
    }
}

impl Core {
    /// Answers one tool call, at once: `core_ask_user` too, whose answer arrives later as a message
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
        if !Tool::allowed(call.name(), self.config.suites.for_role(role)) {
            let _ = reply.send(error(format!(
                "`{}` is not in this session's tool suites",
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
            Tool::Report(report) => self.slice_report(&agent, report).await,
            Tool::ClaimCriterion(claim) => self.claim_criterion_tool(&agent, claim).await,
            Tool::CanvasAddBlock(add) => self.canvas_add_block_tool(&agent, add).await,
            Tool::CanvasAddCard(add) => self.canvas_add_card_tool(&agent, add).await,
            Tool::CanvasLinkCards(link) => self.canvas_link_cards_tool(link).await,
            Tool::DeclareVariants(declared) => self.declare_variants_tool(&agent, declared).await,
            Tool::CreateTask(task) => {
                let new = NewTask {
                    title: task.title,
                    p_level: task.p_level,
                    bucket: task.bucket,
                    notes: task.notes,
                    follow_up_of: task.follow_up_of,
                    pr: None,
                    related: task.related,
                    effort: Default::default(),
                    criterion: None,
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
                    repo: start.repo,
                    // Always explicit: a slice never runs on Claude Code's own default model.
                    model_choice: Some(ModelChoice {
                        model: start.model.map_or_else(
                            || DEFAULT_SLICE_MODEL.to_owned(),
                            |m| m.trim().to_owned(),
                        ),
                        effort: start.effort,
                    }),
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
                        "repo": request.repo,
                        "model": request.model_choice.as_ref().map(|c| &c.model),
                        "effort": request.model_choice.as_ref().and_then(|c| c.effort),
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
                         as a message: \"project_start_slice {name} approved\" (then wait for \"slice \
                         {name} ready\") or \"project_start_slice {name} denied\"."
                    )));
                    return;
                }
                let result = self.request_slice(request).await;
                slice_reply(result)
            }
            Tool::AddProject(_) => {
                let before = self.project.registered().map(|p| p.id.clone());
                match self.register_project().await {
                    Ok((project, _)) if before.is_some() => ok(format!(
                        "This folder is already the Dovetail project `{}`.",
                        project.id
                    )),
                    Ok((project, starter)) => {
                        // The profile stays commented out: no picker from a tool call (task
                        // #31, decision #74).
                        let recipe = match starter {
                            Some(path) => format!(
                                " A starter recipe is at `{}`: set `profile` and the worktree \
                                 setup there.",
                                path.display()
                            ),
                            None => format!(
                                " Its recipe `{}` was already there and is unchanged.",
                                self.paths.recipe(&project.id).display()
                            ),
                        };
                        let text = format!(
                            "Added `{}` as the Dovetail project `{}`. Its tasks, inbox and this \
                             conversation are now kept.{recipe}",
                            project
                                .first_repo()
                                .map_or_else(String::new, |r| r.path.display().to_string()),
                            project.id
                        );
                        self.add_entry(&agent, EntryKind::Notice, text.clone());
                        ok(text)
                    }
                    Err(e) => error(format!("could not add the project: {e}")),
                }
            }
            Tool::ArchiveSlice(archive) => {
                let name = archive.name.trim().to_owned();
                if let Some(at) = self.queue.iter().position(|q| q.name == name) {
                    // Not started yet: dropping the queued start is all there is to archive.
                    if let Some(queued) = self.queue.remove(at) {
                        self.persist("the slice queue", self.db.dequeue_slice(queued.id));
                    }
                    ok(format!(
                        "{ARCHIVED}: slice {name} was still queued; its start is dropped."
                    ))
                } else {
                    match self.slice_named(&name) {
                        Err(reason) => error(reason.to_string()),
                        Ok(slice) => {
                            self.archive_for_tool(vec![slice], None, reply).await;
                            return;
                        }
                    }
                }
            }
            Tool::ArchiveTask(archive) => {
                let id = archive.id;
                let own = self.agents.get(&agent).and_then(|r| r.agent.task);
                if own == Some(id) {
                    error(format!(
                        "task #{} is this conversation's own task; it cannot be archived from here",
                        id.0
                    ))
                } else {
                    match self.task(id).await {
                        None => error(format!("there is no task #{}", id.0)),
                        Some(t) if t.status == TaskStatus::Archived => {
                            error(format!("task #{} is already archived", id.0))
                        }
                        Some(_) => {
                            // Closing a research round on the user's behalf is the user's call
                            // (they are asked when they archive the task themselves).
                            if let Some(round) = self.round_blocking_archive(id).await {
                                let _ = reply.send(error(format!(
                                    "task #{} has {round}: close it with research_write_report \
                                     (or ask the user to drop it), or ask the user to archive \
                                     the task, which asks them first",
                                    id.0
                                )));
                                return;
                            }
                            let slices = self.task_slices(id);
                            self.archive_for_tool(slices, Some(id), reply).await;
                            return;
                        }
                    }
                }
            }
            Tool::ListTasks(filter) => match self.db.load_tasks().await {
                Ok(tasks) => ok(read::task_list(&tasks, &filter, |id| self.live_slices(id))),
                Err(e) => error(format!("could not read the tasks: {e}")),
            },
            Tool::GetTask(get) => match self.task(get.id).await {
                None => error(format!("there is no task #{}", get.id.0)),
                Some(task) => match self.db.task_decisions(task.id).await {
                    Ok(decisions) => ok(read::task_detail(
                        &task,
                        &self.live_slices(task.id),
                        &decisions,
                    )),
                    Err(e) => error(format!("could not read the decisions: {e}")),
                },
            },
            Tool::UpdateTask(update) => self.update_task_tool(update).await,
            Tool::DrawMap(draw) => self.draw_map(&agent, draw.0).await,
            Tool::ResearchStartRound(args) => self.research_start_round(args).await,
            Tool::ResearchWriteReport(args) => self.research_write_report(args).await,
            Tool::ResearchPropose(args) => self.research_propose(&agent, args).await,
            Tool::ResearchGet(args) => self.research_get(args).await,
        };
        let _ = reply.send(answer);
    }

    /// `tasks_update`: applies only the fields given, through the same patch path as the TUI's
    /// edits, and answers with the task's new `tasks_list` line. Validation already refused
    /// `archived`. A task moved into Now is marked unseen, so the user notices it.
    async fn update_task_tool(&mut self, update: UpdateTask) -> SocketResponseBody {
        let id = update.id;
        let was_in_now = match self.task(id).await {
            None => return error(format!("there is no task #{}", id.0)),
            Some(t) if t.status == TaskStatus::Archived => {
                return error(format!("task #{} is archived", id.0));
            }
            Some(t) => t.bucket == Some(dovetail_api::Bucket::Now),
        };
        // Validation refused a status: Dovetail sets it (ADR 0024).
        let mut patch = Vec::new();
        if let Some(bucket) = update.new_bucket() {
            patch.push(TaskChange::Bucket(bucket));
            // The user did not put it in Now, so it carries the marker new tasks have.
            if bucket == Some(dovetail_api::Bucket::Now) && !was_in_now {
                patch.push(TaskChange::Unseen);
            }
        }
        if let Some(p_level) = update.p_level {
            patch.push(TaskChange::PLevel(p_level));
        }
        if let Some(title) = update.title {
            patch.push(TaskChange::Title(title.trim().to_owned()));
        }
        if let Some(notes) = update.notes {
            patch.push(TaskChange::Notes(notes));
        }
        let described = patch.clone();
        let changed = match self.db.update_task(id, patch).await {
            Ok(changed) => changed,
            Err(e) => return error(format!("could not update task #{}: {e}", id.0)),
        };
        let line = changed
            .iter()
            .find(|t| t.id == id)
            .map(|task| read::task_line(task, &self.live_slices(id)));
        self.task_activity(id, &changed, &described);
        self.emit_tasks(changed);
        match line {
            Some(line) => ok(line),
            None => ok(format!("Updated task #{}.", id.0)),
        }
    }

    /// The live slices on task `id`, by name.
    fn live_slices(&self, id: TaskId) -> Vec<&Agent> {
        let mut slices: Vec<&Agent> = self
            .agents
            .values()
            .map(|r| &r.agent)
            .filter(|a| a.kind == AgentKind::Slice && a.task == Some(id))
            .collect();
        slices.sort_by(|a, b| a.name.cmp(&b.name));
        slices
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
    fn suites_are_checked_with_the_shared_rule_for_new_and_old_names() {
        let slice = Role::Slice.default_suites();
        let coordinator = Role::Coordinator.default_suites();
        for name in [
            "project_start_slice",
            "start_slice",
            "project_add",
            "add_project",
            "project_archive_slice",
            "tasks_archive",
            "tasks_list",
            "tasks_get",
            "tasks_update",
            "research_get",
        ] {
            assert!(!Tool::allowed(name, slice), "{name}");
            assert!(Tool::allowed(name, coordinator), "{name}");
        }
        for name in ["core_ask_user", "ask_user", "maps_draw", "draw_map"] {
            assert!(Tool::allowed(name, slice), "{name}");
        }
        let side_chat = Role::SideChat.default_suites();
        assert!(Tool::allowed("core_log_decision", side_chat));
        assert!(!Tool::allowed("maps_draw", side_chat));
    }
}

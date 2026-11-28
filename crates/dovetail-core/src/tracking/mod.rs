//! Tracking domain: projects, registration, Home loop, efforts, and task/decision formatting & SQL.

pub(crate) mod db;
pub(crate) mod efforts;
pub mod home;
pub(crate) mod project_admin;
pub mod projects;
pub(crate) mod read;
pub(crate) mod registration;

pub use registration::RegisterError;

use std::collections::{BTreeMap, HashMap};

use dovetail_api::activity::ActivityKind;
use dovetail_api::agents::{
    agent::{Agent, AgentId, AgentKind},
    conversation::EntryKind,
};
use dovetail_api::tracking::{
    decision::{DecisionAuthor, DecisionCommand, DecisionEvent, NewDecision},
    effort::{Effort, EffortCommand, EffortId},
    project::{ProjectCommand, ProjectEvent, ProjectId},
    task::{Bucket, NewTask, Task, TaskChange, TaskCommand, TaskEvent, TaskId, TaskStatus},
};
use dovetail_api::{Command, Event};
use dovetail_tools::{
    SocketResponseBody,
    report::LogDecision,
    task::{TaskTool, UpdateTask},
};
use tokio::sync::oneshot;

use crate::engine::Core;
use crate::mcp::tools::{error, ok};
use crate::storage::DbError;

/// State owned by the tracking domain: the open project, efforts, tags, and review tracking.
pub(crate) struct TrackingState {
    pub(crate) project: registration::ProjectState,
    /// Every effort of the open project with its criteria (`efforts`).
    pub(crate) efforts: BTreeMap<EffortId, Effort>,
    /// The effort tag each agent's new entries get: that of the user's last message to it.
    pub(crate) effort_tags: HashMap<AgentId, EffortId>,
    /// Tasks the user sent back from review, with how far their pull requests had come then:
    /// they stay in progress until that changes or a slice reports again (decided 2026-11-16).
    /// Memory only: a restart forgets it.
    pub(crate) sent_back: HashMap<TaskId, crate::code::worktree_info::PrStage>,
    /// The project's repos changed while the coordinator ran (ADR 0021 point 4): what to tell it
    /// once it has restarted with its new `--add-dir` flags.
    pub(crate) reattach: Option<String>,
}

impl TrackingState {
    pub(crate) fn new(project: registration::ProjectState) -> Self {
        Self {
            project,
            efforts: BTreeMap::new(),
            effort_tags: HashMap::new(),
            sent_back: HashMap::new(),
            reattach: None,
        }
    }
}

impl Core {
    pub(crate) async fn handle_project_command(&mut self, command: ProjectCommand) {
        match command {
            project @ (ProjectCommand::Create { .. }
            | ProjectCommand::Rename { .. }
            | ProjectCommand::SetAccount { .. }
            | ProjectCommand::AddRepo { .. }
            | ProjectCommand::RemoveRepo { .. }) => {
                if let Some(op) = project_admin::ProjectOp::parse(Command::Project(project)) {
                    self.project_op(op).await;
                }
            }
            ProjectCommand::Register { path } => {
                let wanted = path.canonicalize().unwrap_or(path);
                if !wanted.as_os_str().is_empty()
                    && wanted != self.tracking.project.coordinator_folder(&self.paths)
                {
                    self.fail("only the open folder can be added as a project");
                    return;
                }
                match self.register_project().await {
                    Ok((project, Some(path))) => {
                        self.emit(Event::Project(ProjectEvent::RecipeStarted {
                            project: ProjectId(project.id),
                            name: project.name,
                            path,
                            profiles: self.config.profiles.keys().cloned().collect(),
                        }))
                    }
                    Ok((_, None)) => {}
                    Err(message) => self.fail(message),
                }
            }
            ProjectCommand::SetRecipeProfile { project, profile } => {
                self.set_recipe_profile(project, profile);
            }
            ProjectCommand::Open { id } => self.open_other_project(id).await,
            ProjectCommand::NewResearch { name } => self.new_research_project(name).await,
        }
    }

    pub(crate) async fn handle_task_command(&mut self, command: TaskCommand) {
        match command {
            TaskCommand::Create(new) => {
                if let Err(e) = self.create_task(new, false).await {
                    self.fail(e.to_string());
                }
            }
            TaskCommand::Update { id, patch } => self.user_update_task(id, patch).await,
            TaskCommand::Archive { id } => {
                let slices = self.task_slices(id);
                self.archive_task(id, &slices).await;
            }
            TaskCommand::Reorder { id, delta } => match self.db.reorder_task(id, delta).await {
                Ok(tasks) => self.emit_tasks(tasks),
                Err(e) => self.fail(e.to_string()),
            },
            TaskCommand::Move {
                id,
                effort,
                criterion,
            } => self.move_task(id, effort, criterion).await,
        }
    }

    pub(crate) async fn handle_effort_command(&mut self, command: EffortCommand) {
        match command {
            EffortCommand::Create(new) => self.create_effort(new).await,
            EffortCommand::Rename { id, name } => self.rename_effort(id, name).await,
            EffortCommand::Update { id, patch } => self.update_effort(id, patch).await,
            EffortCommand::SetWhereWasI { id, where_was_i } => {
                self.set_where_was_i(id, where_was_i).await;
            }
            EffortCommand::AddCriterion { effort, text } => self.add_criterion(effort, text).await,
            EffortCommand::ClaimCriterion { id, evidence } => {
                self.claim_criterion(id, evidence).await;
            }
            EffortCommand::ApproveCriterion { id } => self.approve_criterion(id).await,
            EffortCommand::ReturnCriterion { id, reason } => {
                self.return_criterion(id, reason).await;
            }
        }
    }

    pub(crate) async fn handle_decision_command(&mut self, command: DecisionCommand) {
        match command {
            DecisionCommand::Log(new) => {
                match self.db.log_decision(new, DecisionAuthor::User).await {
                    Ok(decision) => self.emit(Event::Decision(DecisionEvent::Added(decision))),
                    Err(e) => self.fail(e.to_string()),
                }
            }
        }
    }

    pub(crate) async fn handle_task_tool(
        &mut self,
        agent: &AgentId,
        tool: TaskTool,
        reply: oneshot::Sender<SocketResponseBody>,
    ) {
        let answer = match tool {
            TaskTool::Create(task) => {
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
            TaskTool::Archive(archive) => {
                let id = archive.id;
                let own = self.agents.rows.get(agent).and_then(|r| r.agent.task);
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
            TaskTool::List(filter) => match self.db.load_tasks().await {
                Ok(tasks) => ok(read::task_list(&tasks, &filter, |id| self.live_slices(id))),
                Err(e) => error(format!("could not read the tasks: {e}")),
            },
            TaskTool::Get(get) => match self.task(get.id).await {
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
            TaskTool::Update(update) => self.update_task_tool(update).await,
        };
        let _ = reply.send(answer);
    }

    pub(crate) async fn handle_log_decision_tool(
        &mut self,
        agent: &AgentId,
        decision: LogDecision,
    ) -> SocketResponseBody {
        let own_task = self.agents.rows.get(agent).and_then(|r| r.agent.task);
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
                        let text = format!("Logged decision #{} on task #{}.", logged.id.0, task.0);
                        self.emit(Event::Decision(DecisionEvent::Added(logged)));
                        ok(text)
                    }
                    Err(e) => error(format!("could not log the decision: {e}")),
                }
            }
        }
    }

    pub(crate) async fn handle_add_project_tool(&mut self, agent: &AgentId) -> SocketResponseBody {
        let before = self.tracking.project.registered().map(|p| p.id.clone());
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
                self.add_entry(agent, EntryKind::Notice, text.clone());
                ok(text)
            }
            Err(e) => error(format!("could not add the project: {e}")),
        }
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
            Some(t) => t.bucket == Some(Bucket::Now),
        };
        // Validation refused a status: Dovetail sets it (ADR 0024).
        let mut patch = Vec::new();
        if let Some(bucket) = update.new_bucket() {
            patch.push(TaskChange::Bucket(bucket));
            // The user did not put it in Now, so it carries the marker new tasks have.
            if bucket == Some(Bucket::Now) && !was_in_now {
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
            .rows
            .values()
            .map(|r| &r.agent)
            .filter(|a| a.kind == AgentKind::Slice && a.task == Some(id))
            .collect();
        slices.sort_by(|a, b| a.name.cmp(&b.name));
        slices
    }

    /// Creates a task and tells the UI.
    pub(crate) async fn create_task(
        &mut self,
        new: NewTask,
        coordinator_created: bool,
    ) -> Result<TaskId, DbError> {
        let tasks = self.db.create_task(new, coordinator_created).await?;
        let id = tasks
            .first()
            .map(|t| t.id)
            .ok_or_else(|| DbError::NotFound("no task was created".into()))?;
        let title = tasks.first().map(|t| t.title.clone()).unwrap_or_default();
        let who = if coordinator_created {
            "the coordinator"
        } else {
            "you"
        };
        self.activity(
            ActivityKind::Task,
            None,
            format!("task #{} {title}: created by {who}", id.0),
        );
        self.emit_tasks(tasks);
        Ok(id)
    }

    pub(crate) fn emit_tasks(&mut self, tasks: Vec<Task>) {
        for task in tasks {
            self.emit(Event::Task(TaskEvent::Upserted(task)));
        }
    }

    pub(crate) async fn task(&self, id: TaskId) -> Option<Task> {
        self.db
            .load_tasks()
            .await
            .ok()?
            .into_iter()
            .find(|t| t.id == id)
    }

    pub(crate) async fn update_task(&mut self, id: TaskId, patch: Vec<TaskChange>) {
        let described = patch.clone();
        match self.db.update_task(id, patch).await {
            Ok(tasks) => {
                self.task_activity(id, &tasks, &described);
                self.emit_tasks(tasks);
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// Adds the Activity line for a task patch that was applied.
    pub(crate) fn task_activity(&mut self, id: TaskId, changed: &[Task], patch: &[TaskChange]) {
        let title = changed.iter().find(|t| t.id == id).map_or("", |t| &t.title);
        if let Some(text) = dovetail_api::tracking::task::describe_task_patch(id, title, patch) {
            self.activity(ActivityKind::Task, None, text);
        }
    }

    /// The open slices working on task `id`.
    pub(crate) fn task_slices(&self, id: TaskId) -> Vec<AgentId> {
        self.agents
            .rows
            .values()
            .filter(|r| r.agent.kind == AgentKind::Slice && r.agent.task == Some(id))
            .map(|r| r.agent.id.clone())
            .collect()
    }

    /// Archives task `id`: its `slices` first (normally [`Core::task_slices`]), with the
    /// usual unsaved-work rule, then the task, then its queued slice starts.
    pub(crate) async fn archive_task(&mut self, id: TaskId, slices: &[AgentId]) {
        for slice in slices {
            self.close_slice(slice, false);
        }
        // The user was asked about an open or queued research round (the UI's confirmation); the
        // round closes with its findings kept, and queued ones are dropped.
        self.release_research_for_archive(id).await;
        self.update_task(id, vec![TaskChange::Status(TaskStatus::Archived)])
            .await;
        let dropped: Vec<i64> = self
            .agents
            .queue
            .iter()
            .filter(|q| q.task == Some(id))
            .map(|q| q.id)
            .collect();
        self.agents.queue.retain(|q| q.task != Some(id));
        // Digest §4: the lists Dovetail created go with their task.
        self.delete_task_todos(id);
        for queued in dropped {
            self.persist("the slice queue", self.db.dequeue_slice(queued));
        }
    }
}

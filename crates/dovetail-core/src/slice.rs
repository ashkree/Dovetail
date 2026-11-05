//! Slices (M8, ADR 0004, ADR 0005): starting them in hook-created worktrees, the ready or
//! setup-failed signal, the `max_slices` queue, and archiving with cleanup.
//!
//! **Start.** A slice in a git project launches in the main checkout with `--worktree <name>`;
//! the `WorktreeCreate` hook creates the worktree and runs the recipe's setup, then records its
//! outcome (`worktree::HookOutcome`). The core holds the brief until that outcome says the
//! worktree is ready, so a failed setup never gets a first prompt. In a folder without git the
//! slice runs in the shared folder, and its first prompt says so.
//!
//! **Archive.** The session is stopped first. Then the worktree is inspected: clean and pushed
//! work is removed at once (our symlinks, `git worktree remove`, the `dovetail/*` branch);
//! otherwise the user is asked through a Permission item before anything is lost.

use std::time::Duration;

use dovetail_api::{
    ActivityKind, AgentId, AgentKind, AgentStatus, Event, InboxKind, TaskChange, TaskId,
    TaskStatus, WorktreeRef,
};
use serde_json::json;
use tokio::sync::oneshot;

use crate::db::{AgentRow, now_ms};
use crate::inbox::Waiter;
use crate::projects;
use crate::service::{COORDINATOR, Core, Input, new_agent};
use crate::worktree::{self, SUBAGENT_PREFIX, WorktreeError};

/// Longer than the hook's own 15-minute timeout (B5), so the hook always reports first.
const SETUP_TIMEOUT: Duration = Duration::from_secs(16 * 60);
const SETUP_POLL: Duration = Duration::from_millis(500);

/// A request to start a slice, from `<leader>sn` or `start_slice`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SliceRequest {
    pub(crate) task: Option<TaskId>,
    pub(crate) name: String,
    pub(crate) brief: String,
    pub(crate) by_coordinator: bool,
}

/// What became of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SliceStart {
    /// Launched; its ready signal follows.
    Started(String),
    /// Over the cap; it starts when a slice is archived.
    Queued(String),
}

/// A slice's agent id.
pub(crate) fn slice_id(name: &str) -> AgentId {
    AgentId(format!("slice-{name}"))
}

impl Core {
    fn open_slices(&self) -> usize {
        self.agents
            .values()
            .filter(|r| r.agent.kind == AgentKind::Slice)
            .count()
    }

    /// Everything that makes a request impossible, checked before asking anyone.
    pub(crate) async fn check_slice(&self, request: &SliceRequest) -> Result<(), String> {
        if self.project.registered().is_none() {
            return Err(
                "this folder is not a Dovetail project yet: add it first (`:project add`, \
                        or the coordinator's `add_project`)"
                    .into(),
            );
        }
        let name = request.name.trim();
        worktree::validate_name(name).map_err(|e| {
            format!("{e}: use letters, digits, `-`, `_` or `.`, not starting with `.` or `-`")
        })?;
        if name.starts_with(SUBAGENT_PREFIX) {
            return Err(format!(
                "slice names may not start with `{SUBAGENT_PREFIX}`: that prefix marks subagent \
                 worktrees, which skip setup"
            ));
        }
        if name == COORDINATOR {
            return Err("`coordinator` is taken".into());
        }
        let taken = self.agents.values().any(|r| r.agent.name == name)
            || self.queue.iter().any(|r| r.name == name);
        if taken {
            return Err(format!("a slice named `{name}` already exists"));
        }
        if let Some(task) = request.task
            && self.task(task).await.is_none()
        {
            return Err(format!(
                "there is no task #{}; create it first with `create_task`",
                task.0
            ));
        }
        Ok(())
    }

    /// Starts a slice now, or queues it when `max_slices` are running (ADR 0004).
    pub(crate) async fn request_slice(
        &mut self,
        request: SliceRequest,
    ) -> Result<SliceStart, String> {
        self.check_slice(&request).await?;
        let name = request.name.trim().to_owned();
        if self.open_slices() >= self.config.max_slices as usize {
            self.activity(
                ActivityKind::Lifecycle,
                None,
                format!(
                    "slice {name} queued: {} slices are running",
                    self.config.max_slices
                ),
            );
            self.queue.push_back(request);
            return Ok(SliceStart::Queued(name));
        }
        self.start_slice(request).await?;
        Ok(SliceStart::Started(name))
    }

    /// Starts queued slices while there is room.
    pub(crate) async fn start_queued(&mut self) {
        while self.open_slices() < self.config.max_slices as usize {
            let Some(request) = self.queue.pop_front() else {
                return;
            };
            let name = request.name.clone();
            let by_coordinator = request.by_coordinator;
            if let Err(e) = self.start_slice(request).await {
                if by_coordinator {
                    self.tell_coordinator(format!("queued slice {name} could not start: {e}"));
                }
                self.fail(format!("queued slice {name} could not start: {e}"));
            }
        }
    }

    async fn start_slice(&mut self, request: SliceRequest) -> Result<(), String> {
        let Some(project) = self.project.registered().cloned() else {
            return Err("this folder is not a Dovetail project yet".into());
        };
        let name = request.name.trim().to_owned();
        let id = slice_id(&name);
        let git = projects::has_git(&project.repo_path);
        let task = match request.task {
            Some(t) => self.task(t).await,
            None => None,
        };

        let mut first_prompt = String::new();
        if !git {
            first_prompt.push_str(&format!(
                "This project has no git repository, so you have no worktree: you work in the \
                 shared project folder `{}`, together with the coordinator and any other slices. \
                 Change only the files your brief gives you.\n\n",
                project.repo_path.display()
            ));
        }
        if let Some(task) = &task {
            first_prompt.push_str(&format!("Task #{}: {}\n\n", task.id.0, task.title));
            if !task.notes.trim().is_empty() {
                first_prompt.push_str(task.notes.trim());
                first_prompt.push_str("\n\n");
            }
        }
        first_prompt.push_str(request.brief.trim());

        let mut agent = new_agent(id.clone(), name.clone(), AgentKind::Slice, request.task);
        agent.status = AgentStatus::SettingUp;
        if git {
            agent.worktree = Some(WorktreeRef {
                path: worktree::worktree_path(&self.paths, &self.key, &name),
                branch: worktree::branch_name(&name),
            });
        }
        let row = AgentRow {
            agent,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            transcript_path: None,
            task_list_id: Some(self.task_list_id(request.task, &name)),
            created_ms: now_ms(),
        };
        self.agents.insert(id.clone(), row);
        self.save_agent(&id);

        if let Some(task) = &task
            && task.status == TaskStatus::Todo
        {
            // Starting a slice starts the task (B4).
            self.update_task(task.id, vec![TaskChange::Status(TaskStatus::InProgress)])
                .await;
        }

        let cwd = project.repo_path.clone();
        if git {
            worktree::clear_hook_outcome(&self.paths, &self.key, &name);
        }
        if let Err(e) = self.launch(&id, cwd, git.then(|| name.clone())) {
            self.closed(&id);
            return Err(e);
        }
        self.activity(
            ActivityKind::Lifecycle,
            Some(&id),
            format!("slice {name} started"),
        );
        let Some(live) = self.live.get_mut(&id) else {
            return Err("the session did not start".into());
        };
        live.brief = Some(first_prompt);
        live.setting_up = true;
        let generation = live.generation;
        if git {
            self.watch_setup(id, name, generation);
        } else {
            self.slice_ready(&id);
        }
        Ok(())
    }

    /// Waits for the worktree hook's outcome off the loop, then reports it as an [`Input`].
    fn watch_setup(&mut self, agent: AgentId, name: String, generation: u64) {
        let (cancel, mut cancelled) = oneshot::channel::<()>();
        if let Some(live) = self.live.get_mut(&agent) {
            live.setup_cancel = Some(cancel);
        }
        let paths = self.paths.clone();
        let key = self.key.clone();
        let inputs = self.inputs.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + SETUP_TIMEOUT;
            let outcome = loop {
                tokio::select! {
                    // The session ended or setup was settled another way.
                    _ = &mut cancelled => return,
                    _ = tokio::time::sleep(SETUP_POLL) => {}
                }
                if let Some(outcome) = worktree::read_hook_outcome(&paths, &key, &name) {
                    break match outcome.error {
                        Some(error) => Err(error),
                        None => Ok(outcome.path.unwrap_or_default()),
                    };
                }
                if tokio::time::Instant::now() >= deadline {
                    break Err("the worktree hook did not report within 16 minutes".into());
                }
            };
            let _ = inputs.send(Input::SetupFinished {
                agent,
                generation,
                outcome,
            });
        });
    }

    pub(crate) async fn on_setup(
        &mut self,
        agent: AgentId,
        generation: u64,
        outcome: Result<std::path::PathBuf, String>,
    ) {
        let waiting = self
            .live
            .get(&agent)
            .is_some_and(|l| l.generation == generation && l.setting_up);
        if !waiting {
            return;
        }
        match outcome {
            Ok(_) => self.slice_ready(&agent),
            Err(reason) => self.setup_failed(&agent, reason),
        }
    }

    /// The worktree is ready: send the brief and tell the coordinator (ADR 0004).
    pub(crate) fn slice_ready(&mut self, agent: &AgentId) {
        let Some(live) = self.live.get_mut(agent) else {
            return;
        };
        live.setting_up = false;
        live.setup_cancel = None;
        let brief = live.brief.take();
        self.refresh_status(agent);
        if let Some(brief) = brief
            && let Err(e) =
                self.send_text(agent, &brief, dovetail_api::EntryKind::User, brief.clone())
        {
            self.fail(e);
        }
        let name = self.name_of(agent);
        self.activity(
            ActivityKind::Lifecycle,
            Some(agent),
            format!("slice {name} ready"),
        );
        self.tell_coordinator(format!("slice {name} ready"));
    }

    /// Setup failed: no first prompt, an inbox error, and the coordinator is told.
    pub(crate) fn setup_failed(&mut self, agent: &AgentId, reason: String) {
        if let Some(live) = self.live.get_mut(agent) {
            live.setting_up = false;
            live.setup_cancel = None;
            live.brief = None;
            live.stopping = true;
            live.session.kill();
        }
        let name = self.name_of(agent);
        self.raise(
            agent,
            InboxKind::Error {
                text: format!("Slice {name}: worktree setup failed: {reason}"),
            },
            Waiter::None,
        );
        self.update_agent(agent, |row| row.agent.status = AgentStatus::Error);
        self.activity(
            ActivityKind::Lifecycle,
            Some(agent),
            format!("slice {name} setup failed"),
        );
        self.tell_coordinator(format!("slice {name} setup failed: {reason}"));
    }

    pub(crate) fn name_of(&self, agent: &AgentId) -> String {
        self.agents
            .get(agent)
            .map_or_else(|| agent.0.clone(), |r| r.agent.name.clone())
    }

    /// Archives a slice: stop its session, then clean up its worktree.
    pub(crate) fn close_slice(&mut self, agent: &AgentId) {
        if self.agents.get(agent).map(|r| r.agent.kind) != Some(AgentKind::Slice) {
            self.fail(format!("{} is not a slice", agent.0));
            return;
        }
        if let Some(live) = self.live.get_mut(agent) {
            live.stopping = true;
            live.close_after = true;
            live.session.shutdown(Duration::from_secs(3));
            return;
        }
        self.finish_close(agent);
    }

    /// The session is gone: remove the worktree if there is one, else close the slice.
    pub(crate) fn finish_close(&mut self, agent: &AgentId) {
        let has_worktree = self
            .agents
            .get(agent)
            .is_some_and(|r| r.agent.worktree.is_some());
        if has_worktree {
            self.remove_worktree(agent, false);
        } else {
            self.closed(agent);
        }
    }

    /// Removes a slice's worktree on a blocking task. Unless `confirmed`, unsaved work stops it
    /// and the user is asked.
    pub(crate) fn remove_worktree(&mut self, agent: &AgentId, confirmed: bool) {
        if self.live.contains_key(agent) {
            // Stop the session first; the removal follows its exit.
            self.close_slice(agent);
            return;
        }
        let Some(row) = self.agents.get(agent) else {
            self.fail(format!("no agent {}", agent.0));
            return;
        };
        if row.agent.worktree.is_none() {
            self.closed(agent);
            return;
        }
        let name = row.agent.name.clone();
        let paths = self.paths.clone();
        let key = self.key.clone();
        let repo = self.project.repo_path().to_path_buf();
        let inputs = self.inputs.clone();
        let agent = agent.clone();
        tokio::task::spawn_blocking(move || {
            let result = worktree::remove(&paths, &key, &name, &repo, confirmed);
            let _ = inputs.send(Input::WorktreeRemoved { agent, result });
        });
    }

    pub(crate) async fn on_worktree_removed(
        &mut self,
        agent: AgentId,
        result: Result<(), WorktreeError>,
    ) {
        match result {
            Ok(()) => self.closed(&agent),
            Err(WorktreeError::Unsafe(report)) => {
                let worktree = self
                    .agents
                    .get(&agent)
                    .and_then(|r| r.agent.worktree.clone());
                let input = json!({
                    "worktree": worktree.as_ref().map(|w| w.path.clone()),
                    "branch": worktree.as_ref().map(|w| w.branch.clone()),
                    "uncommitted": report.uncommitted,
                    "unpushed_commits": report.unpushed_commits,
                });
                self.raise(
                    &agent,
                    InboxKind::Permission {
                        tool: "Remove worktree".into(),
                        input,
                        reason: Some(
                            "the slice has uncommitted or unpushed work, which removing it \
                             would lose"
                                .into(),
                        ),
                        suggestions: Vec::new(),
                    },
                    Waiter::RemoveWorktree,
                );
            }
            Err(e) => {
                let name = self.name_of(&agent);
                self.raise(
                    &agent,
                    InboxKind::Error {
                        text: format!("Slice {name}: removing its worktree failed: {e}"),
                    },
                    Waiter::None,
                );
                self.refresh_status(&agent);
            }
        }
    }

    /// The user kept a slice with unsaved work: it stays, idle, with its worktree.
    pub(crate) fn keep_slice(&mut self, agent: &AgentId) {
        self.notice(agent, "Kept: the worktree has unsaved work");
        self.refresh_status(agent);
    }

    /// The slice is gone for good: forget it and make room for a queued one.
    pub(crate) fn closed(&mut self, agent: &AgentId) {
        let Some(row) = self.agents.remove(agent) else {
            return;
        };
        self.live.remove(agent);
        self.tails.remove(agent);
        let items: Vec<_> = self
            .inbox
            .values()
            .filter(|p| &p.item.agent == agent)
            .map(|p| p.item.id)
            .collect();
        for id in items {
            self.resolve(id, Some("slice archived".into()));
        }
        worktree::clear_hook_outcome(&self.paths, &self.key, &row.agent.name);
        self.persist("a closed agent", self.db.close_agent(agent.clone()));
        self.emit(Event::AgentRemoved(agent.clone()));
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!("slice {} archived", row.agent.name),
        );
    }
}

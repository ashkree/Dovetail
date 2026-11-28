//! Agents domain: slices, side chats, tangents, reports, journals, todos, attachments, completion,
//! and agy model discovery.

pub(crate) mod agy_models;
pub(crate) mod attachments;
pub(crate) mod completion;
pub(crate) mod journal;
pub(crate) mod report;
pub(crate) mod side_chat;
pub(crate) mod slice;
pub(crate) mod tangents;
pub(crate) mod todos;

pub use slice::{SetupFailure, SliceError};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::Duration;

use dovetail_api::Event;
use dovetail_api::activity::inbox::{InboxKind, START_SLICE_PERMISSION};
use dovetail_api::agents::{
    agent::{
        Agent, AgentCommand, AgentEvent, AgentId, AgentKind, AgentStatus, PermissionMode,
        SessionLock, SlashCommand,
    },
    conversation::{
        Attachment, ConversationCommand, ConversationEntry, ConversationEvent, ConversationTail,
        EntryId, EntryKind,
    },
    journal::JournalCommand,
    model::{DriverKind, ModelCommand},
    tangent::TangentCommand,
};
use dovetail_api::snapshot::CONVERSATION_TAIL;
use dovetail_api::tracking::task::TaskId;
use dovetail_tools::{
    SocketResponseBody,
    project::{ArchiveSlice, StartSlice, archive_result::ARCHIVED},
    report::ReportStatus,
};
use serde_json::json;
use tokio::sync::oneshot;

use crate::activity::inbox::Waiter;
use crate::engine::{Core, EntryTag, Input};
use crate::mcp::tools::{error, ok, slice_reply};
use crate::storage::{AgentRow, Db, ModelChoice, QueuedSlice, now_ms};

/// The streaming coalescing window (≤ ~30 fps).
const DELTA_WINDOW: Duration = Duration::from_millis(33);
/// The model a coordinator-started slice gets when it names none (task #42).
const DEFAULT_SLICE_MODEL: &str = "sonnet";
/// Most characters of a status line (B17).
const STATUS_MAX: usize = 120;

/// State owned by the agents domain: agent rows, conversation tails, queues, todos, and models.
pub(crate) struct AgentState {
    pub(crate) rows: BTreeMap<AgentId, AgentRow>,
    pub(crate) tails: BTreeMap<AgentId, ConversationTail>,
    pub(crate) next_entry: u64,
    /// Streamed text not yet sent, coalesced to at most ~30 updates a second.
    pub(crate) deltas: BTreeMap<(AgentId, EntryId), String>,
    pub(crate) flush_at: Option<tokio::time::Instant>,
    /// Slice starts over the `max_slices` cap, oldest first (ADR 0004).
    pub(crate) queue: VecDeque<QueuedSlice>,
    /// `project_archive_slice` / `tasks_archive` calls waiting for their slices' outcomes (task #9).
    pub(crate) archive_calls: Vec<slice::ArchiveCall>,
    /// Slices whose archive call returned `waiting_for_user`: the coordinator is told how the
    /// user decided.
    pub(crate) archive_told: HashSet<AgentId>,
    /// Archived slices and the task each worked on, kept so a pull-request lookup started as
    /// the slice was archived can still move its task (ADR 0024).
    pub(crate) closed_slices: HashMap<AgentId, TaskId>,
    /// Signals for a coordinator that may not be woken (it crashed, or the app is quitting);
    /// sent when its session next starts.
    pub(crate) coordinator_notes: Vec<String>,
    /// The coordinator's last turn was cut off by a restart: held notes wait for the user's
    /// next message to it instead of waking it.
    pub(crate) notes_wait_for_user: bool,
    /// Native to-do lists: items, owners, the watcher (M11, `todos`).
    pub(crate) todos: todos::Todos,
    /// Each session's slash commands and skills, as it last listed them; kept after the process
    /// stops so `/` completion still works for a dormant slice.
    pub(crate) commands: BTreeMap<AgentId, Vec<SlashCommand>>,
    /// An `:eod` waiting for the coordinator's handoff (ADR 0012, `journal`).
    pub(crate) closing: Option<journal::Closing>,
    /// A `:fresh` whose handoff is written: the new coordinator starts once the old exits.
    pub(crate) pending_fresh: Option<journal::PendingFresh>,
    /// Tangents' database (ADR 0017, `tangents`), or why tangents are off.
    pub(crate) tangents: Result<Db, tangents::TangentError>,
    /// The models sessions last listed (`list_models` / ACP config options), per driver.
    pub(crate) model_catalog: BTreeMap<DriverKind, Vec<dovetail_driver::ModelInfo>>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
            tails: BTreeMap::new(),
            next_entry: 1,
            deltas: BTreeMap::new(),
            flush_at: None,
            queue: VecDeque::new(),
            archive_calls: Vec::new(),
            archive_told: HashSet::new(),
            closed_slices: HashMap::new(),
            coordinator_notes: Vec::new(),
            notes_wait_for_user: false,
            todos: Default::default(),
            commands: BTreeMap::new(),
            closing: None,
            pending_fresh: None,
            tangents: Err(tangents::TangentError::Off("not opened".into())),
            model_catalog: BTreeMap::new(),
        }
    }
}

/// A fresh agent, idle and in auto mode.
pub(crate) fn new_agent(id: AgentId, name: String, kind: AgentKind, task: Option<TaskId>) -> Agent {
    Agent {
        id,
        name,
        kind,
        parent: None,
        task,
        worktree: None,
        status: AgentStatus::Idle,
        status_line: String::new(),
        memory_mb: None,
        cost_usd: 0.0,
        permission_mode: PermissionMode::Auto,
        context_pct: None,
        progress: None,
        last_activity_ms: now_ms(),
        lock: SessionLock::Headless,
        model: None,
        effort: None,
        subagent_tokens: 0,
        tool: None,
        driver: DriverKind::Claude,
    }
}

impl Core {
    pub(crate) async fn handle_agent_command(&mut self, command: AgentCommand) {
        match command {
            AgentCommand::Interrupt { agent } => self.interrupt(&agent),
            AgentCommand::CyclePermissionMode { agent } => self.cycle_mode(&agent),
            AgentCommand::NewSideChat { question } => self.start_side_chat(question),
            AgentCommand::NewSlice {
                task,
                name,
                brief,
                repo,
            } => {
                let request = slice::SliceRequest {
                    task,
                    name,
                    brief,
                    by_coordinator: false,
                    repo,
                    model_choice: None,
                    driver: DriverKind::Claude,
                };
                if let Err(message) = self.request_slice(request).await {
                    self.fail(message);
                }
            }
            AgentCommand::CloseSlice { agent } => self.close_slice(&agent, false),
        }
    }

    pub(crate) async fn handle_conversation_command(&mut self, command: ConversationCommand) {
        match command {
            ConversationCommand::Prompt {
                agent,
                text,
                attachments,
                effort,
            } => self.prompt(&agent, text, attachments, effort).await,
            ConversationCommand::ListPaths { agent, dir } => self.list_paths(agent, dir),
            ConversationCommand::Load { agent, before } => {
                let before = before.or_else(|| {
                    self.agents
                        .tails
                        .get(&agent)
                        .and_then(|t| t.entries.first().map(|e| e.id))
                });
                match self
                    .db
                    .load_entries(agent.clone(), before, CONVERSATION_TAIL)
                    .await
                {
                    Ok((entries, has_more)) => {
                        self.emit(Event::Conversation(ConversationEvent::Page {
                            agent,
                            entries,
                            has_more,
                        }))
                    }
                    Err(e) => self.fail(e.to_string()),
                }
            }
        }
    }

    pub(crate) async fn handle_tangent_command(&mut self, command: TangentCommand) {
        match command {
            TangentCommand::New { question } => self.start_tangent(question).await,
            TangentCommand::Attach { tangent, task } => self.attach_tangent(tangent, task).await,
            TangentCommand::File { tangent, project } => {
                self.file_tangent(tangent, project).await;
            }
            TangentCommand::StartFrom { tangent, project } => {
                self.start_from_tangent(tangent, project).await;
            }
            TangentCommand::Drop { tangent } => self.drop_tangent(tangent).await,
            TangentCommand::Load { tangent, what } => self.load_tangent(tangent, what).await,
        }
    }

    pub(crate) fn handle_journal_command(&mut self, command: JournalCommand) {
        match command {
            JournalCommand::EndOfDay => self.end_of_day(),
            JournalCommand::Handoff => self.handoff_start(),
            JournalCommand::FreshSession => self.fresh_session(),
        }
    }

    pub(crate) fn handle_model_command(&mut self, command: ModelCommand) {
        match command {
            ModelCommand::Set {
                agent,
                model,
                effort,
            } => self.switch_model(&agent, model, effort),
            ModelCommand::SwitchDriver {
                agent,
                driver,
                model,
                effort,
                handoff,
            } => self.switch_driver(&agent, driver, model, effort, handoff),
        }
    }

    pub(crate) fn handle_report_status_tool(
        &mut self,
        agent: &AgentId,
        report: ReportStatus,
    ) -> SocketResponseBody {
        let line: String = report
            .status
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(STATUS_MAX)
            .collect();
        self.update_agent(agent, |row| row.agent.status_line = line);
        ok("Status set.")
    }

    pub(crate) async fn handle_start_slice_tool(
        &mut self,
        agent: &AgentId,
        start: StartSlice,
    ) -> SocketResponseBody {
        let driver = start.driver.unwrap_or(DriverKind::Claude);
        if driver == DriverKind::Agy && start.model.is_none() && start.effort.is_some() {
            return error("denied: `effort` on an agy slice requires `model`");
        }
        let model_choice = match driver {
            // Always explicit for Claude: a slice never runs on Claude Code's own default.
            DriverKind::Claude => Some(ModelChoice {
                model: start
                    .model
                    .map_or_else(|| DEFAULT_SLICE_MODEL.to_owned(), |m| m.trim().to_owned()),
                effort: start.effort,
            }),
            DriverKind::Agy => start.model.map(|m| ModelChoice {
                model: m.trim().to_owned(),
                effort: start.effort,
            }),
        };
        let request = slice::SliceRequest {
            task: Some(start.task),
            name: start.name.trim().to_owned(),
            brief: start.brief,
            by_coordinator: true,
            repo: start.repo,
            model_choice,
            driver,
        };
        if let Err(reason) = self.check_slice(&request).await {
            return error(format!("denied: {reason}"));
        }
        let mode = self
            .agents
            .rows
            .get(agent)
            .map_or(PermissionMode::Manual, |r| r.agent.permission_mode);
        if mode != PermissionMode::Auto {
            // Approval follows the coordinator's permission mode (ADR 0004).
            let name = request.name.clone();
            let input = json!({
                "task": request.task.map(|t| t.0),
                "name": request.name,
                "brief": request.brief,
                "repo": request.repo,
                "driver": request.driver,
                "model": request.model_choice.as_ref().map(|c| &c.model),
                "effort": request.model_choice.as_ref().and_then(|c| c.effort),
            });
            self.raise(
                agent,
                InboxKind::Permission {
                    tool: START_SLICE_PERMISSION.into(),
                    input,
                    reason: Some("the coordinator wants to start a slice".into()),
                    suggestions: Vec::new(),
                },
                Waiter::StartSlice { request },
            );
            // The call returns at once; the answer comes as a message (decided
            // 2026-11-06, ADR 0004).
            return ok(format!(
                "pending: the user is asked to approve slice {name}. The answer arrives \
                 as a message: \"project_start_slice {name} approved\" (then wait for \"slice \
                 {name} ready\") or \"project_start_slice {name} denied\"."
            ));
        }
        let result = self.request_slice(request).await;
        slice_reply(result)
    }

    pub(crate) async fn handle_archive_slice_tool(
        &mut self,
        archive: ArchiveSlice,
        reply: oneshot::Sender<SocketResponseBody>,
    ) {
        let name = archive.name.trim().to_owned();
        if let Some(at) = self.agents.queue.iter().position(|q| q.name == name) {
            // Not started yet: dropping the queued start is all there is to archive.
            if let Some(queued) = self.agents.queue.remove(at) {
                self.persist("the slice queue", self.db.dequeue_slice(queued.id));
            }
            let _ = reply.send(ok(format!(
                "{ARCHIVED}: slice {name} was still queued; its start is dropped."
            )));
        } else {
            match self.slice_named(&name) {
                Err(reason) => {
                    let _ = reply.send(error(reason.to_string()));
                }
                Ok(slice) => {
                    self.archive_for_tool(vec![slice], None, reply).await;
                }
            }
        }
    }

    /// Removes saved paste attachments older than a week that no live agent in any project
    /// names, on a blocking thread so the first frame never waits for it.
    pub(crate) fn sweep_attachments(&self) {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || {
            let Some(keep) = crate::storage::live_attachments(&paths) else {
                tracing::warn!("attachment sweep skipped: a project database could not be read");
                return;
            };
            let dir = paths.attachments();
            let n = attachments::sweep(
                &dir,
                std::time::SystemTime::now(),
                attachments::MAX_AGE,
                &keep,
            );
            if n > 0 {
                tracing::info!("removed {n} old attachment files");
            }
        });
    }

    /// Deletes the saved images an archived agent's messages named.
    pub(crate) fn remove_attachments_of(&self, agent: &AgentId) {
        let pending = self.db.agent_attachments(agent.clone());
        let dir = self.paths.attachments();
        tokio::spawn(async move {
            match pending.await {
                Ok(files) => {
                    let _ = tokio::task::spawn_blocking(move || attachments::remove(&dir, &files))
                        .await;
                }
                Err(e) => tracing::warn!("listing an archived agent's attachments: {e}"),
            }
        });
    }

    /// Lists paths for `@` completion on a blocking thread (git can take a moment on a big
    /// tree), relative to the agent's folder: its worktree, else the main checkout.
    fn list_paths(&mut self, agent: AgentId, dir: Option<String>) {
        let root = self
            .agents
            .rows
            .get(&agent)
            .and_then(|r| r.agent.worktree.as_ref())
            .map(|w| w.path.clone())
            .or_else(|| self.repo_of(&agent).map(|r| r.path))
            .unwrap_or_else(|| self.tracking.project.coordinator_folder(&self.paths));
        let inputs = self.inputs.clone();
        // A research project has no repo: no git, just the folder listing as it is typed.
        let git = !self.tracking.project.is_research();
        tokio::task::spawn_blocking(move || {
            let paths = match dir.as_deref() {
                None if git => completion::tree(&root),
                None => Vec::new(),
                Some(d) => completion::dir(&root, d),
            };
            let _ = inputs.send(Input::PathsListed { agent, dir, paths });
        });
    }

    // --- streaming text ---

    /// Queues streamed text; it goes out within [`DELTA_WINDOW`].
    pub(crate) fn queue_delta(&mut self, agent: &AgentId, entry: EntryId, text: &str) {
        if let Some(tail) = self.agents.tails.get_mut(agent)
            && let Some(e) = tail.entries.iter_mut().find(|e| e.id == entry)
        {
            e.text.push_str(text);
        }
        self.agents
            .deltas
            .entry((agent.clone(), entry))
            .or_default()
            .push_str(text);
        if self.agents.flush_at.is_none() {
            self.agents.flush_at = Some(tokio::time::Instant::now() + DELTA_WINDOW);
        }
    }

    pub(crate) fn flush_deltas(&mut self) {
        self.agents.flush_at = None;
        for ((agent, entry), text) in std::mem::take(&mut self.agents.deltas) {
            self.send_event(Event::Conversation(ConversationEvent::AssistantDelta {
                agent,
                entry,
                text,
            }));
        }
    }

    // --- conversation entries ---

    /// Adds an entry to an agent's conversation, tells the UI and stores it.
    pub(crate) fn add_entry(&mut self, agent: &AgentId, kind: EntryKind, text: String) -> EntryId {
        self.add_entry_with(agent, kind, text, Vec::new())
    }

    /// [`add_entry`](Self::add_entry) with images attached (a user message).
    pub(crate) fn add_entry_with(
        &mut self,
        agent: &AgentId,
        kind: EntryKind,
        text: String,
        attachments: Vec<Attachment>,
    ) -> EntryId {
        self.add_entry_tagged(agent, kind, text, attachments, EntryTag::ByKind)
    }

    /// [`add_entry_with`](Self::add_entry_with), saying how the entry gets its effort tag.
    pub(crate) fn add_entry_tagged(
        &mut self,
        agent: &AgentId,
        kind: EntryKind,
        text: String,
        attachments: Vec<Attachment>,
        tag: EntryTag,
    ) -> EntryId {
        let effort = match tag {
            EntryTag::ByKind => self.entry_tag(agent, kind),
            EntryTag::Effort(effort) => effort,
        };
        let entry = ConversationEntry {
            id: EntryId(self.agents.next_entry),
            at_ms: now_ms(),
            kind,
            text,
            attachments,
            effort,
        };
        self.agents.next_entry += 1;
        let tail = self.agents.tails.entry(agent.clone()).or_default();
        tail.entries.push(entry.clone());
        if tail.entries.len() > CONVERSATION_TAIL {
            let excess = tail.entries.len() - CONVERSATION_TAIL;
            tail.entries.drain(..excess);
            tail.has_more = true;
        }
        let id = entry.id;
        // A streaming entry is stored once its text is complete.
        if !(kind == EntryKind::Assistant && entry.text.is_empty()) {
            self.persist(
                "a conversation entry",
                self.db.upsert_entry(agent.clone(), entry.clone()),
            );
        }
        self.emit(Event::Conversation(ConversationEvent::Appended {
            agent: agent.clone(),
            entry,
        }));
        id
    }

    /// Replaces an entry's text, tells the UI and stores it.
    pub(crate) fn set_entry_text(&mut self, agent: &AgentId, id: EntryId, text: String) {
        let Some(entry) = self
            .agents
            .tails
            .get_mut(agent)
            .and_then(|t| t.entries.iter_mut().find(|e| e.id == id))
        else {
            return;
        };
        entry.text = text;
        let entry = entry.clone();
        // Streamed text for it is superseded.
        self.agents.deltas.remove(&(agent.clone(), id));
        self.persist(
            "a conversation entry",
            self.db.upsert_entry(agent.clone(), entry.clone()),
        );
        self.emit(Event::Conversation(ConversationEvent::Appended {
            agent: agent.clone(),
            entry,
        }));
    }

    pub(crate) fn notice(&mut self, agent: &AgentId, text: impl Into<String>) {
        self.add_entry(agent, EntryKind::Notice, text.into());
    }

    // --- agents ---

    /// Tells the UI about an agent and stores it.
    pub(crate) fn save_agent(&mut self, id: &AgentId) {
        let Some(row) = self.agents.rows.get(id).cloned() else {
            return;
        };
        self.persist("an agent", self.db.upsert_agent(row.clone()));
        self.emit(Event::Agent(AgentEvent::Upserted(row.agent)));
    }

    /// Changes an agent and, if anything changed, tells the UI and stores it.
    pub(crate) fn update_agent(&mut self, id: &AgentId, change: impl FnOnce(&mut AgentRow)) {
        let Some(row) = self.agents.rows.get_mut(id) else {
            return;
        };
        let before = row.clone();
        change(row);
        if *row != before {
            self.save_agent(id);
        }
    }

    /// Recomputes an agent's status from its session, its inbox items and its setup.
    pub(crate) fn refresh_status(&mut self, id: &AgentId) {
        // While quitting, a session that has exited keeps the status it had, so a turn cut off
        // by the quit reads as cut off on the next open (M10). A turn that still ends normally
        // in the quit window is recorded as ended.
        if self.quitting.is_some() && !self.sessions.live.contains_key(id) {
            return;
        }
        // A subagent's status comes from its `task_*` events (see `session::subagent`).
        if self
            .agents
            .rows
            .get(id)
            .is_some_and(|r| !r.agent.kind.is_session())
        {
            return;
        }
        let needs_you = self
            .activity
            .inbox
            .values()
            .any(|p| &p.item.agent == id && p.waiter.holds_agent());
        let (setting_up, busy) = match self.sessions.live.get(id) {
            Some(live) => (live.setting_up, live.turn.busy()),
            None => (false, false),
        };
        let failed = self
            .agents
            .rows
            .get(id)
            .is_some_and(|r| r.agent.status == AgentStatus::Error);
        let status = crate::sessions::state::StatusInputs {
            setting_up,
            needs_you,
            busy,
            failed,
        }
        .status();
        self.update_agent(id, |row| {
            row.agent.status = status;
            row.agent.last_activity_ms = now_ms();
        });
    }
}

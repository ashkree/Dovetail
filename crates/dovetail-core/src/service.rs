//! The real core: one thread that owns the event sequence, talks to the DB thread, the socket
//! and the sessions, and hands the UI the same [`CoreLink`] as `MockCore`.
//!
//! Everything that changes state goes through one loop ([`Core::run`]): UI commands, socket
//! requests (`dovetail add`, tool calls), driver events and finished background jobs (worktree
//! setup and removal). Handlers never block the loop: session methods only enqueue, git work runs
//! on blocking tasks and reports back as an [`Input`].
//!
//! The rest of `impl Core` lives with its subject: `session` (launching and the stream),
//! `inbox` (Needs you and its answers), `slice` (start, ready signal, archive) and `tools`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::{
    ACTIVITY_TAIL, ActivityItem, ActivityKind, Agent, AgentId, AgentKind, AgentMessage,
    AgentStatus, Attachment, CONVERSATION_TAIL, Command, ConversationEntry, ConversationTail,
    CoreLink, DecisionAuthor, EntryId, EntryKind, Event, InboxId, InboxItem, MESSAGE_TAIL, NewTask,
    PROTOCOL_VERSION, PermissionMode, ServerMsg, SessionLock, Snapshot, Task, TaskChange, TaskId,
    TaskStatus, Totals, Warning, WarningId,
};
use dovetail_driver::{DriverEvent, SessionDriver};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

use crate::config::Config;
use crate::db::{AgentRow, Db, DbError, Pending, now_ms};
use crate::inbox::Waiter;
use crate::paths::Paths;
use crate::projects::{Project, Resolution};
use crate::registration::ProjectState;
use crate::session::Live;
use crate::shutdown::CoreHandle;
use crate::slice::SliceRequest;
use crate::socket::{self, BoxFuture, HandlerError, SocketError, SocketHandler, ToolCall};
use crate::worktree::WorktreeError;

/// The coordinator's agent id: one per project.
pub const COORDINATOR: &str = "coordinator";

/// Errors starting the core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The databases could not be opened.
    #[error(transparent)]
    Db(#[from] DbError),
    /// The socket could not be created.
    #[error(transparent)]
    Socket(#[from] SocketError),
    /// The core thread could not be started.
    #[error("starting the core thread: {0}")]
    Thread(std::io::Error),
}

/// What the core needs from the binary.
#[derive(Clone)]
pub struct CoreOptions {
    /// Starts sessions: `ClaudeDriver` in the app, `FakeDriver` in tests.
    pub driver: Arc<dyn SessionDriver>,
    /// The `dovetail` binary, for the worktree hook and the MCP bridge in launch settings.
    pub exe: PathBuf,
}

/// Starts the core on its own thread for the folder `resolution` names, and returns the UI's end
/// of the link plus a handle to wait for it on exit.
///
/// The first messages are `Hello` and a `Snapshot`. The core listens on the project's socket
/// (an unregistered folder gets a private one) for `dovetail add` and the MCP bridges.
pub fn spawn(
    resolution: Resolution,
    paths: Paths,
    config: Config,
    options: CoreOptions,
) -> Result<(CoreLink, CoreHandle), CoreError> {
    let (project, db) = match resolution {
        Resolution::Registered(p) => {
            let db = Db::open(&paths, Some(&p.id))?;
            (ProjectState::Registered(p), db)
        }
        Resolution::Candidate(candidate) => {
            let db = Db::open_unregistered(&paths)?;
            let key = format!("new-{}", std::process::id());
            (ProjectState::Unregistered { candidate, key }, db)
        }
    };
    let socket_path = paths.socket(project.key());
    let listener = socket::bind(&socket_path)?;

    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (message_tx, message_rx) = mpsc::channel::<ServerMsg>();
    let (input_tx, input_rx) = tokio_mpsc::unbounded_channel::<Input>();

    // Commands arrive on a std channel (the UI is synchronous); forward them into the loop.
    let forward = input_tx.clone();
    std::thread::Builder::new()
        .name("dovetail-commands".into())
        .spawn(move || {
            for command in command_rx {
                if forward.send(Input::Command(command)).is_err() {
                    return;
                }
            }
            // The UI dropped its end.
            let _ = forward.send(Input::Command(Command::Quit { end_sessions: true }));
        })
        .map_err(CoreError::Thread)?;

    let thread = std::thread::Builder::new()
        .name("dovetail-core".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    tracing::error!("starting the core runtime: {e}");
                    return;
                }
            };
            let mut sockets = vec![socket_path.clone()];
            let more = runtime.block_on(async move {
                let handler: Arc<dyn SocketHandler> = Arc::new(CoreHandler {
                    inputs: input_tx.clone(),
                });
                tokio::spawn(serve(listener, handler));
                let core = Core::new(
                    db,
                    paths,
                    config,
                    options,
                    project,
                    socket_path,
                    message_tx,
                    input_tx,
                );
                core.run(input_rx).await
            });
            sockets.extend(more);
            for path in sockets {
                let _ = std::fs::remove_file(path);
            }
        })
        .map_err(CoreError::Thread)?;

    Ok((
        CoreLink {
            commands: command_tx,
            messages: message_rx,
        },
        CoreHandle { thread },
    ))
}

pub(crate) async fn serve(
    listener: std::os::unix::net::UnixListener,
    handler: Arc<dyn SocketHandler>,
) {
    if let Err(e) = socket::serve(listener, handler).await {
        tracing::error!("socket server stopped: {e}");
    }
}

/// What the core loop reacts to.
pub(crate) enum Input {
    Command(Command),
    /// A task from `dovetail add` over the socket.
    AddTask {
        task: NewTask,
        reply: oneshot::Sender<Result<TaskId, HandlerError>>,
    },
    /// A tool call from a session's MCP bridge.
    Tool {
        call: ToolCall,
        reply: oneshot::Sender<SocketResponseBody>,
    },
    /// Something a session reported. `generation` tells a restarted session's events from a
    /// stale process's.
    Driver {
        agent: AgentId,
        generation: u64,
        event: DriverEvent,
    },
    /// A slice's worktree hook finished: the worktree path, or why it failed.
    SetupFinished {
        agent: AgentId,
        generation: u64,
        outcome: Result<PathBuf, crate::error::SetupFailure>,
    },
    /// A slice's worktree removal finished.
    WorktreeRemoved {
        agent: AgentId,
        result: Result<(), WorktreeError>,
    },
    /// The sweep inspected a worktree no agent owns.
    OrphanInspected {
        found: crate::restore::Orphan,
        result: Result<crate::worktree::Inspection, WorktreeError>,
    },
    /// An unowned worktree's removal (after `y`) finished.
    OrphanRemoved {
        name: String,
        result: Result<(), WorktreeError>,
    },
}

/// Hands socket requests to the loop.
pub(crate) struct CoreHandler {
    pub(crate) inputs: tokio_mpsc::UnboundedSender<Input>,
}

impl SocketHandler for CoreHandler {
    fn add_task(&self, task: NewTask) -> BoxFuture<Result<TaskId, HandlerError>> {
        let (reply, answer) = oneshot::channel();
        let sent = self.inputs.send(Input::AddTask { task, reply });
        Box::pin(async move {
            sent.map_err(|_| HandlerError::ShuttingDown)?;
            answer.await.map_err(|_| HandlerError::ShuttingDown)?
        })
    }

    fn tool_call(&self, call: ToolCall) -> BoxFuture<SocketResponseBody> {
        let (reply, answer) = oneshot::channel();
        let sent = self.inputs.send(Input::Tool { call, reply });
        Box::pin(async move {
            let shutting_down = SocketResponseBody::Error {
                message: "Dovetail is shutting down".into(),
            };
            if sent.is_err() {
                return shutting_down;
            }
            answer.await.unwrap_or(shutting_down)
        })
    }
}

/// An inbox item and whatever waits on its answer.
pub(crate) struct InboxEntry {
    pub(crate) item: InboxItem,
    pub(crate) waiter: Waiter,
}

/// The core's state. Fields are shared with the sibling modules that hold the rest of `impl Core`.
pub(crate) struct Core {
    pub(crate) db: Db,
    pub(crate) paths: Paths,
    pub(crate) config: Config,
    pub(crate) options: CoreOptions,
    pub(crate) project: ProjectState,
    /// The socket the bridges of this app's sessions use.
    pub(crate) socket: PathBuf,
    /// Sockets bound since start (after registration), removed on exit.
    pub(crate) extra_sockets: Vec<PathBuf>,
    out: mpsc::Sender<ServerMsg>,
    pub(crate) inputs: tokio_mpsc::UnboundedSender<Input>,
    seq: u64,
    ui_alive: bool,
    pub(crate) agents: BTreeMap<AgentId, AgentRow>,
    pub(crate) live: HashMap<AgentId, Live>,
    pub(crate) generation: u64,
    pub(crate) tails: BTreeMap<AgentId, ConversationTail>,
    pub(crate) inbox: BTreeMap<InboxId, InboxEntry>,
    pub(crate) activity: VecDeque<ActivityItem>,
    pub(crate) messages: VecDeque<AgentMessage>,
    pub(crate) warnings: Vec<Warning>,
    totals: Totals,
    pub(crate) next_entry: u64,
    pub(crate) next_inbox: u64,
    next_activity: u64,
    next_message: u64,
    /// Streamed text not yet sent, coalesced to at most ~30 updates a second.
    deltas: BTreeMap<(AgentId, EntryId), String>,
    flush_at: Option<tokio::time::Instant>,
    /// Slice starts over the `max_slices` cap, oldest first (ADR 0004).
    pub(crate) queue: VecDeque<crate::db::QueuedSlice>,
    /// `archive_slice` / `archive_task` calls waiting for their slices' outcomes (task #9).
    pub(crate) archive_calls: Vec<crate::slice::ArchiveCall>,
    /// Slices whose archive call returned `waiting_for_user`: the coordinator is told how the
    /// user decided.
    pub(crate) archive_told: std::collections::HashSet<AgentId>,
    /// `msg_id`s already on the message timeline, so sender and receiver don't both add one.
    pub(crate) seen_messages: std::collections::HashSet<String>,
    /// Signals for a coordinator that may not be woken (it crashed, or the app is quitting);
    /// sent when its session next starts.
    pub(crate) coordinator_notes: Vec<String>,
    /// The coordinator's last turn was cut off by a restart: held notes wait for the user's
    /// next message to it instead of waking it.
    pub(crate) notes_wait_for_user: bool,
    pub(crate) quitting: Option<tokio::time::Instant>,
    pub(crate) killed: bool,
}

/// The streaming coalescing window (≤ ~30 fps).
const DELTA_WINDOW: Duration = Duration::from_millis(33);

impl Core {
    #[allow(clippy::too_many_arguments)]
    fn new(
        db: Db,
        paths: Paths,
        config: Config,
        options: CoreOptions,
        project: ProjectState,
        socket: PathBuf,
        out: mpsc::Sender<ServerMsg>,
        inputs: tokio_mpsc::UnboundedSender<Input>,
    ) -> Self {
        Core {
            db,
            paths,
            config,
            options,
            project,
            socket,
            extra_sockets: Vec::new(),
            out,
            inputs,
            seq: 0,
            ui_alive: true,
            agents: BTreeMap::new(),
            live: HashMap::new(),
            generation: 0,
            tails: BTreeMap::new(),
            inbox: BTreeMap::new(),
            activity: VecDeque::new(),
            messages: VecDeque::new(),
            warnings: Vec::new(),
            totals: Totals::default(),
            next_entry: 1,
            next_inbox: 1,
            next_activity: 1,
            next_message: 1,
            deltas: BTreeMap::new(),
            flush_at: None,
            queue: VecDeque::new(),
            archive_calls: Vec::new(),
            archive_told: Default::default(),
            seen_messages: Default::default(),
            coordinator_notes: Vec::new(),
            notes_wait_for_user: false,
            quitting: None,
            killed: false,
        }
    }

    /// Runs until the UI quits and every session has exited. Returns sockets to remove.
    async fn run(mut self, mut inputs: tokio_mpsc::UnboundedReceiver<Input>) -> Vec<PathBuf> {
        let restored = match self.load().await {
            Ok(restored) => restored,
            Err(e) => {
                tracing::error!("loading state: {e}");
                self.fail(format!("loading state: {e}"));
                crate::restore::Restored::default()
            }
        };
        self.startup_warnings();
        self.send_snapshot(true).await;
        self.reconcile(restored).await;
        self.start_queued().await;
        loop {
            if self.quitting.is_some() && self.live.is_empty() {
                break;
            }
            let flush_at = self.flush_at;
            let quit_at = self.quitting;
            let input = tokio::select! {
                input = inputs.recv() => input,
                _ = sleep_until(flush_at), if flush_at.is_some() => {
                    self.flush_deltas();
                    continue;
                }
                _ = sleep_until(quit_at), if quit_at.is_some() => {
                    if self.quit_deadline() {
                        break;
                    }
                    continue;
                }
            };
            let Some(input) = input else { break };
            match input {
                Input::Command(Command::Quit { .. }) => self.begin_quit(),
                Input::Command(command) if self.quitting.is_none() => self.command(command).await,
                Input::Command(_) => {}
                Input::AddTask { task, reply } => {
                    let result = if self.project.registered().is_none() {
                        Err(HandlerError::NotRegistered)
                    } else {
                        self.create_task(task, false)
                            .await
                            .map_err(HandlerError::from)
                    };
                    let _ = reply.send(result);
                }
                Input::Tool { call, reply } => self.tool(call, reply).await,
                Input::Driver {
                    agent,
                    generation,
                    event,
                } => self.on_driver(agent, generation, event).await,
                // During quit nothing new starts, and no signal wakes the coordinator.
                Input::SetupFinished { .. } if self.quitting.is_some() => {}
                Input::SetupFinished {
                    agent,
                    generation,
                    outcome,
                } => self.on_setup(agent, generation, outcome).await,
                Input::WorktreeRemoved { agent, result } => {
                    self.on_worktree_removed(agent, result).await;
                }
                Input::OrphanInspected { found, result } => self.on_orphan(found, result),
                Input::OrphanRemoved { name, result } => self.on_orphan_removed(name, result),
            }
            if self.quitting.is_none() {
                self.start_queued().await;
            }
            self.refresh_totals();
            if !self.ui_alive && self.quitting.is_none() {
                self.begin_quit();
            }
        }
        self.flush_deltas();
        std::mem::take(&mut self.extra_sockets)
    }

    /// Loads agents, conversation tails and id counters, and makes sure a coordinator exists.
    async fn load(&mut self) -> Result<crate::restore::Restored, DbError> {
        let mut restored = crate::restore::Restored::default();
        let ids = self.db.max_ids().await?;
        self.next_entry = ids.entry + 1;
        self.next_inbox = ids.inbox + 1;
        self.next_activity = ids.activity + 1;
        self.next_message = ids.message + 1;
        for mut row in self.db.load_agents().await? {
            // Nothing runs yet: whatever it was doing was cut off (M10, see `restore`).
            if matches!(
                row.agent.status,
                AgentStatus::Working | AgentStatus::NeedsYou | AgentStatus::SettingUp
            ) {
                restored
                    .cut_off
                    .push((row.agent.id.clone(), row.agent.status));
                row.agent.status = AgentStatus::Idle;
            }
            let (entries, has_more) = self
                .db
                .load_entries(row.agent.id.clone(), None, CONVERSATION_TAIL)
                .await?;
            self.tails
                .insert(row.agent.id.clone(), ConversationTail { entries, has_more });
            self.agents.insert(row.agent.id.clone(), row);
        }
        // Queued starts survive a restart and run when a slot frees (decided 2026-11-06).
        self.queue = self.db.load_queue().await?.into();
        // Signals the coordinator missed while it could not be woken.
        self.coordinator_notes = self.db.load_notes().await?;
        restored.inbox = self.db.load_open_inbox().await?;
        let coordinator = AgentId(COORDINATOR.into());
        if !self.agents.contains_key(&coordinator) {
            let row = AgentRow {
                agent: new_agent(
                    coordinator.clone(),
                    COORDINATOR.into(),
                    AgentKind::Coordinator,
                    None,
                ),
                session_id: Some(uuid::Uuid::new_v4().to_string()),
                transcript_path: None,
                task_list_id: Some(self.task_list_id(None, COORDINATOR)),
                created_ms: now_ms(),
            };
            self.db.upsert_agent(row.clone()).await?;
            self.agents.insert(coordinator, row);
        }
        Ok(restored)
    }

    /// Status-bar warnings known at start.
    fn startup_warnings(&mut self) {
        let blockers: Vec<&str> = [
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "DISABLE_TELEMETRY",
            "DO_NOT_TRACK",
            "DISABLE_GROWTHBOOK",
        ]
        .into_iter()
        .filter(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
        .collect();
        if !blockers.is_empty() {
            self.warnings.push(Warning {
                id: WarningId("messaging".into()),
                text: format!(
                    "{} set: sessions may not be able to message each other",
                    blockers.join(", ")
                ),
            });
        }
        let missing: Vec<&str> = ["coordinator", "slice"]
            .into_iter()
            .filter(|r| !self.paths.role_prompt(r).exists())
            .collect();
        if !missing.is_empty() {
            self.warnings.push(Warning {
                id: WarningId("roles".into()),
                text: format!(
                    "role prompts missing ({}): run scripts/bootstrap.sh",
                    missing.join(", ")
                ),
            });
        }
        if self.project.registered().is_none() {
            self.warnings.push(Warning {
                id: WarningId(crate::registration::UNREGISTERED_WARNING.into()),
                text: "not a Dovetail project yet: :project add keeps it".into(),
            });
        }
    }

    /// Sends `Hello` (when `hello`) and a fresh snapshot.
    pub(crate) async fn send_snapshot(&mut self, hello: bool) {
        self.flush_deltas();
        let snapshot = match self.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(e) => {
                tracing::error!("loading state: {e}");
                Snapshot {
                    seq: self.seq,
                    now_ms: now_ms(),
                    project: Some(self.project.summary()),
                    ..Snapshot::default()
                }
            }
        };
        if hello {
            self.send(ServerMsg::Hello {
                protocol: PROTOCOL_VERSION,
            });
        }
        self.send(ServerMsg::Snapshot(snapshot));
    }

    async fn snapshot(&mut self) -> Result<Snapshot, DbError> {
        let projects = self.db.list_projects().await?;
        let mut snapshot = Snapshot {
            seq: self.seq,
            now_ms: now_ms(),
            project: Some(self.project.summary()),
            projects: projects.iter().map(Project::summary).collect(),
            tasks: self.db.load_tasks().await?,
            decisions: self.db.load_decisions().await?,
            agents: self.agents.values().map(|r| r.agent.clone()).collect(),
            inbox: self.inbox.values().map(|p| p.item.clone()).collect(),
            activity: self.activity.iter().cloned().collect(),
            messages: self.messages.iter().cloned().collect(),
            conversations: self.tails.clone(),
            warnings: self.warnings.clone(),
            ..Snapshot::default()
        };
        self.totals = totals_of(&snapshot);
        snapshot.totals = self.totals.clone();
        Ok(snapshot)
    }

    fn send(&mut self, msg: ServerMsg) {
        if self.out.send(msg).is_err() {
            self.ui_alive = false;
        }
    }

    /// Numbers and sends one event. Streamed text waiting to be sent goes first, so the UI sees
    /// events in the order they happened.
    pub(crate) fn emit(&mut self, event: Event) {
        if !self.deltas.is_empty() {
            self.flush_deltas();
        }
        self.send_event(event);
    }

    fn send_event(&mut self, event: Event) {
        self.seq += 1;
        let seq = self.seq;
        self.send(ServerMsg::Event { seq, event });
    }

    pub(crate) fn fail(&mut self, message: impl std::fmt::Display) {
        self.emit(Event::CommandFailed {
            message: message.to_string(),
        });
    }

    /// Runs a storage write in the background, logging a failure.
    pub(crate) fn persist<T: Send + 'static>(&self, what: &'static str, pending: Pending<T>) {
        tokio::spawn(async move {
            if let Err(e) = pending.await {
                tracing::warn!("storing {what}: {e}");
            }
        });
    }

    fn refresh_totals(&mut self) {
        let snapshot = Snapshot {
            agents: self.agents.values().map(|r| r.agent.clone()).collect(),
            inbox: self.inbox.values().map(|p| p.item.clone()).collect(),
            ..Snapshot::default()
        };
        let totals = totals_of(&snapshot);
        if totals != self.totals {
            self.totals = totals.clone();
            self.emit(Event::TotalsChanged(totals));
        }
    }

    // --- streaming text ---

    /// Queues streamed text; it goes out within [`DELTA_WINDOW`].
    pub(crate) fn queue_delta(&mut self, agent: &AgentId, entry: EntryId, text: &str) {
        if let Some(tail) = self.tails.get_mut(agent)
            && let Some(e) = tail.entries.iter_mut().find(|e| e.id == entry)
        {
            e.text.push_str(text);
        }
        self.deltas
            .entry((agent.clone(), entry))
            .or_default()
            .push_str(text);
        if self.flush_at.is_none() {
            self.flush_at = Some(tokio::time::Instant::now() + DELTA_WINDOW);
        }
    }

    fn flush_deltas(&mut self) {
        self.flush_at = None;
        for ((agent, entry), text) in std::mem::take(&mut self.deltas) {
            self.send_event(Event::AssistantDelta { agent, entry, text });
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
        let entry = ConversationEntry {
            id: EntryId(self.next_entry),
            at_ms: now_ms(),
            kind,
            text,
            attachments,
        };
        self.next_entry += 1;
        let tail = self.tails.entry(agent.clone()).or_default();
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
        self.emit(Event::ConversationAppended {
            agent: agent.clone(),
            entry,
        });
        id
    }

    /// Replaces an entry's text, tells the UI and stores it.
    pub(crate) fn set_entry_text(&mut self, agent: &AgentId, id: EntryId, text: String) {
        let Some(entry) = self
            .tails
            .get_mut(agent)
            .and_then(|t| t.entries.iter_mut().find(|e| e.id == id))
        else {
            return;
        };
        entry.text = text;
        let entry = entry.clone();
        // Streamed text for it is superseded.
        self.deltas.remove(&(agent.clone(), id));
        self.persist(
            "a conversation entry",
            self.db.upsert_entry(agent.clone(), entry.clone()),
        );
        self.emit(Event::ConversationAppended {
            agent: agent.clone(),
            entry,
        });
    }

    pub(crate) fn notice(&mut self, agent: &AgentId, text: impl Into<String>) {
        self.add_entry(agent, EntryKind::Notice, text.into());
    }

    // --- activity and messages ---

    pub(crate) fn activity(&mut self, kind: ActivityKind, agent: Option<&AgentId>, text: String) {
        let item = ActivityItem {
            id: self.next_activity,
            at_ms: now_ms(),
            agent: agent.cloned(),
            kind,
            text,
        };
        self.next_activity += 1;
        self.activity.push_back(item.clone());
        while self.activity.len() > ACTIVITY_TAIL {
            self.activity.pop_front();
        }
        self.persist("an activity item", self.db.add_activity(item.clone()));
        self.emit(Event::ActivityAdded(item));
    }

    pub(crate) fn message(
        &mut self,
        from: AgentId,
        to: AgentId,
        text: String,
        msg_id: Option<String>,
    ) {
        if let Some(id) = &msg_id
            && !self.seen_messages.insert(id.clone())
        {
            return;
        }
        let message = AgentMessage {
            id: self.next_message,
            at_ms: now_ms(),
            from,
            to,
            text,
        };
        self.next_message += 1;
        self.messages.push_back(message.clone());
        while self.messages.len() > MESSAGE_TAIL {
            self.messages.pop_front();
        }
        self.persist("a message", self.db.add_message(message.clone(), msg_id));
        self.emit(Event::MessageAdded(message));
    }

    // --- agents ---

    /// Tells the UI about an agent and stores it.
    pub(crate) fn save_agent(&mut self, id: &AgentId) {
        let Some(row) = self.agents.get(id).cloned() else {
            return;
        };
        self.persist("an agent", self.db.upsert_agent(row.clone()));
        self.emit(Event::AgentUpserted(row.agent));
    }

    /// Changes an agent and, if anything changed, tells the UI and stores it.
    pub(crate) fn update_agent(&mut self, id: &AgentId, change: impl FnOnce(&mut AgentRow)) {
        let Some(row) = self.agents.get_mut(id) else {
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
        if self.quitting.is_some() && !self.live.contains_key(id) {
            return;
        }
        let needs_you = self
            .inbox
            .values()
            .any(|p| &p.item.agent == id && p.waiter.holds_agent());
        let (setting_up, busy) = match self.live.get(id) {
            Some(live) => (live.setting_up, live.turn.busy()),
            None => (false, false),
        };
        let failed = self
            .agents
            .get(id)
            .is_some_and(|r| r.agent.status == AgentStatus::Error);
        let status = crate::session::state::StatusInputs {
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

    /// `dovetail-<project>-coordinator`, `dovetail-<project>-task-<id>` (A12), or
    /// `dovetail-<project>-slice-<name>` for a slice without a task.
    pub(crate) fn task_list_id(&self, task: Option<TaskId>, name: &str) -> String {
        let key = self.project.key();
        let id = match task {
            Some(t) => format!("dovetail-{key}-task-{}", t.0),
            None if name == COORDINATOR => format!("dovetail-{key}-coordinator"),
            None => format!("dovetail-{key}-slice-{name}"),
        };
        id.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }

    // --- tasks ---

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
        self.emit_tasks(tasks);
        Ok(id)
    }

    pub(crate) fn emit_tasks(&mut self, tasks: Vec<Task>) {
        for task in tasks {
            self.emit(Event::TaskUpserted(task));
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
        match self.db.update_task(id, patch).await {
            Ok(tasks) => self.emit_tasks(tasks),
            Err(e) => self.fail(e.to_string()),
        }
    }

    // --- commands ---

    async fn command(&mut self, command: Command) {
        match command {
            Command::Prompt {
                agent,
                text,
                attachments,
            } => self.prompt(&agent, text, attachments).await,
            Command::Interrupt { agent } => self.interrupt(&agent),
            Command::AnswerPermission { item, answer } => {
                self.answer_permission(item, answer).await
            }
            Command::AnswerQuestion { item, text } => self.answer_question(item, text).await,
            Command::DismissInbox { item } => self.dismiss(item).await,
            Command::CyclePermissionMode { agent } => self.cycle_mode(&agent),
            Command::NewSlice { task, name, brief } => {
                let request = SliceRequest {
                    task,
                    name,
                    brief,
                    by_coordinator: false,
                };
                if let Err(message) = self.request_slice(request).await {
                    self.fail(message);
                }
            }
            Command::CloseSlice { agent } => self.close_slice(&agent, false),
            Command::RemoveWorktree { agent, confirmed } => {
                self.remove_worktree(&agent, confirmed);
            }
            Command::CreateTask(new) => {
                if let Err(e) = self.create_task(new, false).await {
                    self.fail(e.to_string());
                }
            }
            Command::UpdateTask { id, patch } => self.update_task(id, patch).await,
            Command::ArchiveTask { id } => {
                let slices = self.task_slices(id);
                self.archive_task(id, &slices).await;
            }
            Command::ReorderTask { id, delta } => match self.db.reorder_task(id, delta).await {
                Ok(tasks) => self.emit_tasks(tasks),
                Err(e) => self.fail(e.to_string()),
            },
            Command::LogDecision(new) => {
                match self.db.log_decision(new, DecisionAuthor::User).await {
                    Ok(decision) => self.emit(Event::DecisionAdded(decision)),
                    Err(e) => self.fail(e.to_string()),
                }
            }
            Command::RegisterProject { path } => {
                let wanted = path.canonicalize().unwrap_or(path);
                if !wanted.as_os_str().is_empty() && wanted != self.project.repo_path() {
                    self.fail("only the open folder can be added as a project");
                    return;
                }
                if let Err(message) = self.register_project().await {
                    self.fail(message);
                }
            }
            Command::LoadConversation { agent, before } => {
                let before = before.or_else(|| {
                    self.tails
                        .get(&agent)
                        .and_then(|t| t.entries.first().map(|e| e.id))
                });
                match self
                    .db
                    .load_entries(agent.clone(), before, CONVERSATION_TAIL)
                    .await
                {
                    Ok((entries, has_more)) => self.emit(Event::ConversationPage {
                        agent,
                        entries,
                        has_more,
                    }),
                    Err(e) => self.fail(e.to_string()),
                }
            }
            Command::Resync => self.send_snapshot(false).await,
            Command::TakeOver { .. } => self.fail("take-over arrives in M14"),
            Command::OpenProject { .. } => {
                self.fail("switching projects: quit and open dovetail in the other folder");
            }
            // Quit is handled by the loop.
            Command::Quit { .. } => {}
        }
    }

    /// The open slices working on task `id`.
    pub(crate) fn task_slices(&self, id: TaskId) -> Vec<AgentId> {
        self.agents
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
        self.update_task(id, vec![TaskChange::Status(TaskStatus::Archived)])
            .await;
        let dropped: Vec<i64> = self
            .queue
            .iter()
            .filter(|q| q.task == Some(id))
            .map(|q| q.id)
            .collect();
        self.queue.retain(|q| q.task != Some(id));
        for queued in dropped {
            self.persist("the slice queue", self.db.dequeue_slice(queued));
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
    }
}

fn totals_of(state: &Snapshot) -> Totals {
    Totals {
        memory_mb: state.agents.iter().filter_map(|a| a.memory_mb).sum(),
        needs_you: state.inbox.len() as u32,
        cost_usd: (state.agents.iter().map(|a| a.cost_usd).sum::<f64>() * 100.0).round() / 100.0,
    }
}

async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

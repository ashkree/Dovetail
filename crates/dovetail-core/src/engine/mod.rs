//! The core engine: one thread that owns the event sequence, talks to the DB thread, the socket
//! and the sessions, and hands the UI the same [`CoreLink`] as `MockCore`.
//!
//! Everything that changes state goes through one loop ([`Core::run`]): UI commands, socket
//! requests (`dovetail add`, tool calls), driver events and finished background jobs (worktree
//! setup and removal). Handlers never block the loop: session methods only enqueue, git work runs
//! on blocking tasks and reports back as an [`Input`].
//!
//! Domain state lives in one struct per domain on [`Core`], and [`Core::command`] and
//! [`Core::tool`] are thin routers that delegate to each domain module.

pub(crate) mod shutdown;
pub(crate) mod startup;

pub use shutdown::CoreHandle;

pub(crate) use crate::agents::new_agent;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use dovetail_api::activity::{
    inbox::InboxItem,
    status::{DriverUsage, StatusEvent, Warning, WarningId},
};
use dovetail_api::agents::{
    agent::{AgentId, AgentKind, AgentStatus, TodoListId},
    conversation::{ConversationEvent, ConversationTail},
};
use dovetail_api::app::{AppCommand, AppEvent, Paste};
use dovetail_api::snapshot::CONVERSATION_TAIL;
use dovetail_api::tracking::{
    effort::EffortId,
    task::{NewTask, TaskId},
};
use dovetail_api::{Command, CoreLink, Event, PROTOCOL_VERSION, ServerMsg, Snapshot};
use dovetail_driver::DriverEvent;
use dovetail_tools::SocketResponseBody;
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

use crate::activity::inbox::Waiter;
use crate::activity::totals_of;
use crate::code::worktree::WorktreeError;
use crate::mcp::socket::{self, BoxFuture, HandlerError, SocketError, SocketHandler, ToolCall};
use crate::settings::{config::Config, paths::Paths};
use crate::storage::{AgentRow, Db, DbError, Pending, now_ms};
use crate::tracking::{projects::Resolution, registration::ProjectState};

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
    /// Available session drivers (`ClaudeDriver` and `AgyDriver` in the app, fakes in tests).
    pub drivers: crate::Drivers,
    /// The `dovetail` binary, for the worktree hook and the MCP bridge in launch settings.
    pub exe: PathBuf,
    /// Where Claude Code keeps native to-do lists; `None` for `tasks/` in each session's Claude
    /// home (its profile's folder, ADR 0009). Tests point it at a temp dir.
    pub tasks_dir: Option<PathBuf>,
    /// The editor command line for [`AppCommand::OpenEditor`]; `None` for `$EDITOR`, else `nvim`.
    /// Tests give a fake one.
    pub editor: Option<std::ffi::OsString>,
    /// The `claude` program run for take-over (M14); `None` for `claude` on `PATH`. Tests give
    /// a fake one.
    pub claude: Option<std::ffi::OsString>,
    /// How a stored take-over lock's native process is checked; `None` reads `/proc`. Tests
    /// give one that cannot tell.
    pub processes: Option<Arc<dyn crate::takeover::Processes + Send + Sync>>,
    /// Probe `systemd-run --user --scope` at start and run sessions under memory caps (M13).
    /// Tests leave it off: sessions run uncapped and no warning is raised.
    pub memory_caps: bool,
    /// Run `claude --version` at start and warn when it is missing or older than
    /// [`crate::engine::startup::MIN_CLAUDE`]. Tests leave it off: no `claude` process is started.
    pub check_claude: bool,
    /// Tests: take these scopes instead of probing (a temp dir stands in for the cgroup slice;
    /// nothing is launched under `systemd-run` by the fake driver).
    pub scopes: Option<crate::memory::Scopes>,
    /// What `:upgrade` needs (ADR 0014); `None` turns it off, as in tests that are not about it.
    pub upgrade: Option<UpgradeOptions>,
    /// The program [`AppCommand::OpenExternal`] runs with the URL; `None` for `$BROWSER`, else
    /// `xdg-open`. Tests give a fake one.
    pub opener: Option<std::ffi::OsString>,
    /// The GitHub CLI asked for a slice branch's pull request; `None` for `gh` on `PATH`. Tests
    /// give a fake one.
    pub gh: Option<PathBuf>,
}

/// What `:upgrade` runs its build with and installs over.
#[derive(Clone)]
pub struct UpgradeOptions {
    /// Runs git and cargo: `SystemExec` in the app, a fake in tests.
    pub exec: Arc<dyn crate::upgrade::Exec>,
    /// The installed binary, `~/.local/bin/dovetail`. `:upgrade` is refused unless `exe` is it.
    pub install: PathBuf,
}

impl std::fmt::Debug for UpgradeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpgradeOptions")
            .field("install", &self.install)
            .finish_non_exhaustive()
    }
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
    // Tangents is always there (ADR 0017): registered now if missing.
    let tangents = crate::tangents::open(&paths, &db, &project);
    if let Err(why) = &tangents {
        tracing::warn!("{why}");
    }
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
            let _ = forward.send(Input::Command(Command::App(AppCommand::Quit {
                end_sessions: true,
            })));
        })
        .map_err(CoreError::Thread)?;

    let restart = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let restart_flag = restart.clone();
    let switch_to: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let switch_slot = switch_to.clone();
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
                let mut core = Core::new(
                    db,
                    paths,
                    config,
                    options,
                    project,
                    socket_path,
                    message_tx,
                    input_tx,
                );
                core.app.upgrade.restart = restart_flag;
                core.switch_to = switch_slot;
                core.agents.tangents = tangents;
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
        CoreHandle {
            thread,
            restart,
            switch_to,
        },
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
    /// `dovetail upgrade` staged a build and hands it to this app over the socket.
    StagedUpgrade {
        reply: oneshot::Sender<Result<(), HandlerError>>,
    },
    /// The background build of `:upgrade` finished.
    UpgradeBuilt(Result<crate::upgrade::Ready, crate::upgrade::BuildError>),
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
        outcome: Result<PathBuf, crate::agents::SetupFailure>,
    },
    /// An `agy` slice's blocking `worktree::create` finished.
    AgyWorktreeCreated {
        agent: AgentId,
        brief: String,
        result: Result<crate::worktree::Created, WorktreeError>,
    },
    /// Driver usage limits loaded concurrently on the runtime (`Command::LoadUsage`).
    UsageLoaded(Vec<DriverUsage>),
    /// agy's model list, read in the background at start (`agy_models`).
    AgyModelsRefreshed(Result<Vec<dovetail_driver::ModelInfo>, dovetail_driver::ModelsError>),
    /// A slice's worktree removal finished.
    WorktreeRemoved {
        agent: AgentId,
        result: Result<(), WorktreeError>,
    },
    /// A finished subagent's worktree `name` was checked and, if unchanged, removed.
    SubagentWorktree {
        agent: AgentId,
        name: String,
        result: Result<crate::worktree::SubagentCleanup, WorktreeError>,
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
    /// A file in a watched native to-do list changed (M11).
    TodosChanged {
        list: TodoListId,
    },
    /// A slice worktree's state for the worktree view was gathered.
    WorktreeGathered(crate::worktree_info::Gathered),
    /// A clipboard read for an agent's chat bar finished.
    Pasted {
        agent: AgentId,
        paste: Paste,
    },
    /// Paths for an agent's `@` completion were listed.
    PathsListed {
        agent: AgentId,
        dir: Option<String>,
        paths: Vec<String>,
    },
    /// Copying text to the clipboard failed.
    CopyFailed(crate::clipboard::CopyError),
    /// The browser opener exited unsuccessfully; the message says how.
    OpenerFailed(String),
    /// The editor run in the UI's terminal exited; the message says why it failed, if it did.
    EditorExited(Option<String>),
    /// Native `claude` started for a take-over (M14).
    NativeStarted {
        agent: AgentId,
        pid: u32,
    },
    /// Native `claude` exited, or could not start.
    NativeExited {
        agent: AgentId,
        end: crate::takeover::NativeEnd,
    },
    /// A capped session's `memory.events` changed: its scope's `oom_kill` count (M13b).
    OomKills {
        cgroup: PathBuf,
        count: u64,
    },
    /// `:eod` has its handoff, or knows none is coming: write the day's journal file.
    CloseDay,
    /// A slice's preview process reported (ADR 0026). `run` tells which start it belongs to.
    Preview {
        agent: AgentId,
        run: u64,
        report: crate::preview::PreviewReport,
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

    fn staged_upgrade(&self) -> BoxFuture<Result<(), HandlerError>> {
        let (reply, answer) = oneshot::channel();
        let sent = self.inputs.send(Input::StagedUpgrade { reply });
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

/// The core's state. Infrastructure handles live directly on `Core`; domain state is grouped into
/// one struct per domain/infrastructure module.
pub(crate) struct Core {
    pub(crate) db: Db,
    pub(crate) paths: Paths,
    pub(crate) config: Config,
    pub(crate) options: CoreOptions,
    out: mpsc::Sender<ServerMsg>,
    pub(crate) inputs: tokio_mpsc::UnboundedSender<Input>,
    seq: u64,
    ui_alive: bool,
    pub(crate) quitting: Option<tokio::time::Instant>,
    pub(crate) killed: bool,
    /// When running sessions' memory is next read.
    pub(crate) next_sample: tokio::time::Instant,
    /// The project the user picked to open next (B24); the binary reads it once the core stopped.
    pub(crate) switch_to: Arc<std::sync::Mutex<Option<String>>>,

    // Infrastructure & domain state structs
    pub(crate) mcp: crate::mcp::McpState,
    pub(crate) sessions: crate::sessions::SessionState,
    pub(crate) tracking: crate::tracking::TrackingState,
    pub(crate) agents: crate::agents::AgentState,
    pub(crate) activity: crate::activity::ActivityState,
    pub(crate) visual: crate::visual::VisualState,
    pub(crate) research: crate::research::Research,
    pub(crate) code: crate::code::CodeState,
    pub(crate) app: crate::app::AppState,
}

/// How a new conversation entry gets its effort tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryTag {
    /// From the agent's current tag, by the entry's kind ([`Core::entry_tag`]).
    ByKind,
    /// This tag, or none: a slice's report carries its task's effort.
    Effort(Option<EffortId>),
}

/// The Claude home of a project: its recipe's profile out of `config.toml`; the default home for
/// a folder not registered yet (it has no recipe).
pub(crate) fn project_claude_home(
    paths: &Paths,
    config: &Config,
    project: &ProjectState,
) -> Result<crate::profile::ClaudeHome, crate::profile::ProfileError> {
    let Some(p) = project.registered() else {
        return config.claude_home(None);
    };
    let recipe = crate::recipe::Recipe::load(&paths.recipe(&p.id))
        .map_err(|e| crate::profile::ProfileError::Recipe(e.to_string()))?;
    // The agent row records the profile's folder: a recipe moving it behind Dovetail's back
    // would make that record wrong.
    if recipe.env.contains_key("CLAUDE_CONFIG_DIR") {
        return Err(crate::profile::ProfileError::EnvOverride);
    }
    // The project's account wins; while it is empty the recipe's `profile` is still read, so
    // nothing changes on the day of the migration (ADR 0021 point 1).
    config.claude_home(p.account.as_deref().or(recipe.profile.as_deref()))
}

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
        let claude = project_claude_home(&paths, &config, &project);
        let session_prefix = project.key().to_owned();
        Core {
            db,
            paths,
            config,
            options,
            out,
            inputs,
            seq: 0,
            ui_alive: true,
            quitting: None,
            killed: false,
            next_sample: tokio::time::Instant::now(),
            switch_to: Default::default(),
            mcp: crate::mcp::McpState::new(socket),
            sessions: crate::sessions::SessionState::new(session_prefix, claude),
            tracking: crate::tracking::TrackingState::new(project),
            agents: Default::default(),
            activity: Default::default(),
            visual: Default::default(),
            research: Default::default(),
            code: Default::default(),
            app: Default::default(),
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
        self.load_agy_models().await;
        self.send_snapshot(true).await;
        self.refresh_agy_models();
        self.sweep_attachments();
        self.rebuild_research_files().await;
        // After the first snapshot, so a slow user systemd never holds up the UI; before
        // anything can start a session (restore, the queue, commands), so every session gets
        // the probe's answer.
        if let Some(scopes) = self.options.scopes.clone() {
            self.sessions.scopes = scopes;
        } else if self.options.memory_caps {
            self.sessions.scopes = crate::memory::probe().await;
            if let crate::memory::Scopes::Unavailable(reason) = &self.sessions.scopes {
                let warning = Warning {
                    id: WarningId("memory-caps".into()),
                    text: format!("memory caps off, sessions run uncapped: {reason}"),
                };
                self.activity.warnings.push(warning.clone());
                self.emit(Event::Status(StatusEvent::WarningSet(warning)));
            }
        }
        if self.options.check_claude {
            let program = self.options.claude.clone().unwrap_or("claude".into());
            let found = startup::run_claude_version(&program).await;
            if let Some(warning) = startup::claude_version(found) {
                self.activity.warnings.push(warning.clone());
                self.emit(Event::Status(StatusEvent::WarningSet(warning)));
            }
        }
        self.reconcile(restored).await;
        self.resume_upgrade();
        self.start_queued().await;
        loop {
            if self.quitting.is_some() && self.sessions.live.is_empty() {
                break;
            }
            let flush_at = self.agents.flush_at;
            let quit_at = self.quitting;
            let todos_at = self.agents.todos.flush_at;
            // No idle ticks: memory is read only while a session it can be read for runs.
            let sample_at = self.sampled().then_some(self.next_sample);
            let oom_at = self.sessions.oom.check_at;
            let upgrade_at = self.app.upgrade.recheck_at();
            let input = tokio::select! {
                input = inputs.recv() => input,
                _ = sleep_until(flush_at), if flush_at.is_some() => {
                    self.flush_deltas();
                    continue;
                }
                _ = sleep_until(todos_at), if todos_at.is_some() => {
                    self.flush_todos();
                    continue;
                }
                _ = sleep_until(sample_at), if sample_at.is_some() => {
                    self.sample_memory();
                    continue;
                }
                // `y` on "Upgrade ready" waits for other projects' apps, which send no input here.
                _ = sleep_until(upgrade_at), if upgrade_at.is_some() => {
                    self.upgrade_check();
                    continue;
                }
                _ = sleep_until(oom_at), if oom_at.is_some() => {
                    self.check_oom();
                    self.refresh_totals();
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
                Input::Command(Command::App(AppCommand::Quit { .. })) => self.begin_quit(),
                Input::Command(command) if self.quitting.is_none() => self.command(command).await,
                Input::Command(_) => {}
                Input::AddTask { task, reply } => {
                    let result = if self.tracking.project.registered().is_none() {
                        Err(HandlerError::NotRegistered)
                    } else {
                        self.create_task(task, false)
                            .await
                            .map_err(HandlerError::from)
                    };
                    let _ = reply.send(result);
                }
                Input::StagedUpgrade { reply } => {
                    let _ = reply.send(self.take_staged_upgrade());
                }
                Input::UpgradeBuilt(result) => self.on_upgrade_built(result),
                Input::Tool { call, reply } => self.tool(call, reply).await,
                Input::Driver {
                    agent,
                    generation,
                    event,
                } => self.on_driver(agent, generation, event).await,
                // During quit nothing new starts, and no signal wakes the coordinator.
                Input::SetupFinished { .. } | Input::AgyWorktreeCreated { .. }
                    if self.quitting.is_some() => {}
                Input::SetupFinished {
                    agent,
                    generation,
                    outcome,
                } => self.on_setup(agent, generation, outcome).await,
                Input::AgyWorktreeCreated {
                    agent,
                    brief,
                    result,
                } => self.on_agy_worktree_created(agent, brief, result),
                Input::UsageLoaded(reports) => self.on_usage_loaded(reports),
                Input::AgyModelsRefreshed(result) => self.on_agy_models_refreshed(result),
                Input::WorktreeRemoved { agent, result } => {
                    self.on_worktree_removed(agent, result).await;
                }
                Input::SubagentWorktree {
                    agent,
                    name,
                    result,
                } => self.on_subagent_worktree(&agent, &name, result),
                Input::OrphanInspected { found, result } => self.on_orphan(found, result),
                Input::OrphanRemoved { name, result } => self.on_orphan_removed(name, result),
                Input::TodosChanged { list } => self.mark_todos(list),
                Input::WorktreeGathered(info) => self.on_worktree_gathered(info).await,
                Input::Pasted { agent, paste } => {
                    self.emit(Event::App(AppEvent::Pasted { agent, paste }));
                }
                Input::PathsListed { agent, dir, paths } => {
                    self.emit(Event::Conversation(ConversationEvent::PathsListed {
                        agent,
                        dir,
                        paths,
                    }));
                }
                Input::CopyFailed(e) => self.fail(e.to_string()),
                Input::OpenerFailed(message) => self.fail(message),
                Input::EditorExited(error) => {
                    self.app.editor_open = false;
                    self.emit(Event::App(AppEvent::TerminalReturned { error }));
                }
                Input::OomKills { cgroup, count } => self.on_oom_kills(cgroup, count),
                Input::CloseDay => self.write_journal().await,
                Input::Preview { agent, run, report } => self.on_preview(&agent, run, report),
                Input::NativeStarted { agent, pid } => self.native_started(&agent, pid),
                Input::NativeExited { agent, end } => self.native_exited(&agent, end),
            }
            if self.quitting.is_none() {
                self.start_queued().await;
            }
            self.refresh_totals();
            self.upgrade_check();
            if !self.ui_alive && self.quitting.is_none() {
                self.begin_quit();
            }
        }
        self.flush_deltas();
        std::mem::take(&mut self.mcp.extra_sockets)
    }

    /// Thin command router: delegates each feature's command to its owning domain.
    async fn command(&mut self, command: Command) {
        match command {
            Command::Project(cmd) => self.handle_project_command(cmd).await,
            Command::Task(cmd) => self.handle_task_command(cmd).await,
            Command::Effort(cmd) => self.handle_effort_command(cmd).await,
            Command::Decision(cmd) => self.handle_decision_command(cmd).await,
            Command::Agent(cmd) => self.handle_agent_command(cmd).await,
            Command::Conversation(cmd) => self.handle_conversation_command(cmd).await,
            Command::Tangent(cmd) => self.handle_tangent_command(cmd).await,
            Command::Journal(cmd) => self.handle_journal_command(cmd),
            Command::Model(cmd) => self.handle_model_command(cmd),
            Command::Mcp(cmd) => self.handle_mcp_command(cmd),
            Command::Inbox(cmd) => self.handle_inbox_command(cmd).await,
            Command::Status(cmd) => self.handle_status_command(cmd),
            Command::Canvas(cmd) => self.handle_canvas_command(cmd).await,
            Command::Design(cmd) => self.handle_design_command(cmd).await,
            Command::Map(cmd) => self.handle_map_command(cmd).await,
            Command::Research(cmd) => self.handle_research_command(cmd).await,
            Command::Library(cmd) => self.handle_library_command(cmd).await,
            Command::Worktree(cmd) => self.handle_worktree_command(cmd),
            Command::App(cmd) => self.handle_app_command(cmd).await,
        }
    }

    /// Loads agents, conversation tails and id counters, and makes sure a coordinator exists.
    async fn load(&mut self) -> Result<crate::restore::Restored, DbError> {
        let mut restored = crate::restore::Restored::default();
        let ids = self.db.max_ids().await?;
        self.agents.next_entry = ids.entry + 1;
        self.activity.next_inbox = ids.inbox + 1;
        self.activity.next_activity = ids.activity + 1;
        self.activity.next_message = ids.message + 1;
        for mut row in self.db.load_agents().await? {
            // A subagent ran inside its parent's process, which is gone: it stopped with it. Its
            // parent's cut-off turn is what gets reported.
            if !row.agent.kind.is_session() {
                if row.agent.status == AgentStatus::Working {
                    crate::session::subagent::mark_stopped(&mut row.agent, "stopped by a restart");
                    self.db.upsert_agent(row.clone()).await?;
                }
                self.agents.rows.insert(row.agent.id.clone(), row);
                continue;
            }
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
            self.agents
                .tails
                .insert(row.agent.id.clone(), ConversationTail { entries, has_more });
            self.agents.rows.insert(row.agent.id.clone(), row);
        }
        // Queued starts survive a restart and run when a slot frees (decided 2026-11-06).
        self.agents.queue = self.db.load_queue().await?.into();
        // Signals the coordinator missed while it could not be woken.
        self.agents.coordinator_notes = self.db.load_notes().await?;
        restored.inbox = self.db.load_open_inbox().await?;
        let coordinator = AgentId(COORDINATOR.into());
        if !self.agents.rows.contains_key(&coordinator) {
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
                tool_use_id: None,
                claude_home: None,
                takeover: None,
                model_choice: None,
                repo: None,
            };
            self.db.upsert_agent(row.clone()).await?;
            self.agents.rows.insert(coordinator, row);
        }
        // The native lists as they are on disk now, with their owners.
        self.load_todos().await?;
        self.load_research().await?;
        self.load_efforts().await?;
        Ok(restored)
    }

    /// Status-bar warnings known at start.
    fn startup_warnings(&mut self) {
        let checks = [
            startup::messaging(|v| std::env::var(v).ok()),
            startup::roles(
                &["coordinator", "slice"]
                    .into_iter()
                    .filter(|r| !self.paths.role_prompt(r).exists())
                    .collect::<Vec<_>>(),
            ),
            startup::side_chat_suites(self.config.suites.side_chat_override()),
            // A research project has no worktrees, so no recipe to miss.
            self.tracking
                .project
                .registered()
                .filter(|p| !p.is_research())
                .and_then(|p| startup::recipe(&p.id, self.paths.recipe(&p.id).exists())),
        ];
        self.activity.warnings.extend(checks.into_iter().flatten());
        if let Err(why) = &self.agents.tangents {
            self.activity.warnings.push(Warning {
                id: WarningId("tangents".into()),
                text: why.to_string(),
            });
        }
        if self.tracking.project.registered().is_none() {
            self.activity.warnings.push(Warning {
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
                    project: Some(self.tracking.project.summary(&self.paths)),
                    usage: self.current_usage(),
                    profile: self.profile_name(),
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
        let projects = self.project_summaries().await?;
        let tasks = self.db.load_tasks().await?;
        let canvas = self.db.load_canvas_state().await?;
        let research = self.research_summaries(&tasks).await?;
        let mut snapshot = Snapshot {
            seq: self.seq,
            now_ms: now_ms(),
            project: Some(self.tracking.project.summary(&self.paths)),
            projects,
            efforts: self.tracking.efforts.values().cloned().collect(),
            blocks: canvas.blocks,
            cards: canvas.cards,
            card_links: canvas.links,
            tasks,
            research,
            decisions: self.db.load_decisions().await?,
            maps: self.db.load_maps().await?,
            agents: self.agents.rows.values().map(|r| r.agent.clone()).collect(),
            inbox: self
                .activity
                .inbox
                .values()
                .map(|p| p.item.clone())
                .collect(),
            activity: self.activity.items.iter().cloned().collect(),
            messages: self.activity.messages.iter().cloned().collect(),
            conversations: self.agents.tails.clone(),
            warnings: self.activity.warnings.clone(),
            todos: self.agents.todos.items(),
            usage: self.current_usage(),
            profile: self.profile_name(),
            worktrees: self.code.worktrees.values().cloned().collect(),
            commands: self.agents.commands.clone(),
            journal: self.journal_state(),
            upgrade_building: self.app.upgrade.building.is_some(),
            tangents: self.tangent_summaries().await,
            models: self.model_options(),
            previews: self.visual.previews.list(),
            variants: self.db.load_variants().await?,
            effort_limit: self.config.max_efforts,
            agy_usage: self.activity.agy_usage.clone(),
            ..Snapshot::default()
        };
        self.activity.totals = totals_of(&snapshot);
        snapshot.totals = self.activity.totals.clone();
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
        if !self.agents.deltas.is_empty() {
            self.flush_deltas();
        }
        self.send_event(event);
    }

    pub(crate) fn send_event(&mut self, event: Event) {
        self.seq += 1;
        let seq = self.seq;
        self.send(ServerMsg::Event { seq, event });
    }

    pub(crate) fn fail(&mut self, message: impl std::fmt::Display) {
        self.emit(Event::App(AppEvent::CommandFailed {
            message: message.to_string(),
        }));
    }

    /// Runs a storage write in the background, logging a failure.
    pub(crate) fn persist<T: Send + 'static>(&self, what: &'static str, pending: Pending<T>) {
        tokio::spawn(async move {
            if let Err(e) = pending.await {
                tracing::warn!("storing {what}: {e}");
            }
        });
    }
}

async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

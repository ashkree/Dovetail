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
    TaskStatus, Totals, UsageLimits, Warning, WarningId,
};
use dovetail_driver::{DriverEvent, SessionDriver};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

use crate::config::Config;
use crate::db::{AgentRow, Db, DbError, Pending, now_ms};
use crate::inbox::Waiter;
use crate::paths::Paths;
use crate::projects::Resolution;
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

/// Most message ids remembered to drop duplicates.
const MAX_SEEN_MESSAGES: usize = 4096;

/// What the core needs from the binary.
#[derive(Clone)]
pub struct CoreOptions {
    /// Starts sessions: `ClaudeDriver` in the app, `FakeDriver` in tests.
    pub driver: Arc<dyn SessionDriver>,
    /// The `dovetail` binary, for the worktree hook and the MCP bridge in launch settings.
    pub exe: PathBuf,
    /// Where Claude Code keeps native to-do lists; `None` for `tasks/` in each session's Claude
    /// home (its profile's folder, ADR 0009). Tests point it at a temp dir.
    pub tasks_dir: Option<PathBuf>,
    /// The editor command line for [`Command::OpenEditor`]; `None` for `$EDITOR`, else `nvim`.
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
    /// [`crate::startup::MIN_CLAUDE`]. Tests leave it off: no `claude` process is started.
    pub check_claude: bool,
    /// Tests: take these scopes instead of probing (a temp dir stands in for the cgroup slice;
    /// nothing is launched under `systemd-run` by the fake driver).
    pub scopes: Option<crate::memory::Scopes>,
    /// What `:upgrade` needs (ADR 0014); `None` turns it off, as in tests that are not about it.
    pub upgrade: Option<UpgradeOptions>,
    /// The program [`Command::OpenExternal`] runs with the URL; `None` for `$BROWSER`, else
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
            let _ = forward.send(Input::Command(Command::Quit { end_sessions: true }));
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
                core.upgrade.restart = restart_flag;
                core.switch_to = switch_slot;
                core.tangents = tangents;
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
        outcome: Result<PathBuf, crate::error::SetupFailure>,
    },
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
        list: dovetail_api::TodoListId,
    },
    /// A slice worktree's state for the worktree view was gathered.
    WorktreeGathered(crate::worktree_info::Gathered),
    /// A clipboard read for an agent's chat bar finished.
    Pasted {
        agent: AgentId,
        paste: dovetail_api::Paste,
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

/// The core's state. Fields are shared with the sibling modules that hold the rest of `impl Core`.
pub(crate) struct Core {
    pub(crate) db: Db,
    pub(crate) paths: Paths,
    pub(crate) config: Config,
    pub(crate) options: CoreOptions,
    pub(crate) project: ProjectState,
    /// The prefix of every session name this process launches (ADR 0022): the project key at
    /// start. It stays when an unregistered project is added, so sessions launched as
    /// `new-<pid>.<name>` keep being found and stripped until the next start.
    pub(crate) session_prefix: String,
    /// The socket the bridges of this app's sessions use.
    pub(crate) socket: PathBuf,
    /// Sockets bound since start (after registration), removed on exit.
    pub(crate) extra_sockets: Vec<PathBuf>,
    out: mpsc::Sender<ServerMsg>,
    pub(crate) inputs: tokio_mpsc::UnboundedSender<Input>,
    seq: u64,
    ui_alive: bool,
    /// An editor runs in the UI's terminal ([`Command::OpenEditor`]).
    pub(crate) editor_open: bool,
    /// An editor the core accepted and is waiting for the UI to let go of the terminal for.
    pub(crate) editor_pending: Option<PathBuf>,
    /// A take-over this app runs: the terminal is lent to native `claude` (M14, `takeover`).
    pub(crate) takeover: Option<crate::takeover::Active>,
    /// Dovetail's notes for sessions taken over, sent once headless is back.
    pub(crate) held_for_takeover: HashMap<AgentId, Vec<String>>,
    pub(crate) agents: BTreeMap<AgentId, AgentRow>,
    pub(crate) live: HashMap<AgentId, Live>,
    pub(crate) generation: u64,
    pub(crate) tails: BTreeMap<AgentId, ConversationTail>,
    pub(crate) inbox: BTreeMap<InboxId, InboxEntry>,
    pub(crate) activity: VecDeque<ActivityItem>,
    pub(crate) messages: VecDeque<AgentMessage>,
    pub(crate) warnings: Vec<Warning>,
    totals: Totals,
    /// Usage limits per Claude home (limits are per account, task #16 and ADR 0009), as last
    /// reported by a session running under it.
    pub(crate) usage: BTreeMap<PathBuf, UsageLimits>,
    /// Context windows by model id, as `result`s report them (decision #22, `context`).
    pub(crate) context_windows: BTreeMap<String, u64>,
    pub(crate) next_entry: u64,
    pub(crate) next_inbox: u64,
    next_activity: u64,
    next_message: u64,
    /// Streamed text not yet sent, coalesced to at most ~30 updates a second.
    deltas: BTreeMap<(AgentId, EntryId), String>,
    flush_at: Option<tokio::time::Instant>,
    /// Slice starts over the `max_slices` cap, oldest first (ADR 0004).
    pub(crate) queue: VecDeque<crate::db::QueuedSlice>,
    /// `project_archive_slice` / `tasks_archive` calls waiting for their slices' outcomes (task #9).
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
    /// Native to-do lists: items, owners, the watcher (M11, `todos`).
    pub(crate) todos: crate::todos::Todos,
    /// The Claude home (profile) this project's sessions run under, or why it cannot be used.
    pub(crate) claude: Result<crate::profile::ClaudeHome, crate::profile::ProfileError>,
    /// The worktree view's data, per slice (M12b, `worktree_info`).
    pub(crate) worktrees: BTreeMap<AgentId, dovetail_api::WorktreeInfo>,
    /// Each session's slash commands and skills, as it last listed them; kept after the process
    /// stops so `/` completion still works for a dormant slice.
    pub(crate) commands: BTreeMap<AgentId, Vec<dovetail_api::SlashCommand>>,
    /// Whether sessions run under capped scopes (M13, `memory`).
    pub(crate) scopes: crate::memory::Scopes,
    /// When running sessions' memory is next read.
    next_sample: tokio::time::Instant,
    /// OOM kills: the `memory.events` watcher and automatic restarts (M13b, `oom`).
    pub(crate) oom: crate::oom::Oom,
    /// An `:eod` waiting for the coordinator's handoff (ADR 0012, `journal`).
    pub(crate) closing: Option<crate::journal::Closing>,
    /// Yesterday's coordinator session (id, transcript), asked back with `y`: it resumes once
    /// the fresh session has exited (ADR 0012).
    pub(crate) revert_coordinator: Option<(Option<String>, Option<PathBuf>)>,
    /// A `:fresh` whose handoff is written: the new coordinator starts once the old exits.
    pub(crate) pending_fresh: Option<crate::journal::PendingFresh>,
    /// `:upgrade`: the background build, the "Upgrade ready" item, the wait for idle (ADR 0014).
    pub(crate) upgrade: crate::upgrade::flow::UpgradeState,
    /// The project the user picked to open next (B24); the binary reads it once the core stopped.
    pub(crate) switch_to: Arc<std::sync::Mutex<Option<String>>>,
    /// Research capture: the open round and the subagents being watched (R1, `research`).
    pub(crate) research: crate::research::Research,
    /// Tangents' database (ADR 0017, `tangents`), or why tangents are off.
    pub(crate) tangents: Result<Db, crate::tangents::TangentError>,
    /// The models sessions last listed (`list_models`), for the selector's effort levels.
    pub(crate) model_catalog: Vec<dovetail_driver::ModelInfo>,
    /// Every effort of the open project with its criteria (`efforts`).
    pub(crate) efforts: BTreeMap<dovetail_api::EffortId, dovetail_api::Effort>,
    /// The effort tag each agent's new entries get: that of the user's last message to it.
    pub(crate) effort_tags: HashMap<AgentId, dovetail_api::EffortId>,
    /// Archived slices and the task each worked on, kept so a pull-request lookup started as
    /// the slice was archived can still move its task (ADR 0024).
    pub(crate) closed_slices: HashMap<AgentId, TaskId>,
}

/// How a new conversation entry gets its effort tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryTag {
    /// From the agent's current tag, by the entry's kind ([`Core::entry_tag`]).
    ByKind,
    /// This tag, or none: a slice's report carries its task's effort.
    Effort(Option<dovetail_api::EffortId>),
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
    config.claude_home(recipe.profile.as_deref())
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
        let claude = project_claude_home(&paths, &config, &project);
        let session_prefix = project.key().to_owned();
        Core {
            db,
            paths,
            config,
            options,
            project,
            session_prefix,
            socket,
            extra_sockets: Vec::new(),
            out,
            inputs,
            seq: 0,
            ui_alive: true,
            editor_open: false,
            editor_pending: None,
            takeover: None,
            held_for_takeover: HashMap::new(),
            agents: BTreeMap::new(),
            live: HashMap::new(),
            generation: 0,
            tails: BTreeMap::new(),
            inbox: BTreeMap::new(),
            activity: VecDeque::new(),
            messages: VecDeque::new(),
            warnings: Vec::new(),
            totals: Totals::default(),
            usage: BTreeMap::new(),
            context_windows: BTreeMap::new(),
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
            todos: Default::default(),
            claude,
            worktrees: BTreeMap::new(),
            commands: BTreeMap::new(),
            scopes: crate::memory::Scopes::Off,
            next_sample: tokio::time::Instant::now(),
            oom: Default::default(),
            closing: None,
            revert_coordinator: None,
            pending_fresh: None,
            upgrade: Default::default(),
            switch_to: Default::default(),
            research: Default::default(),
            tangents: Err(crate::tangents::TangentError::Off("not opened".into())),
            model_catalog: Vec::new(),
            efforts: BTreeMap::new(),
            effort_tags: HashMap::new(),
            closed_slices: HashMap::new(),
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
        self.sweep_attachments();
        self.rebuild_research_files().await;
        // After the first snapshot, so a slow user systemd never holds up the UI; before
        // anything can start a session (restore, the queue, commands), so every session gets
        // the probe's answer.
        if let Some(scopes) = self.options.scopes.clone() {
            self.scopes = scopes;
        } else if self.options.memory_caps {
            self.scopes = crate::memory::probe().await;
            if let crate::memory::Scopes::Unavailable(reason) = &self.scopes {
                let warning = Warning {
                    id: WarningId("memory-caps".into()),
                    text: format!("memory caps off, sessions run uncapped: {reason}"),
                };
                self.warnings.push(warning.clone());
                self.emit(Event::WarningSet(warning));
            }
        }
        if self.options.check_claude {
            let program = self.options.claude.clone().unwrap_or("claude".into());
            let found = crate::startup::run_claude_version(&program).await;
            if let Some(warning) = crate::startup::claude_version(found) {
                self.warnings.push(warning.clone());
                self.emit(Event::WarningSet(warning));
            }
        }
        self.reconcile(restored).await;
        self.resume_upgrade();
        self.start_queued().await;
        loop {
            if self.quitting.is_some() && self.live.is_empty() {
                break;
            }
            let flush_at = self.flush_at;
            let quit_at = self.quitting;
            let todos_at = self.todos.flush_at;
            // No idle ticks: memory is read only while a session it can be read for runs.
            let sample_at = self.sampled().then_some(self.next_sample);
            let oom_at = self.oom.check_at;
            let upgrade_at = self.upgrade.recheck_at();
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
                Input::SetupFinished { .. } if self.quitting.is_some() => {}
                Input::SetupFinished {
                    agent,
                    generation,
                    outcome,
                } => self.on_setup(agent, generation, outcome).await,
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
                    self.emit(Event::ClipboardPasted { agent, paste });
                }
                Input::PathsListed { agent, dir, paths } => {
                    self.emit(Event::PathsListed { agent, dir, paths });
                }
                Input::CopyFailed(e) => self.fail(e.to_string()),
                Input::OpenerFailed(message) => self.fail(message),
                Input::EditorExited(error) => {
                    self.editor_open = false;
                    self.emit(Event::TerminalReturned { error });
                }
                Input::OomKills { cgroup, count } => self.on_oom_kills(cgroup, count),
                Input::CloseDay => self.write_journal().await,
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
            // A subagent ran inside its parent's process, which is gone: it stopped with it. Its
            // parent's cut-off turn is what gets reported.
            if !row.agent.kind.is_session() {
                if row.agent.status == AgentStatus::Working {
                    crate::session::subagent::mark_stopped(&mut row.agent, "stopped by a restart");
                    self.db.upsert_agent(row.clone()).await?;
                }
                self.agents.insert(row.agent.id.clone(), row);
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
                tool_use_id: None,
                claude_home: None,
                takeover: None,
                model_choice: None,
            };
            self.db.upsert_agent(row.clone()).await?;
            self.agents.insert(coordinator, row);
        }
        // The native lists as they are on disk now, with their owners.
        self.load_todos().await?;
        self.load_research().await?;
        self.load_efforts().await?;
        Ok(restored)
    }

    /// Removes saved paste attachments older than a week that no live agent in any project
    /// names, on a blocking thread so the first frame never waits for it.
    fn sweep_attachments(&self) {
        let paths = self.paths.clone();
        tokio::task::spawn_blocking(move || {
            let Some(keep) = crate::db::live_attachments(&paths) else {
                tracing::warn!("attachment sweep skipped: a project database could not be read");
                return;
            };
            let dir = paths.attachments();
            let n = crate::attachments::sweep(
                &dir,
                std::time::SystemTime::now(),
                crate::attachments::MAX_AGE,
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
                    let _ = tokio::task::spawn_blocking(move || {
                        crate::attachments::remove(&dir, &files)
                    })
                    .await;
                }
                Err(e) => tracing::warn!("listing an archived agent's attachments: {e}"),
            }
        });
    }

    /// Status-bar warnings known at start.
    fn startup_warnings(&mut self) {
        let checks = [
            crate::startup::messaging(|v| std::env::var(v).ok()),
            crate::startup::roles(
                &["coordinator", "slice"]
                    .into_iter()
                    .filter(|r| !self.paths.role_prompt(r).exists())
                    .collect::<Vec<_>>(),
            ),
            crate::startup::side_chat_suites(self.config.suites.side_chat_override()),
            // A research project has no worktrees, so no recipe to miss.
            self.project
                .registered()
                .filter(|p| !p.is_research())
                .and_then(|p| crate::startup::recipe(&p.id, self.paths.recipe(&p.id).exists())),
        ];
        self.warnings.extend(checks.into_iter().flatten());
        if let Err(why) = &self.tangents {
            self.warnings.push(Warning {
                id: WarningId("tangents".into()),
                text: why.to_string(),
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
                    project: Some(self.project.summary(&self.paths)),
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
        let projects = self.db.list_projects().await?;
        let tasks = self.db.load_tasks().await?;
        let research = self.research_summaries(&tasks).await?;
        let mut snapshot = Snapshot {
            seq: self.seq,
            now_ms: now_ms(),
            project: Some(self.project.summary(&self.paths)),
            projects: projects.iter().map(|p| p.summary(&self.paths)).collect(),
            efforts: self.efforts.values().cloned().collect(),
            tasks,
            research,
            decisions: self.db.load_decisions().await?,
            maps: self.db.load_maps().await?,
            agents: self.agents.values().map(|r| r.agent.clone()).collect(),
            inbox: self.inbox.values().map(|p| p.item.clone()).collect(),
            activity: self.activity.iter().cloned().collect(),
            messages: self.messages.iter().cloned().collect(),
            conversations: self.tails.clone(),
            warnings: self.warnings.clone(),
            todos: self.todos.items(),
            usage: self.current_usage(),
            profile: self.profile_name(),
            worktrees: self.worktrees.values().cloned().collect(),
            commands: self.commands.clone(),
            journal: self.journal_state(),
            upgrade_building: self.upgrade.building.is_some(),
            tangents: self.tangent_summaries().await,
            models: self.model_options(),
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

    /// Memory is read from `/proc` (decision #47): the probe found caps unavailable, so
    /// sessions have no scope to read.
    fn proc_fallback(&self) -> bool {
        matches!(self.scopes, crate::memory::Scopes::Unavailable(_))
    }

    /// Whether a running session's memory can be read: from its scope, or from `/proc`.
    fn sampled(&self) -> bool {
        let fallback = self.proc_fallback();
        self.live
            .values()
            .any(|l| l.cgroup.is_some() || (fallback && l.pid.is_some()))
    }

    /// Reads each session's memory, from its scope or (uncapped) from `/proc`, and tells the UI
    /// about figures that changed as shown (see [`crate::memory::shown_mb`]). Capped scopes'
    /// OOM counts are read too, in case the watcher missed a change.
    fn sample_memory(&mut self) {
        use crate::memory;
        self.next_sample = tokio::time::Instant::now() + memory::SAMPLE_INTERVAL;
        let fallback = self.proc_fallback();
        let parents = if fallback && self.live.values().any(|l| l.pid.is_some()) {
            memory::read_parents()
        } else {
            Vec::new()
        };
        let mut readings = Vec::new();
        let mut ooms = Vec::new();
        for (id, live) in &self.live {
            if let Some(cgroup) = live.cgroup.as_deref() {
                readings.push((id.clone(), memory::read_current(cgroup)));
                if let Some(count) = memory::read_oom_kills(cgroup) {
                    ooms.push((id.clone(), count));
                }
            } else if let Some(pid) = live.pid.filter(|_| fallback) {
                readings.push((id.clone(), memory::read_tree_rss(pid, &parents)));
            }
        }
        for (id, bytes) in readings {
            self.set_memory(&id, bytes.map(memory::shown_mb));
            self.watch_oom(&id);
        }
        for (id, count) in ooms {
            self.oom_count(&id, count);
        }
        self.refresh_totals();
    }

    /// Sets an agent's shown memory. Memory is not stored, so only the UI hears of it.
    pub(crate) fn set_memory(&mut self, id: &AgentId, mb: Option<u32>) {
        let Some(row) = self.agents.get_mut(id) else {
            return;
        };
        if row.agent.memory_mb == mb {
            return;
        }
        row.agent.memory_mb = mb;
        let agent = row.agent.clone();
        self.emit(Event::AgentUpserted(agent));
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
            id: EntryId(self.next_entry),
            at_ms: now_ms(),
            kind,
            text,
            attachments,
            effort,
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
        // Old ids do not come back, so forgetting them all now and then is safe.
        if self.seen_messages.len() > MAX_SEEN_MESSAGES {
            self.seen_messages.clear();
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
        // A subagent's status comes from its `task_*` events (see `session::subagent`).
        if self
            .agents
            .get(id)
            .is_some_and(|r| !r.agent.kind.is_session())
        {
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

    /// The name a session is given in Claude Code (`--name`), which its own messaging finds
    /// sessions by across the whole machine: `<project>.<agent name>`, so two open projects
    /// never share one (ADR 0022). The prefix is fixed for this process ([`Core::session_prefix`]).
    pub(crate) fn session_name(&self, agent_name: &str) -> String {
        format!("{}.{agent_name}", self.session_prefix)
    }

    /// An agent's own name, from a session name as the stream gives it back: without this
    /// project's prefix. A name with no prefix (a session started before ADR 0022, or a
    /// subagent) is returned as it is.
    pub(crate) fn bare_name<'a>(&self, session_name: &'a str) -> &'a str {
        session_name
            .strip_prefix(self.session_prefix.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(session_name)
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
        if let Some(text) = dovetail_api::describe_task_patch(id, title, patch) {
            self.activity(ActivityKind::Task, None, text);
        }
    }

    // --- commands ---

    async fn command(&mut self, command: Command) {
        match command {
            Command::Prompt {
                agent,
                text,
                attachments,
                effort,
            } => self.prompt(&agent, text, attachments, effort).await,
            Command::PasteClipboard { agent } => self.paste_clipboard(agent),
            Command::PasteText { agent, text } => self.paste_text(agent, text),
            Command::ListPaths { agent, dir } => self.list_paths(agent, dir),
            Command::CopyToClipboard { text } => self.copy_to_clipboard(text),
            Command::OpenEditor { path } => self.open_editor(path),
            Command::TerminalReleased => self.run_editor(),
            Command::Interrupt { agent } => self.interrupt(&agent),
            Command::AnswerPermission { item, answer } => {
                self.answer_permission(item, answer).await
            }
            Command::AnswerQuestion { item, text } => self.answer_question(item, text).await,
            Command::DismissInbox { item } => self.dismiss(item).await,
            Command::CyclePermissionMode { agent } => self.cycle_mode(&agent),
            Command::SetModel {
                agent,
                model,
                effort,
            } => self.switch_model(&agent, model, effort),
            Command::NewSideChat { question } => self.start_side_chat(question),
            Command::NewTangent { question } => self.start_tangent(question).await,
            Command::AttachTangent { tangent, task } => self.attach_tangent(tangent, task).await,
            Command::FileTangent { tangent, project } => {
                self.file_tangent(tangent, project).await;
            }
            Command::StartFromTangent { tangent, project } => {
                self.start_from_tangent(tangent, project).await;
            }
            Command::DropTangent { tangent } => self.drop_tangent(tangent).await,
            Command::LoadTangent { tangent, what } => self.load_tangent(tangent, what).await,
            Command::EndOfDay => self.end_of_day(),
            Command::FreshStart => self.fresh_start(),
            Command::Upgrade { local } => self.start_upgrade(local),
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
            Command::CreateEffort(new) => self.create_effort(new).await,
            Command::RenameEffort { id, name } => self.rename_effort(id, name).await,
            Command::UpdateEffort { id, patch } => self.update_effort(id, patch).await,
            Command::SetWhereWasI { id, where_was_i } => {
                self.set_where_was_i(id, where_was_i).await;
            }
            Command::AddCriterion { effort, text } => self.add_criterion(effort, text).await,
            Command::ClaimCriterion { id, evidence } => self.claim_criterion(id, evidence).await,
            Command::ApproveCriterion { id } => self.approve_criterion(id).await,
            Command::ReturnCriterion { id, reason } => self.return_criterion(id, reason).await,
            Command::MoveTask {
                id,
                effort,
                criterion,
            } => self.move_task(id, effort, criterion).await,
            Command::LogDecision(new) => {
                match self.db.log_decision(new, DecisionAuthor::User).await {
                    Ok(decision) => self.emit(Event::DecisionAdded(decision)),
                    Err(e) => self.fail(e.to_string()),
                }
            }
            Command::RegisterProject { path } => {
                let wanted = path.canonicalize().unwrap_or(path);
                if !wanted.as_os_str().is_empty() && wanted != self.project.folder(&self.paths) {
                    self.fail("only the open folder can be added as a project");
                    return;
                }
                match self.register_project().await {
                    Ok((project, Some(path))) => self.emit(Event::RecipeStarted {
                        project: dovetail_api::ProjectId(project.id),
                        name: project.name,
                        path,
                        profiles: self.config.profiles.keys().cloned().collect(),
                    }),
                    Ok((_, None)) => {}
                    Err(message) => self.fail(message),
                }
            }
            Command::SetRecipeProfile { project, profile } => {
                self.set_recipe_profile(project, profile)
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
            Command::LoadMap { id, version } => match self.db.load_map(id.clone(), version).await {
                Ok(Some(map)) => self.emit(Event::MapLoaded(map)),
                Ok(None) => self.fail(match version {
                    Some(v) => format!("map {} has no version {v}", id.0),
                    None => format!("there is no map {}", id.0),
                }),
                Err(e) => self.fail(e.to_string()),
            },
            Command::StartRound { task, brief } => self.start_round(task, brief).await,
            Command::CloseRound { task } => self.close_round(task).await,
            Command::DropRound { task, round } => self.drop_round(task, round).await,
            Command::SetResearchKind { task, kind } => self.set_research_kind(task, kind).await,
            Command::FileUnfiled { item, task } => self.file_unfiled(item, task).await,
            Command::LoadResearch { task, what } => self.load_research_content(task, what).await,
            Command::OpenExternal { url } => self.open_external(url),
            Command::StartAsk { task } => self.start_ask(task).await,
            Command::ListDocs => self.list_docs(),
            Command::LoadDoc { path } => self.load_doc(path),
            Command::BrowseProject { id } => self.browse_project(id).await,
            Command::LoadBrowsed { project, what } => self.load_browsed(project, what).await,
            Command::RefreshWorktrees => self.refresh_worktrees(),
            Command::Resync => self.send_snapshot(false).await,
            Command::TakeOver { agent } => self.take_over(agent).await,
            Command::OpenProject { id } => self.open_other_project(id).await,
            Command::NewResearchProject { name } => self.new_research_project(name).await,
            // Quit is handled by the loop.
            Command::Quit { .. } => {}
        }
    }

    /// Reads the clipboard on a blocking thread (the tools may take seconds) and answers with
    /// [`Event::ClipboardPasted`] when done. Images are saved under `attachments/`.
    fn paste_clipboard(&mut self, agent: AgentId) {
        let dir = self.paths.attachments();
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let paste = crate::clipboard::paste(&dir);
            let _ = inputs.send(Input::Pasted { agent, paste });
        });
    }

    /// Pasted text: a path to an existing image becomes an attachment (the file is read and
    /// copied on a blocking thread), anything else comes back as the text.
    fn paste_text(&mut self, agent: AgentId, text: String) {
        let dir = self.paths.attachments();
        let inputs = self.inputs.clone();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        tokio::task::spawn_blocking(move || {
            let paste = crate::attachments::paste_text(text, &dir, home.as_deref());
            let _ = inputs.send(Input::Pasted { agent, paste });
        });
    }

    /// Lists paths for `@` completion on a blocking thread (git can take a moment on a big
    /// tree), relative to the agent's folder: its worktree, else the main checkout.
    fn list_paths(&mut self, agent: AgentId, dir: Option<String>) {
        let root = self
            .agents
            .get(&agent)
            .and_then(|r| r.agent.worktree.as_ref())
            .map(|w| w.path.clone())
            .unwrap_or_else(|| self.project.folder(&self.paths));
        let inputs = self.inputs.clone();
        // A research project has no repo: no git, just the folder listing as it is typed.
        let git = !self.project.is_research();
        tokio::task::spawn_blocking(move || {
            let paths = match dir.as_deref() {
                None if git => crate::completion::tree(&root),
                None => Vec::new(),
                Some(d) => crate::completion::dir(&root, d),
            };
            let _ = inputs.send(Input::PathsListed { agent, dir, paths });
        });
    }

    /// Opens a web page in the browser (ADR 0018). The opener is waited for on a blocking
    /// thread, so it is reaped; only a failure is reported.
    fn open_external(&mut self, url: String) {
        let opener = crate::external::resolve(self.options.opener.as_deref());
        let mut child = match crate::external::spawn(&opener, &url) {
            Ok(child) => child,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let inputs = self.inputs.clone();
        let shown = opener.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || match child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                let _ = inputs.send(Input::OpenerFailed(format!(
                    "{shown} could not open {url} ({status})"
                )));
            }
            Err(e) => {
                let _ = inputs.send(Input::OpenerFailed(format!("{shown}: {e}")));
            }
        });
    }

    /// The repo's `docs/` folder for reading mode's Docs · ADRs; `None` for a research project,
    /// which has no repo.
    fn docs_repo(&self) -> Option<PathBuf> {
        (!self.project.is_research()).then(|| self.project.folder(&self.paths))
    }

    /// `Command::ListDocs`: small folders, read in place.
    fn list_docs(&mut self) {
        let docs = self
            .docs_repo()
            .map(|repo| crate::docs::list(&repo))
            .unwrap_or_default();
        self.emit(Event::DocsListed { docs });
    }

    /// `Command::LoadDoc`.
    fn load_doc(&mut self, path: PathBuf) {
        let Some(repo) = self.docs_repo() else {
            self.fail("a research project has no repo docs");
            return;
        };
        match crate::docs::load(&repo, &path) {
            Ok(document) => self.emit(Event::DocLoaded { path, document }),
            Err(e) => self.fail(e),
        }
    }

    /// Copies text to the clipboard on a blocking thread; only a failure is reported.
    fn copy_to_clipboard(&mut self, text: String) {
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::clipboard::copy(&text) {
                let _ = inputs.send(Input::CopyFailed(e));
            }
        });
    }

    /// Accepts [`Command::OpenEditor`]: checks the path, then asks the UI to let go of the
    /// terminal ([`Event::LendTerminal`]). A missing path or a terminal already lent out is
    /// answered with [`Event::CommandFailed`] and the UI keeps the terminal, so nothing
    /// flickers.
    fn open_editor(&mut self, path: PathBuf) {
        if self.editor_open || self.editor_pending.is_some() || self.takeover.is_some() {
            self.fail("the terminal is already lent out");
            return;
        }
        if !path.exists() {
            self.fail(crate::editor::EditorError::Missing(
                path.display().to_string(),
            ));
            return;
        }
        self.editor_pending = Some(path);
        self.emit(Event::LendTerminal);
    }

    /// Runs the accepted editor in the terminal the UI has let go of, and answers
    /// [`Event::TerminalReturned`] when it exits.
    fn run_editor(&mut self) {
        let Some(path) = self.editor_pending.take() else {
            return;
        };
        self.editor_open = true;
        let editor = crate::editor::resolve(self.options.editor.as_deref());
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let error = crate::editor::run(&editor, &path).err().map(|e| {
                tracing::info!(%e, "editor");
                e.to_string()
            });
            let _ = inputs.send(Input::EditorExited(error));
        });
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
        // The user was asked about an open or queued research round (the UI's confirmation); the
        // round closes with its findings kept, and queued ones are dropped.
        self.release_research_for_archive(id).await;
        self.update_task(id, vec![TaskChange::Status(TaskStatus::Archived)])
            .await;
        let dropped: Vec<i64> = self
            .queue
            .iter()
            .filter(|q| q.task == Some(id))
            .map(|q| q.id)
            .collect();
        self.queue.retain(|q| q.task != Some(id));
        // Digest §4: the lists Dovetail created go with their task.
        self.delete_task_todos(id);
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
        model: None,
        effort: None,
        subagent_tokens: 0,
        tool: None,
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

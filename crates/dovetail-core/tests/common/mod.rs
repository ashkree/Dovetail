//! The harness shared by the session tests: a core on a fake driver and its helpers.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};

use std::os::unix::net::UnixStream;

use std::path::{Path, PathBuf};

use std::sync::Arc;

use std::time::{Duration, Instant};

use dovetail_api::{
    Event,
    app::{AppCommand, AppEvent},
    conversation::ConversationCommand,
    mcp::McpEvent,
    research::{ResearchContent, ResearchEvent, ResearchSummary, ResearchWhat},
    status::StatusEvent,
    task::TaskCommand,
};
use dovetail_tools::project::ProjectTool;

use dovetail_tools::{
    SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody, Tool, project::StartSlice,
};

use dovetail_api::{
    Command, CoreLink, ServerMsg, Snapshot,
    agent::{AgentId, AgentKind, PermissionMode},
    apply,
    conversation::EntryKind,
    inbox::InboxKind,
    task::{NewTask, TaskId},
};

use dovetail_core::config::Config;

use dovetail_core::memory::Scopes;

use dovetail_core::paths::Paths;

use dovetail_core::projects::{Candidate, Project, Resolution};

use dovetail_core::{CoreHandle, CoreOptions, Drivers};

use dovetail_driver::{FakeDriver, ReplayOptions, Sent};

use dovetail_api::{agent::AgentCommand, effort::EffortCommand, inbox::InboxId, task::TaskStatus};
use dovetail_tools::{
    coordination::CoordinationTool,
    project::ArchiveSlice,
    report::{AskUser, ReportTool},
};
use tempfile::TempDir;

thread_local! {
    /// Taken by the next `Harness::start` on this test's thread: what `:upgrade` runs with.
    pub static UPGRADE: std::cell::RefCell<Option<dovetail_core::service::UpgradeOptions>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Taken by the next `Harness::start` on this test's thread: the project it opens, when a
    /// test registered its own (ADR 0021: several repos).
    pub static PROJECT: std::cell::RefCell<Option<Project>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Taken by the next `Harness::start` on this test's thread: the fake `gh` it looks pull
    /// requests up with.
    pub static GH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Taken by the next `Harness::start*` on this test's thread: the fake browser opener for
    /// `AppCommand::OpenExternal`.
    pub static OPENER: std::cell::RefCell<Option<std::ffi::OsString>> =
        const { std::cell::RefCell::new(None) };
}

/// Standard test paths under a temporary directory (`runtime` is replaced per run by `Harness`).
pub fn test_paths(dir: &Path) -> Paths {
    Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("run"),
    }
}

pub fn fixture(name: &str) -> FakeDriver {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures")
        .join(format!("{name}.ndjson"));
    FakeDriver::from_file(path).expect("fixture")
}

pub fn agy_fixture(name: &str) -> FakeDriver {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures/agy")
        .join(format!("{name}.ndjson"));
    FakeDriver::from_file(path).expect("fixture").for_agy()
}

pub fn both_drivers(claude: &FakeDriver, agy: &FakeDriver) -> Drivers {
    let mut drivers = Drivers::new();
    drivers.insert(Arc::new(claude.clone()));
    drivers.insert(Arc::new(agy.clone()));
    drivers
}

pub struct Harness {
    pub link: CoreLink,
    pub core: Option<CoreHandle>,
    pub fake: FakeDriver,
    pub snap: Snapshot,
    pub paths: Paths,
    pub repo: PathBuf,
    /// `TerminalReturned` events, in order: the error each carried.
    pub returned: Vec<Option<String>>,
    /// How many `LendTerminal` events came.
    pub lent: usize,
    /// `CommandFailed` messages, in order.
    pub failures: Vec<String>,
    /// `McpReply` events, in order: what was asked and the answer.
    pub mcp: Vec<(dovetail_api::mcp::McpRequest, dovetail_api::mcp::McpOutcome)>,
    /// `UsageLoaded` events, in order.
    pub usage_loaded: Vec<Vec<dovetail_api::status::DriverUsage>>,
    /// `ResearchEvent::Loaded` events, in order.
    pub loaded: Vec<(TaskId, ResearchWhat, ResearchContent)>,
    /// Every event received, in order.
    pub events: Vec<Event>,
    pub _dir: TempDir,
    pub _run: TempDir,
}

pub const COORDINATOR: &str = "coordinator";

impl Harness {
    pub fn start(driver: FakeDriver, registered: bool, config: Config) -> Harness {
        Self::start_scoped(driver, registered, config, None)
    }

    /// With `scopes` in place of the probe (a temp dir standing in for the cgroup slice).
    pub fn start_scoped(
        driver: FakeDriver,
        registered: bool,
        config: Config,
        scopes: Option<Scopes>,
    ) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        // Unix socket paths are limited to about 108 bytes: keep the runtime dir short.
        let run = tempfile::Builder::new()
            .prefix("dvt.")
            .tempdir_in("/tmp")
            .expect("short tempdir");
        let paths = Paths {
            data: dir.path().join("data"),
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            runtime: run.path().to_path_buf(),
        };
        let repo = dir.path().join("work");
        std::fs::create_dir_all(&repo).expect("work dir");
        let resolution = if registered {
            Resolution::Registered(Project::with_repo("p", "work", repo.clone(), None))
        } else {
            Resolution::Candidate(Candidate {
                name: "work".into(),
                repo_path: repo.clone(),
                git_common_dir: None,
            })
        };
        // `:upgrade` tests give the installed binary, which the app must be running.
        let upgrade = UPGRADE.with(|u| u.borrow_mut().take());
        let options = CoreOptions {
            drivers: Drivers::single(Arc::new(driver.clone())),
            exe: upgrade
                .as_ref()
                .map_or_else(|| PathBuf::from("/usr/bin/dovetail"), |u| u.install.clone()),
            tasks_dir: Some(paths.data.with_file_name("claude-tasks")),
            editor: None,
            claude: None,
            processes: None,
            memory_caps: false,
            check_claude: false,
            scopes,
            upgrade,
            opener: OPENER.with(|o| o.borrow_mut().take()),
            gh: GH.with(|g| g.borrow_mut().take()),
        };
        let (link, core) =
            dovetail_core::spawn(resolution, paths.clone(), config, options).expect("core");
        let mut h = Harness {
            link,
            core: Some(core),
            fake: driver,
            snap: Snapshot::default(),
            paths,
            repo,
            returned: Vec::new(),
            lent: 0,
            failures: Vec::new(),
            mcp: Vec::new(),
            usage_loaded: Vec::new(),
            loaded: Vec::new(),
            events: Vec::new(),
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }

    /// Starts a core on an explicit registered `project` with caller-managed `paths`. A fresh
    /// short runtime directory under `/tmp` is created for the unix socket.
    pub fn start_project(driver: FakeDriver, paths: Paths, project: Project) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = tempfile::Builder::new()
            .prefix("dvt.")
            .tempdir_in("/tmp")
            .expect("short tempdir");
        let paths = Paths {
            runtime: run.path().to_path_buf(),
            ..paths
        };
        let repo = project
            .repos
            .first()
            .map(|r| r.path.clone())
            .unwrap_or_else(|| paths.project_dir(&project.id));
        let options = CoreOptions {
            drivers: Drivers::single(Arc::new(driver.clone())),
            exe: PathBuf::from("/usr/bin/dovetail"),
            tasks_dir: Some(paths.data.with_file_name("claude-tasks")),
            editor: None,
            claude: None,
            processes: None,
            memory_caps: false,
            check_claude: false,
            scopes: None,
            upgrade: None,
            opener: OPENER.with(|o| o.borrow_mut().take()),
            gh: GH.with(|g| g.borrow_mut().take()),
        };
        let (link, core) = dovetail_core::spawn(
            Resolution::Registered(project),
            paths.clone(),
            Config::default(),
            options,
        )
        .expect("core");
        let mut h = Harness {
            link,
            core: Some(core),
            fake: driver,
            snap: Snapshot::default(),
            paths,
            repo,
            returned: Vec::new(),
            lent: 0,
            failures: Vec::new(),
            mcp: Vec::new(),
            usage_loaded: Vec::new(),
            loaded: Vec::new(),
            events: Vec::new(),
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }

    pub fn send(&self, command: Command) {
        self.link.commands.send(command).expect("core is running");
    }

    /// Applies messages until `done` holds on the harness.
    pub fn wait_until(&mut self, what: &str, done: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(self) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}: {:#?}", self.snap));
            match self.link.messages.recv_timeout(left) {
                Ok(ServerMsg::Hello { .. }) => {}
                Ok(ServerMsg::Snapshot(s)) => self.snap = s,
                Ok(ServerMsg::Event { seq, event }) => self.fold(seq, event),
                Err(_) => panic!("timed out waiting for {what}: {:#?}", self.snap),
            }
        }
    }

    /// Applies messages until `done` holds on the snapshot.
    pub fn wait_for(&mut self, what: &str, done: impl Fn(&Snapshot) -> bool) {
        self.wait_until(what, |h| done(&h.snap));
    }

    /// Applies a server message from the core link.
    pub fn fold_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Hello { .. } => {}
            ServerMsg::Snapshot(s) => self.snap = s,
            ServerMsg::Event { seq, event } => self.fold(seq, event),
        }
    }

    /// Applies an event, noting the ones that change nothing in the snapshot.
    pub fn fold(&mut self, seq: u64, event: dovetail_api::Event) {
        match &event {
            dovetail_api::Event::App(AppEvent::TerminalReturned { error }) => {
                self.returned.push(error.clone())
            }
            dovetail_api::Event::App(AppEvent::CommandFailed { message }) => {
                self.failures.push(message.clone())
            }
            dovetail_api::Event::App(AppEvent::TerminalLend) => self.lent += 1,
            dovetail_api::Event::Mcp(McpEvent::Reply {
                request, outcome, ..
            }) => self.mcp.push((request.clone(), outcome.clone())),
            dovetail_api::Event::Status(StatusEvent::UsageLoaded(reports)) => {
                self.usage_loaded.push(reports.clone())
            }
            dovetail_api::Event::Research(ResearchEvent::Loaded {
                task,
                what,
                content,
            }) => self.loaded.push((*task, what.clone(), content.clone())),
            _ => {}
        }
        self.events.push(event.clone());
        apply(&mut self.snap, seq, event);
    }

    /// One tool call from the coordinator, as the bridge makes it.
    pub fn call(&self, call: Tool) -> (String, bool) {
        call_coordinator(self, call)
    }

    /// The snapshot's research summary for `task`, if any.
    pub fn research(&self, task: TaskId) -> Option<&ResearchSummary> {
        self.snap.research.iter().find(|r| r.task == task)
    }

    /// Waits until the driver received a user message containing `needle`.
    pub fn wait_for_note(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let found = self.fake.sent().into_iter().find_map(|s| match s {
                Sent::User(text) if text.contains(needle) => Some(text),
                _ => None,
            });
            if let Some(text) = found {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "no message with {needle:?}: {:#?}",
                self.fake.sent()
            );
            self.drain(Duration::from_millis(50));
        }
    }

    /// Applies whatever arrives within `window`, for checking that something did not happen.
    pub fn drain(&mut self, window: Duration) {
        let until = Instant::now() + window;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match self.link.messages.recv_timeout(left) {
                Ok(ServerMsg::Snapshot(s)) => self.snap = s,
                Ok(ServerMsg::Event { seq, event }) => self.fold(seq, event),
                Ok(ServerMsg::Hello { .. }) => {}
                Err(_) => return,
            }
        }
    }

    pub fn entries(&self, agent: &str) -> Vec<(EntryKind, String)> {
        self.snap
            .conversations
            .get(&AgentId(agent.into()))
            .map(|t| t.entries.iter().map(|e| (e.kind, e.text.clone())).collect())
            .unwrap_or_default()
    }

    pub fn socket(&self) -> PathBuf {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&self.paths.runtime)
            .expect("runtime dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        found.sort();
        found.pop().expect("a socket")
    }

    /// Quits and waits for the core. Returns the state directory, which must outlive a reopen.
    pub fn quit(mut self) -> TempDir {
        self.send(Command::App(AppCommand::Quit { end_sessions: true }));
        let core = self.core.take().expect("core handle");
        assert!(core.wait(Duration::from_secs(10)), "core stopped");
        self._dir
    }
}

/// One NDJSON round trip on the app socket, as the bridge does it.
pub fn socket_call(path: &Path, body: SocketRequestBody) -> SocketResponseBody {
    let stream = UnixStream::connect(path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut line = serde_json::to_string(&SocketRequest { id: 7, body }).expect("encode");
    line.push('\n');
    (&stream).write_all(line.as_bytes()).expect("write");
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).expect("read");
    let response: SocketResponse = serde_json::from_str(&reply).expect("decode");
    assert_eq!(response.id, 7);
    response.body
}

pub fn tool(agent: &str, call: Tool) -> SocketRequestBody {
    SocketRequestBody::ToolCall {
        agent: AgentId(agent.into()),
        tool_use_id: None,
        called_as: None,
        call,
    }
}

pub fn result_text(body: SocketResponseBody) -> (String, bool) {
    match body {
        SocketResponseBody::ToolResult { text, is_error } => (text, is_error),
        other => panic!("not a tool result: {other:?}"),
    }
}

pub fn prompt(h: &Harness, text: &str) {
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: text.into(),
        attachments: Vec::new(),
        effort: None,
    }));
}

pub fn finished_turns(s: &Snapshot) -> usize {
    s.inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::FinishedTurn { .. }))
        .count()
}

impl Harness {
    /// A second app on the state of an earlier one (registered project `p`).
    pub fn start_on(driver: FakeDriver, paths: Paths) -> Harness {
        Self::start_in(driver, paths, None)
    }

    /// A second app on earlier state, in `repo` (a git repo) or a fresh folder.
    pub fn start_in(driver: FakeDriver, paths: Paths, repo: Option<PathBuf>) -> Harness {
        Self::start_with(driver, paths, repo, Config::default())
    }

    pub fn start_with(
        driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
    ) -> Harness {
        Self::start_native(driver, paths, repo, config, None)
    }

    /// [`Harness::start_with`], running `claude` (a fake) for take-over.
    pub fn start_native(
        driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
        claude: Option<std::ffi::OsString>,
    ) -> Harness {
        Self::start_full(driver, paths, repo, config, claude, None)
    }

    /// [`Harness::start_native`], checking take-over locks with `processes` instead of `/proc`.
    pub fn start_full(
        driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
        claude: Option<std::ffi::OsString>,
        processes: Option<Arc<dyn dovetail_core::takeover::Processes + Send + Sync>>,
    ) -> Harness {
        Self::start_drivers(
            Drivers::single(Arc::new(driver.clone())),
            driver,
            paths,
            repo,
            config,
            claude,
            processes,
        )
    }

    /// Registers both Claude and agy `FakeDriver`s on the core (`h.fake` holds `claude_driver`).
    pub fn start_both(
        claude_driver: FakeDriver,
        agy_driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
    ) -> Harness {
        let drivers = both_drivers(&claude_driver, &agy_driver);
        Self::start_drivers(drivers, claude_driver, paths, repo, config, None, None)
    }

    pub fn start_drivers(
        drivers: Drivers,
        driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
        claude: Option<std::ffi::OsString>,
        processes: Option<Arc<dyn dovetail_core::takeover::Processes + Send + Sync>>,
    ) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = tempfile::Builder::new()
            .prefix("dvt.")
            .tempdir_in("/tmp")
            .expect("short tempdir");
        let paths = Paths {
            runtime: run.path().to_path_buf(),
            ..paths
        };
        let git_common_dir = repo.as_ref().map(|r| r.join(".git"));
        // Reopened on earlier state: the same `work` folder the first app ran its slices in, so
        // their sessions resume where they were filed (ADR 0021).
        let earlier = paths.data.with_file_name("work");
        let repo = repo.unwrap_or_else(|| {
            if earlier.is_dir() {
                earlier
            } else {
                dir.path().to_path_buf()
            }
        });
        let project = PROJECT
            .with(|p| p.borrow_mut().take())
            .unwrap_or_else(|| Project::with_repo("p", "work", repo.clone(), git_common_dir));
        let resolution = Resolution::Registered(project);
        let options = CoreOptions {
            drivers,
            exe: PathBuf::from("/usr/bin/dovetail"),
            tasks_dir: Some(paths.data.with_file_name("claude-tasks")),
            editor: None,
            claude,
            processes,
            memory_caps: false,
            check_claude: false,
            scopes: None,
            upgrade: None,
            opener: OPENER.with(|o| o.borrow_mut().take()),
            gh: GH.with(|g| g.borrow_mut().take()),
        };
        let (link, core) =
            dovetail_core::spawn(resolution, paths.clone(), config, options).expect("core");
        let mut h = Harness {
            link,
            core: Some(core),
            fake: driver,
            snap: Snapshot::default(),
            paths,
            repo,
            returned: Vec::new(),
            lent: 0,
            failures: Vec::new(),
            mcp: Vec::new(),
            usage_loaded: Vec::new(),
            loaded: Vec::new(),
            events: Vec::new(),
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }
}

pub fn coordinator_of(s: &Snapshot) -> &dovetail_api::agent::Agent {
    s.agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .expect("coordinator")
}

pub fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

// --- native to-do lists (M11, task #12) ---

/// The temp dir standing in for `~/.claude/tasks` (see `Harness::start`).
pub fn tasks_dir(paths: &Paths) -> PathBuf {
    paths.data.with_file_name("claude-tasks")
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Stores an agent row before the core opens.
pub fn seed(paths: &Paths, row: dovetail_core::db::AgentRow) {
    let db = dovetail_core::db::Db::open(paths, Some("p")).expect("db");
    db.upsert_agent(row).wait().expect("agent");
}

pub fn session_row(
    id: &str,
    kind: AgentKind,
    worktree: Option<PathBuf>,
    home: &Path,
    transcript: PathBuf,
    takeover: Option<dovetail_core::takeover::TakeoverLock>,
) -> dovetail_core::db::AgentRow {
    dovetail_core::db::AgentRow {
        agent: dovetail_api::agent::Agent {
            id: AgentId(id.into()),
            name: id.trim_start_matches("slice-").into(),
            kind,
            parent: None,
            task: None,
            worktree: worktree.map(|path| dovetail_api::agent::WorktreeRef {
                path,
                branch: "dovetail/s4".into(),
            }),
            status: dovetail_api::agent::AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: dovetail_api::agent::SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::model::DriverKind::Claude,
        },
        session_id: Some(format!("{id}-session")),
        transcript_path: Some(transcript),
        task_list_id: None,
        created_ms: 0,
        tool_use_id: None,
        claude_home: Some(home.to_path_buf()),
        takeover,
        model_choice: None,
        repo: None,
    }
}

/// The local date of `days_ago` days back, as journal files are named (ADR 0012).
pub fn local_date(days_ago: i64) -> String {
    let now = jiff::Zoned::now();
    now.date()
        .checked_sub(jiff::Span::new().days(days_ago))
        .expect("a date")
        .to_string()
}

pub fn journal(s: &Snapshot) -> dovetail_api::journal::JournalState {
    s.journal
        .clone()
        .expect("a registered project has a journal")
}

/// Two turns of the coordinator: "first", then a handoff reply. `pause_before` holds the replay
/// before that line of the script.
pub fn fresh_fixture(pause_before: usize) -> FakeDriver {
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"first"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"one","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"fresh"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Open: PR 20 waits on review."}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Open: PR 20 waits on review.","total_cost_usd":0.02}"#,
    ];
    FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![pause_before],
            ..ReplayOptions::default()
        })
}

// --- slices started on a chosen model and effort (task #42) ---

pub fn start_on(
    name: &str,
    model: Option<&str>,
    effort: Option<dovetail_api::model::ReasoningEffort>,
) -> Tool {
    Tool::Project(ProjectTool::StartSlice(StartSlice {
        repo: None,
        model: model.map(str::to_owned),
        effort,
        driver: None,
        task: TaskId(1),
        name: name.into(),
        brief: "go".into(),
    }))
}

pub fn make_task(h: &mut Harness) {
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    h.wait_for("the task", |s| !s.tasks.is_empty());
}

pub fn wait_specs(fake: &FakeDriver, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < n {
        assert!(Instant::now() < deadline, "session {n} never started");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn call_coordinator(h: &Harness, call: Tool) -> (String, bool) {
    result_text(socket_call(&h.socket(), tool(COORDINATOR, call)))
}

// --- Shared domain helpers ---

pub fn s_util(w: &Option<dovetail_api::status::UsageWindow>) -> Option<f64> {
    w.as_ref().map(|w| w.utilization)
}

/// The one subagent node in the snapshot.
pub fn subagent(s: &Snapshot) -> Option<&dovetail_api::agent::Agent> {
    s.agents
        .iter()
        .find(|a| matches!(a.kind, AgentKind::Subagent { .. }))
}

/// Waits until the fake session was sent `wanted` as a user message.
pub fn wait_sent(fake: &FakeDriver, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let wanted = Sent::User(wanted.into());
    while !fake.sent().contains(&wanted) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn ask(h: &Harness, question: &str) -> (String, bool) {
    result_text(socket_call(
        &h.socket(),
        tool(
            COORDINATOR,
            Tool::Report(ReportTool::AskUser(AskUser {
                question: question.into(),
            })),
        ),
    ))
}

pub fn open_questions(s: &Snapshot) -> Vec<InboxId> {
    s.inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::Question { .. }))
        .map(|i| i.id)
        .collect()
}

/// Raises a pending `project_start_slice` approval: the coordinator to acceptEdits, a task, the call.
pub fn pending_start(h: &mut Harness) {
    h.send(Command::Agent(AgentCommand::CyclePermissionMode {
        agent: AgentId(COORDINATOR.into()),
    }));
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    h.wait_for("mode and task", |s| {
        !s.tasks.is_empty()
            && s.agents
                .iter()
                .any(|a| a.permission_mode == PermissionMode::AcceptEdits)
    });
    let (text, _) = result_text(socket_call(
        &h.socket(),
        tool(
            COORDINATOR,
            Tool::Project(ProjectTool::StartSlice(StartSlice {
                repo: None,
                model: None,
                effort: None,
                driver: None,
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            })),
        ),
    ));
    assert!(text.starts_with("pending"), "{text}");
}

pub fn sent_users(fake: &FakeDriver) -> Vec<String> {
    fake.sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::User(t) => Some(t),
            _ => None,
        })
        .collect()
}

pub const EXPIRED: &str =
    "project_start_slice later expired at restart; call it again if still needed";

/// A git repo with slices `names` already open (idle, restored on start), each in a real
/// worktree on its `dovetail/<name>` branch at the main checkout's HEAD.
pub fn repo_with_slices(names: &[&str]) -> (TempDir, PathBuf, Paths) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let paths = Paths {
        data: dir.path().join("data"),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        runtime: dir.path().join("run"),
    };
    std::fs::create_dir_all(paths.worktrees("p")).unwrap();
    let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
    for name in names {
        let wt = paths.worktrees("p").join(name);
        let branch = format!("dovetail/{name}");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", &branch, wt.to_str().unwrap()],
        );
        let agent = dovetail_api::agent::Agent {
            id: AgentId(format!("slice-{name}")),
            name: (*name).into(),
            kind: AgentKind::Slice,
            parent: None,
            task: None,
            worktree: Some(dovetail_api::agent::WorktreeRef { path: wt, branch }),
            status: dovetail_api::agent::AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: dovetail_api::agent::SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::model::DriverKind::Claude,
        };
        db.upsert_agent(dovetail_core::db::AgentRow {
            agent,
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        })
        .wait()
        .expect("agent");
    }
    drop(db);
    (dir, repo, paths)
}

pub fn archive_slice(name: &str) -> Tool {
    Tool::Project(ProjectTool::ArchiveSlice(ArchiveSlice {
        name: name.into(),
    }))
}

pub fn profiles_config(dir: &Path) -> Config {
    let mut config = Config::default();
    for name in ["mine", "work"] {
        config.profiles.insert(
            name.into(),
            dir.join("profiles")
                .join(name)
                .to_string_lossy()
                .into_owned(),
        );
    }
    config
}

pub fn write_recipe(paths: &Paths, text: &str) {
    let file = paths.recipe("p");
    std::fs::create_dir_all(file.parent().expect("parent")).expect("projects dir");
    std::fs::write(file, text).expect("recipe");
}

pub fn error_texts(s: &Snapshot) -> Vec<String> {
    s.inbox
        .iter()
        .filter_map(|i| match &i.kind {
            InboxKind::Error { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

pub fn wait_started(fake: &FakeDriver, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < count {
        assert!(Instant::now() < deadline, "sessions did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn profile_errors(s: &Snapshot, needle: &str) -> usize {
    error_texts(s).iter().filter(|t| t.contains(needle)).count()
}

pub fn wait_sent_shutdown(fake: &FakeDriver) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake.sent().iter().any(|s| matches!(s, Sent::Shutdown(_))) {
        assert!(Instant::now() < deadline, "headless was not stopped");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn effort_named<'a>(s: &'a Snapshot, name: &str) -> Option<&'a dovetail_api::effort::Effort> {
    s.efforts.iter().find(|e| e.name == name)
}

pub fn create_effort(h: &mut Harness, typed: &str, stored: &str) -> dovetail_api::effort::EffortId {
    h.send(Command::Effort(EffortCommand::Create(
        dovetail_api::effort::NewEffort {
            name: typed.into(),
            kind: dovetail_api::effort::EffortKind::Feature,
            question_or_goal: " A buyer can pay without an account ".into(),
        },
    )));
    h.wait_for("the effort", |s| effort_named(s, stored).is_some());
    effort_named(&h.snap, stored).expect("effort").id
}

pub fn wait_sent_containing(fake: &FakeDriver, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake
        .sent()
        .iter()
        .any(|s| matches!(s, Sent::User(text) if text.contains(wanted)))
    {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn task_status(s: &Snapshot, id: i64) -> Option<TaskStatus> {
    s.tasks
        .iter()
        .find(|t| t.id == TaskId(id))
        .map(|t| t.status)
}

pub fn report(
    state: dovetail_tools::coordination::ReportState,
    summary: &str,
    pr: Option<u32>,
) -> Tool {
    Tool::Coordination(CoordinationTool::Report(
        dovetail_tools::coordination::Report {
            state,
            summary: summary.into(),
            pr,
        },
    ))
}

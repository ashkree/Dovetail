//! The harness shared by the session tests: a core on a fake driver and its helpers.
#![allow(dead_code, unused_imports)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::map::{Map, MapId, MapKind};
use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::archive_result::{ARCHIVED, WAITING_FOR_USER};
use dovetail_api::tools::{
    ArchiveSlice, ArchiveTask, AskUser, BucketFilter, DrawMap, GetTask, ListTasks, LogDecision,
    ReportStatus, StartSlice, Tool, UpdateTask,
};
use dovetail_api::{
    AgentId, AgentKind, AgentStatus, Alternative, Attachment, Bucket, Command, CoreLink,
    DecisionId, EntryKind, Event, ImageType, InboxId, InboxKind, NewTask, PLevel, PermissionAnswer,
    PermissionMode, ServerMsg, Snapshot, TaskId, TaskStatus, apply,
};
use dovetail_core::config::Config;
use dovetail_core::memory::Scopes;
use dovetail_core::paths::Paths;
use dovetail_core::projects::{Candidate, Project, Resolution};
use dovetail_core::{CoreHandle, CoreOptions, Drivers};
use dovetail_driver::{ControlAnswer, FakeDriver, ReplayOptions, Sent};
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
    pub mcp: Vec<(dovetail_api::McpRequest, dovetail_api::McpOutcome)>,
    /// `UsageLoaded` events, in order.
    pub usage_loaded: Vec<Vec<dovetail_api::DriverUsage>>,
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
            opener: None,
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
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }

    pub fn send(&self, command: Command) {
        self.link.commands.send(command).expect("core is running");
    }

    /// Applies messages until `done` holds.
    pub fn wait_for(&mut self, what: &str, done: impl Fn(&Snapshot) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&self.snap) {
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

    /// Applies an event, noting the ones that change nothing in the snapshot.
    pub fn fold(&mut self, seq: u64, event: dovetail_api::Event) {
        match &event {
            dovetail_api::Event::TerminalReturned { error } => self.returned.push(error.clone()),
            dovetail_api::Event::CommandFailed { message } => self.failures.push(message.clone()),
            dovetail_api::Event::LendTerminal => self.lent += 1,
            dovetail_api::Event::McpReply {
                request, outcome, ..
            } => self.mcp.push((request.clone(), outcome.clone())),
            dovetail_api::Event::UsageLoaded(reports) => self.usage_loaded.push(reports.clone()),
            _ => {}
        }
        apply(&mut self.snap, seq, event);
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
        self.send(Command::Quit { end_sessions: true });
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
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: text.into(),
        attachments: Vec::new(),
        effort: None,
    });
}

pub fn finished_turns(s: &Snapshot) -> usize {
    s.inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::FinishedTurn { .. }))
        .count()
}

pub fn s_util(w: &Option<dovetail_api::UsageWindow>) -> Option<f64> {
    w.as_ref().map(|w| w.utilization)
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
            opener: None,
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
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }
}

/// The one subagent node in the snapshot.
pub fn subagent(s: &Snapshot) -> Option<&dovetail_api::Agent> {
    s.agents
        .iter()
        .find(|a| matches!(a.kind, AgentKind::Subagent { .. }))
}

pub fn coordinator_of(s: &Snapshot) -> &dovetail_api::Agent {
    s.agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .expect("coordinator")
}

/// `memory.events` text with `oom_kill` at `kills`, as the kernel writes it.
pub fn memory_events(kills: u64) -> String {
    format!("low 0\nhigh 0\nmax {kills}\noom {kills}\noom_kill {kills}\noom_group_kill 0\n")
}

/// The text of the coordinator's out-of-memory items.
pub fn oom_items(s: &Snapshot) -> Vec<String> {
    s.inbox
        .iter()
        .filter_map(|i| match &i.kind {
            InboxKind::Error { text } if text.starts_with("Out of memory") => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Waits until the restarted session was told, in one line, that it was OOM-killed and
/// restarted, followed by the in-flight subagent `work`.
pub fn wait_restart_note(fake: &FakeDriver, work: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let is_note = |s: &Sent| {
        matches!(s, Sent::User(t) if t.lines().next().is_some_and(|l| {
            l.starts_with("You were killed for running out of memory")
                && l.ends_with("and have been restarted.")
        }) && t.contains(work))
    };
    while !fake.sent().iter().any(is_note) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
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
            Tool::AskUser(AskUser {
                question: question.into(),
            }),
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

/// A turn that never reaches its `result`: init, some text, then the replay holds.
pub fn held_turn() -> FakeDriver {
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"working on it"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.01}"#,
    ];
    FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![2],
            ..ReplayOptions::default()
        })
}

pub fn cut_off_text(s: &Snapshot) -> Option<String> {
    s.inbox.iter().find_map(|i| match &i.kind {
        InboxKind::Error { text } if text.contains("cut off by a restart") => Some(text.clone()),
        _ => None,
    })
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

/// Raises a pending `project_start_slice` approval: the coordinator to acceptEdits, a task, the call.
pub fn pending_start(h: &mut Harness) {
    h.send(Command::CyclePermissionMode {
        agent: AgentId(COORDINATOR.into()),
    });
    h.send(Command::CreateTask(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    }));
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
            Tool::StartSlice(StartSlice {
                repo: None,
                model: None,
                effort: None,
                driver: None,
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            }),
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
        let agent = dovetail_api::Agent {
            id: AgentId(format!("slice-{name}")),
            name: (*name).into(),
            kind: AgentKind::Slice,
            parent: None,
            task: None,
            worktree: Some(dovetail_api::WorktreeRef { path: wt, branch }),
            status: dovetail_api::AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: dovetail_api::SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::DriverKind::Claude,
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
    Tool::ArchiveSlice(ArchiveSlice { name: name.into() })
}

pub fn branch_exists(repo: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", branch])
        .stdout(std::process::Stdio::null())
        .status()
        .expect("git")
        .success()
}

// --- updating tasks (task #25) ---

pub fn update(id: i64) -> UpdateTask {
    UpdateTask {
        id: TaskId(id),
        status: None,
        bucket: None,
        p_level: None,
        title: None,
        notes: None,
    }
}

// --- native to-do lists (M11, task #12) ---

/// The temp dir standing in for `~/.claude/tasks` (see `Harness::start`).
pub fn tasks_dir(paths: &Paths) -> PathBuf {
    paths.data.with_file_name("claude-tasks")
}

/// Writes one task file the way Claude Code does.
pub fn write_item(list: &Path, id: &str, subject: &str, status: &str) {
    std::fs::create_dir_all(list).expect("list dir");
    let text = format!(
        "{{\n  \"id\": \"{id}\",\n  \"subject\": \"{subject}\",\n  \"description\": \"{subject}\",\n  \
         \"status\": \"{status}\",\n  \"blocks\": [],\n  \"blockedBy\": []\n}}"
    );
    std::fs::write(list.join(format!("{id}.json")), text).expect("task file");
}

pub fn todo(s: &Snapshot, id: &str) -> Option<dovetail_api::TodoItem> {
    s.todos.iter().find(|t| t.id == id).cloned()
}

pub fn owner(s: &Snapshot, id: &str) -> Option<String> {
    todo(s, id).and_then(|t| t.created_by).map(|a| a.0)
}

// --- Profiles (task #27, ADR 0009) ---

/// State dirs for a run whose recipe and config are written before the core starts.
pub fn profile_paths(dir: &Path) -> Paths {
    Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("unused"),
    }
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

pub fn config_dir_env(spec: &dovetail_driver::SessionSpec) -> Option<String> {
    spec.env
        .iter()
        .find(|(k, _)| k == "CLAUDE_CONFIG_DIR")
        .map(|(_, v)| v.clone())
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

pub fn write_recipe_for(paths: &Paths, project: &str, text: &str) {
    let file = paths.recipe(project);
    std::fs::create_dir_all(file.parent().expect("parent")).expect("projects dir");
    std::fs::write(file, text).expect("recipe");
}

// --- Take-over (M14) ---

/// The transcript folder name Claude Code uses for `cwd`.
pub fn encoded(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A fake native `claude`: records its arguments, folder and Claude home in `out`, appends one
/// native turn to the session's transcript (under `$CLAUDE_CONFIG_DIR/projects/<folder>`), and
/// exits once `wait` exists (at once without one).
pub fn fake_claude(out: &Path, folder: &str, wait: Option<&Path>) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt;
    let wait = wait.map(|w| w.display().to_string()).unwrap_or_default();
    let script = out.join("fake-claude");
    let body = format!(
        r#"#!/bin/sh
out='{out}'
printf '%s\n' "$@" > "$out/argv"
pwd -P > "$out/cwd"
printf '%s' "$CLAUDE_CONFIG_DIR" > "$out/config_dir"
printf '%s' "$DOVETAIL_AGENT_ID" > "$out/agent"
prev=
for a in "$@"; do [ "$prev" = --resume ] && id=$a; prev=$a; done
t="$CLAUDE_CONFIG_DIR/projects/{folder}/$id.jsonl"
mkdir -p "$(dirname "$t")"
cat >> "$t" <<'LINES'
{{"type":"user","message":{{"role":"user","content":"native hello"}}}}
{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"native reply"}},{{"type":"tool_use","id":"t1","name":"Bash","input":{{}}}}]}}}}
LINES
touch "$out/started"
while [ -n '{wait}' ] && [ ! -e '{wait}' ]; do sleep 0.05; done
exit 0
"#,
        out = out.display(),
    );
    std::fs::write(&script, body).expect("fake claude");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("runnable");
    script.into_os_string()
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn lock_of(s: &Snapshot, id: &str) -> dovetail_api::SessionLock {
    s.agents
        .iter()
        .find(|a| a.id.0 == id)
        .map(|a| a.lock)
        .unwrap_or_else(|| panic!("no agent {id}"))
}

pub fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_sent_shutdown(fake: &FakeDriver) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake.sent().iter().any(|s| matches!(s, Sent::Shutdown(_))) {
        assert!(Instant::now() < deadline, "headless was not stopped");
        std::thread::sleep(Duration::from_millis(10));
    }
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
        agent: dovetail_api::Agent {
            id: AgentId(id.into()),
            name: id.trim_start_matches("slice-").into(),
            kind,
            parent: None,
            task: None,
            worktree: worktree.map(|path| dovetail_api::WorktreeRef {
                path,
                branch: "dovetail/s4".into(),
            }),
            status: dovetail_api::AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: dovetail_api::SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::DriverKind::Claude,
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

/// Opens a project whose coordinator was taken over when Dovetail stopped: its transcript has
/// one line from before the take-over and one native turn after, and its lock names `pid`.
pub fn open_after_crash(pid: u32, ticks: Option<u64>) -> (Harness, FakeDriver, TempDir) {
    use dovetail_core::takeover::TakeoverLock;
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    // Filed where Claude Code files the coordinator's session: under its `home/` folder.
    let transcript = dovetail_core::session::launch::transcript_path(
        &mine,
        &paths.project_home("p"),
        &format!("{COORDINATOR}-session"),
    );
    std::fs::create_dir_all(transcript.parent().expect("parent")).expect("dir");
    std::fs::write(&transcript, "{\"type\":\"summary\"}\n").expect("transcript");
    let offset = std::fs::metadata(&transcript).expect("len").len();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("open");
    writeln!(
        file,
        "{{\"type\":\"user\",\"message\":{{\"content\":\"typed while Dovetail was down\"}}}}"
    )
    .expect("append");
    let lock = TakeoverLock {
        since_ms: 1,
        pid: Some(pid),
        start_ticks: ticks,
        transcript_offset: Some(offset),
    };
    seed(
        &paths,
        session_row(
            COORDINATOR,
            AgentKind::Coordinator,
            None,
            &mine,
            transcript,
            Some(lock),
        ),
    );
    let fake = fixture("plain_turn");
    let h = Harness::start_with(fake.clone(), paths, None, config);
    (h, fake, dir)
}

/// Processes that cannot be checked, as when `/proc` is unreadable.
pub struct Blind;

impl dovetail_core::takeover::Processes for Blind {
    fn start_ticks(&self, _: u32) -> std::io::Result<Option<u64>> {
        Err(std::io::Error::other("no /proc here"))
    }
    fn find_native(&self, _: &str) -> std::io::Result<Option<u32>> {
        Err(std::io::Error::other("no /proc here"))
    }
}

// --- maps_draw (M15) ---

pub fn draw(h: &Harness, map: serde_json::Value) -> (String, bool) {
    result_text(socket_call(
        &h.socket(),
        tool(COORDINATOR, Tool::DrawMap(DrawMap(map))),
    ))
}

pub fn checkout_map(title: &str) -> serde_json::Value {
    serde_json::json!({
        "map": { "title": title, "type": "structure" },
        "nodes": [
            { "id": "api", "kind": "endpoint", "label": "POST /checkout", "description": "Takes the order." },
            { "id": "orders", "kind": "data", "label": "orders", "description": "Orders table." }
        ],
        "connections": [ { "from": "api", "to": "orders", "label": "writes" } ]
    })
}

pub fn login_lanes(lane_node: &str) -> serde_json::Value {
    serde_json::json!({
        "map": { "title": "Logging in", "type": "swimlane" },
        "nodes": [
            { "id": "user", "kind": "actor", "label": "User", "description": "Signs in." },
            { "id": "check", "kind": "endpoint", "label": "/login", "description": "Checks it." }
        ],
        "lanes": [
            { "id": "you", "label": "User" },
            { "id": "web", "label": "Web", "node": lane_node }
        ],
        "flows": [ { "name": "Log in", "steps": [
            { "node": "user", "lane": "you", "note": "Types a password." },
            { "node": "check", "lane": "web", "arrow": "submits", "note": "Checks it." }
        ] } ]
    })
}

/// Sends `LoadMap` and waits for its answer, folding events in on the way.
pub fn load_map(h: &mut Harness, id: &str, version: Option<u32>) -> Result<Map, String> {
    h.send(Command::LoadMap {
        id: MapId(id.into()),
        version,
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("timed out waiting for the map");
        match h.link.messages.recv_timeout(left).expect("a message") {
            ServerMsg::Event { seq, event } => {
                let answer = match &event {
                    Event::MapLoaded(map) => Some(Ok(map.clone())),
                    Event::CommandFailed { message } => Some(Err(message.clone())),
                    _ => None,
                };
                apply(&mut h.snap, seq, event);
                if let Some(answer) = answer {
                    return answer;
                }
            }
            ServerMsg::Snapshot(s) => h.snap = s,
            ServerMsg::Hello { .. } => {}
        }
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

pub fn journal(s: &Snapshot) -> dovetail_api::JournalState {
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

/// No fresh start raises a Needs-you item: the old transcript just stays on disk (task #38).
pub fn no_permission_item(s: &Snapshot) -> bool {
    s.inbox
        .iter()
        .all(|i| !matches!(i.kind, InboxKind::Permission { .. }))
}

// --- efforts (docs/design-v2.md §4) ---

pub fn effort_named<'a>(s: &'a Snapshot, name: &str) -> Option<&'a dovetail_api::Effort> {
    s.efforts.iter().find(|e| e.name == name)
}

pub fn create_effort(h: &mut Harness, typed: &str, stored: &str) -> dovetail_api::EffortId {
    h.send(Command::CreateEffort(dovetail_api::NewEffort {
        name: typed.into(),
        kind: dovetail_api::EffortKind::Feature,
        question_or_goal: " A buyer can pay without an account ".into(),
    }));
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

// --- blocks and cards (ADR 0025, ADR 0027) ---

pub fn idea(effort: dovetail_api::EffortId, text: &str) -> dovetail_api::NewCard {
    dovetail_api::NewCard {
        effort,
        block: None,
        kind: dovetail_api::CardKind::Idea,
        reference: None,
        text: text.into(),
    }
}

pub fn card_titled<'a>(s: &'a Snapshot, title: &str) -> Option<&'a dovetail_api::Card> {
    s.cards.iter().find(|c| c.title == title)
}

// --- session names and slice reports (ADR 0022), automatic task status (ADR 0024) ---

pub fn task_status(s: &Snapshot, id: i64) -> Option<TaskStatus> {
    s.tasks
        .iter()
        .find(|t| t.id == TaskId(id))
        .map(|t| t.status)
}

pub fn report(state: dovetail_api::tools::ReportState, summary: &str, pr: Option<u32>) -> Tool {
    Tool::Report(dovetail_api::tools::Report {
        state,
        summary: summary.into(),
        pr,
    })
}

/// A fake `gh`: `gh pr list --head <branch> ...` prints `<dir>/<branch with _ for />.json`, or
/// `[]` when there is no such file.
pub fn fake_gh(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("gh");
    let text = format!(
        "#!/bin/sh\nbranch=\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = --head ]; then branch=$2; fi\n  \
         shift\ndone\nfile=\"{}/$(echo \"$branch\" | tr / _).json\"\nif [ -f \"$file\" ]; then cat \
         \"$file\"; else echo '[]'; fi\n",
        dir.display()
    );
    std::fs::write(&script, text).expect("fake gh");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

pub fn set_pr(dir: &Path, slice: &str, state: &str) {
    let text = format!(
        "[{{\"number\": 42, \"state\": \"{state}\", \"url\": \"https://example.test/pr/42\"}}]"
    );
    std::fs::write(dir.join(format!("dovetail_{slice}.json")), text).expect("pr file");
}

/// Slices `names` in a git repo, each on its own task (ids from 1, in order), all in progress.
/// With `shared`, every slice works on task 1.
pub fn repo_with_task_slices(names: &[&str], shared: bool) -> (TempDir, PathBuf, Paths) {
    use dovetail_api::{NewTask, TaskChange};
    let (dir, repo, paths) = repo_with_slices(names);
    let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
    let mut rows = db.load_agents().wait().expect("agents");
    for (index, name) in names.iter().enumerate() {
        let task = if shared && index > 0 {
            TaskId(1)
        } else {
            let created = db
                .create_task(
                    NewTask {
                        title: format!("task for {name}"),
                        p_level: None,
                        bucket: None,
                        notes: String::new(),
                        follow_up_of: None,
                        pr: None,
                        related: Vec::new(),
                        effort: None,
                        criterion: None,
                    },
                    false,
                )
                .wait()
                .expect("task");
            let id = created[0].id;
            db.update_task(id, vec![TaskChange::Status(TaskStatus::InProgress)])
                .wait()
                .expect("status");
            id
        };
        let row = rows
            .iter_mut()
            .find(|r| r.agent.name == *name)
            .expect("slice row");
        row.agent.task = Some(task);
        db.upsert_agent(row.clone()).wait().expect("agent");
    }
    drop(db);
    (dir, repo, paths)
}

// --- multi-repo projects (ADR 0021) ---

/// A git repo with one commit at `<dir>/<name>`, by its canonical path.
pub fn committed_repo(dir: &Path, name: &str) -> PathBuf {
    let repo = dir.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), name).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    repo.canonicalize().unwrap()
}

/// Project `p`, registered in the registry with a repo for each of `names`, and the paths
/// under `dir` it lives in. The next harness opens it.
pub fn registered_project(dir: &Path, names: &[&str]) -> (Paths, Vec<PathBuf>) {
    let paths = Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("run"),
    };
    let repos: Vec<PathBuf> = names.iter().map(|n| committed_repo(dir, n)).collect();
    let db = dovetail_core::db::Db::open(&paths, None).expect("registry");
    let mut project = db
        .create_project("p".into(), "work".into(), None, None)
        .wait()
        .expect("project");
    for repo in &repos {
        let new = dovetail_core::projects::repo_at(repo, None).expect("repo");
        project = db.add_repo("p".into(), new).wait().expect("add");
    }
    drop(db);
    PROJECT.with(|p| *p.borrow_mut() = Some(project));
    (paths, repos)
}

pub fn replied(s: &Snapshot) -> bool {
    s.conversations
        .get(&AgentId(COORDINATOR.into()))
        .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
}

// ---- ADR 0026: slice previews and declared variants

/// The preview state of slice `name` in the snapshot.
pub fn preview_of(s: &Snapshot, agent: &str) -> Option<dovetail_api::PreviewState> {
    s.previews
        .iter()
        .find(|p| p.agent.0 == agent)
        .map(|p| p.state.clone())
}

/// `GET /` of a preview URL, as plain HTTP on a socket: the body, or `None` if nothing answers.
pub fn fetch(url: &str) -> Option<String> {
    let addr = url.strip_prefix("http://")?.trim_end_matches('/');
    let mut stream = std::net::TcpStream::connect(addr).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream
        .write_all(format!("GET / HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
        .ok()?;
    let mut body = String::new();
    std::io::Read::read_to_string(&mut stream, &mut body).ok()?;
    Some(body)
}

/// Waits (up to 5 s) for nothing to answer at `url`.
pub fn gone(url: &str) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if fetch(url).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub fn start_viewer_harness(recipe: Option<&str>) -> (Harness, TempDir, PathBuf) {
    let (dir, repo, paths) = repo_with_slices(&["web"]);
    if let Some(text) = recipe {
        write_recipe(&paths, text);
    }
    let mut h = Harness::start_with(
        fixture("plain_turn"),
        paths.clone(),
        Some(repo.clone()),
        Config::default(),
    );
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "web"));
    (h, dir, repo)
}

pub fn declare(slots: Vec<dovetail_api::VariantSlot>) -> Tool {
    Tool::DeclareVariants(dovetail_api::tools::DeclareVariants { slots })
}

pub fn slot(
    name: &str,
    level: dovetail_api::VariantLevel,
    options: &[&str],
) -> dovetail_api::VariantSlot {
    dovetail_api::VariantSlot {
        name: name.into(),
        level,
        options: options.iter().map(|o| (*o).into()).collect(),
        kept: None,
    }
}

// --- slices started on a chosen model and effort (task #42) ---

pub fn start_on(
    name: &str,
    model: Option<&str>,
    effort: Option<dovetail_api::ReasoningEffort>,
) -> Tool {
    Tool::StartSlice(StartSlice {
        repo: None,
        model: model.map(str::to_owned),
        effort,
        driver: None,
        task: TaskId(1),
        name: name.into(),
        brief: "go".into(),
    })
}

pub fn make_task(h: &mut Harness) {
    h.send(Command::CreateTask(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
}

pub fn wait_specs(fake: &FakeDriver, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < n {
        assert!(Instant::now() < deadline, "session {n} never started");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The spec the session named `p.<name>` was started with, once it has started. The
/// coordinator is woken by "slice ready" and starts too, so a slice is found by its name.
pub fn wait_spec(fake: &FakeDriver, name: &str) -> dovetail_driver::SessionSpec {
    let want = format!("p.{name}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = fake
            .started_specs()
            .into_iter()
            .rev()
            .find(|s| s.name.as_deref() == Some(want.as_str()));
        if let Some(spec) = found {
            return spec;
        }
        assert!(Instant::now() < deadline, "{want} never started");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn call_coordinator(h: &Harness, call: Tool) -> (String, bool) {
    result_text(socket_call(&h.socket(), tool(COORDINATOR, call)))
}

pub fn notices_containing(s: &Snapshot, agent: &str, needle: &str) -> usize {
    s.conversations.get(&AgentId(agent.into())).map_or(0, |t| {
        t.entries.iter().filter(|e| e.text.contains(needle)).count()
    })
}

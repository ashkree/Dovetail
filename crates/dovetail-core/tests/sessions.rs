//! The real core driven by `FakeDriver` replaying recorded fixtures: the stream rules that are
//! cheap to replay (double `result`, concurrent permissions, subagent routing), the inbox round
//! trips, the tool handlers over the real socket, and ADR 0005's unregistered folders.

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
use dovetail_core::{CoreHandle, CoreOptions};
use dovetail_driver::{ControlAnswer, FakeDriver, ReplayOptions, Sent};
use tempfile::TempDir;

thread_local! {
    /// Taken by the next `Harness::start` on this test's thread: what `:upgrade` runs with.
    static UPGRADE: std::cell::RefCell<Option<dovetail_core::service::UpgradeOptions>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Taken by the next `Harness::start` on this test's thread: the project it opens, when a
    /// test registered its own (ADR 0021: several repos).
    static PROJECT: std::cell::RefCell<Option<Project>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Taken by the next `Harness::start` on this test's thread: the fake `gh` it looks pull
    /// requests up with.
    static GH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn fixture(name: &str) -> FakeDriver {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures")
        .join(format!("{name}.ndjson"));
    FakeDriver::from_file(path).expect("fixture")
}

struct Harness {
    link: CoreLink,
    core: Option<CoreHandle>,
    fake: FakeDriver,
    snap: Snapshot,
    paths: Paths,
    repo: PathBuf,
    /// `TerminalReturned` events, in order: the error each carried.
    returned: Vec<Option<String>>,
    /// How many `LendTerminal` events came.
    lent: usize,
    /// `CommandFailed` messages, in order.
    failures: Vec<String>,
    _dir: TempDir,
    _run: TempDir,
}

const COORDINATOR: &str = "coordinator";

impl Harness {
    fn start(driver: FakeDriver, registered: bool, config: Config) -> Harness {
        Self::start_scoped(driver, registered, config, None)
    }

    /// With `scopes` in place of the probe (a temp dir standing in for the cgroup slice).
    fn start_scoped(
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
            driver: Arc::new(driver.clone()),
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
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }

    fn send(&self, command: Command) {
        self.link.commands.send(command).expect("core is running");
    }

    /// Applies messages until `done` holds.
    fn wait_for(&mut self, what: &str, done: impl Fn(&Snapshot) -> bool) {
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
    fn fold(&mut self, seq: u64, event: dovetail_api::Event) {
        match &event {
            dovetail_api::Event::TerminalReturned { error } => self.returned.push(error.clone()),
            dovetail_api::Event::CommandFailed { message } => self.failures.push(message.clone()),
            dovetail_api::Event::LendTerminal => self.lent += 1,
            _ => {}
        }
        apply(&mut self.snap, seq, event);
    }

    /// Applies whatever arrives within `window`, for checking that something did not happen.
    fn drain(&mut self, window: Duration) {
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

    fn entries(&self, agent: &str) -> Vec<(EntryKind, String)> {
        self.snap
            .conversations
            .get(&AgentId(agent.into()))
            .map(|t| t.entries.iter().map(|e| (e.kind, e.text.clone())).collect())
            .unwrap_or_default()
    }

    fn socket(&self) -> PathBuf {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&self.paths.runtime)
            .expect("runtime dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        found.sort();
        found.pop().expect("a socket")
    }

    /// Quits and waits for the core. Returns the state directory, which must outlive a reopen.
    fn quit(mut self) -> TempDir {
        self.send(Command::Quit { end_sessions: true });
        let core = self.core.take().expect("core handle");
        assert!(core.wait(Duration::from_secs(10)), "core stopped");
        self._dir
    }
}

/// One NDJSON round trip on the app socket, as the bridge does it.
fn socket_call(path: &Path, body: SocketRequestBody) -> SocketResponseBody {
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

fn tool(agent: &str, call: Tool) -> SocketRequestBody {
    SocketRequestBody::ToolCall {
        agent: AgentId(agent.into()),
        tool_use_id: None,
        called_as: None,
        call,
    }
}

fn result_text(body: SocketResponseBody) -> (String, bool) {
    match body {
        SocketResponseBody::ToolResult { text, is_error } => (text, is_error),
        other => panic!("not a tool result: {other:?}"),
    }
}

fn prompt(h: &Harness, text: &str) {
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: text.into(),
        attachments: Vec::new(),
        effort: None,
    });
}

fn finished_turns(s: &Snapshot) -> usize {
    s.inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::FinishedTurn { .. }))
        .count()
}

#[test]
fn a_second_input_sent_mid_turn_keeps_the_session_working_until_its_own_result() {
    // Two inputs, two results; the replay holds before each result so the state in between
    // can be checked.
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"first"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"one","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"second"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"two"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"two","total_cost_usd":0.02}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![3, 4],
            ..ReplayOptions::default()
        });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "first");
    prompt(&h, "second");
    h.wait_for("both inputs and the first reply", |s| {
        let entries = s.conversations.get(&AgentId(COORDINATOR.into()));
        entries.is_some_and(|t| {
            t.entries
                .iter()
                .filter(|e| e.kind == EntryKind::User)
                .count()
                == 2
                && t.entries.iter().any(|e| e.text == "one")
        })
    });
    fake.resume();
    h.wait_for("the first result", |s| {
        s.agents.iter().any(|a| (a.cost_usd - 0.01).abs() < 1e-9)
    });
    // The second input is still queued: not idle, and no finished turn yet.
    assert_eq!(finished_turns(&h.snap), 0);
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .unwrap();
    assert_eq!(coordinator.status, dovetail_api::AgentStatus::Working);
    fake.resume();
    h.wait_for("the second result", |s| finished_turns(s) == 1);
    h.quit();
}

fn s_util(w: &Option<dovetail_api::UsageWindow>) -> Option<f64> {
    w.as_ref().map(|w| w.utilization)
}

#[test]
fn the_core_checks_the_editors_path_before_asking_the_ui_to_let_go() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let missing = h.repo.join("no-such-folder");
    h.send(Command::OpenEditor {
        path: missing.clone(),
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(h.lent, 0, "the UI was told to let go of the terminal");
    assert_eq!(h.returned, Vec::<Option<String>>::new());
    assert_eq!(
        h.failures,
        vec![format!("{} does not exist", missing.display())]
    );

    // A path that exists is accepted; until the UI answers, a second request is refused and a
    // stray release does nothing.
    h.send(Command::OpenEditor {
        path: h.repo.clone(),
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(h.lent, 1);
    h.send(Command::OpenEditor {
        path: h.repo.clone(),
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(h.lent, 1);
    assert_eq!(h.failures.len(), 2);
    assert_eq!(h.failures[1], "the terminal is already lent out");
}

#[test]
fn a_peer_message_becomes_an_agent_message_entry() {
    let fake = fixture("peer_message");
    let mut h = Harness::start(fake, true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let entries = h.entries(COORDINATOR);
    let text = dovetail_api::agent_message_text("scout", false, "The report.\nSecond line.");
    assert!(
        entries.contains(&(EntryKind::AgentMessage, text)),
        "{entries:?}"
    );
    assert!(
        !entries
            .iter()
            .any(|(k, t)| *k == EntryKind::Notice && t.contains("scout"))
    );
}

#[test]
fn a_plain_turn_streams_into_the_conversation_and_survives_a_restart() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let entries = h.entries(COORDINATOR);
    assert_eq!(
        entries[0],
        (EntryKind::User, "Reply with exactly the word: alpha".into())
    );
    assert!(entries.contains(&(EntryKind::Assistant, "alpha".into())));
    assert_eq!(
        fake.sent(),
        vec![Sent::User("Reply with exactly the word: alpha".into())]
    );
    // Task #16: the turn's rate-limit event and the session's model reach the UI.
    h.wait_for("the usage limits", |s| s.usage.five_hour.is_some());
    assert_eq!(s_util(&h.snap.usage.five_hour), Some(0.11));
    assert_eq!(s_util(&h.snap.usage.seven_day), Some(0.17));
    let model = h
        .snap
        .agents
        .iter()
        .find(|a| a.id.0 == COORDINATOR)
        .and_then(|a| a.model.clone());
    assert_eq!(model.as_deref(), Some("claude-sonnet-5-5"));
    // M13c: 19 564 tokens in context (input, cache read and creation, output) of the 1M window
    // the result reports.
    let fill = coordinator_of(&h.snap).context_pct;
    assert_eq!(fill, Some(2));

    // Launch flags: auto mode, our MCP config without --strict-mcp-config, our settings.
    let spec = &fake.started_specs()[0];
    let args = spec.claude_args();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "auto")
    );
    assert!(args.contains(&"--mcp-config".to_string()));
    assert!(
        !args
            .iter()
            .any(|a| a.contains("strict-mcp-config") || a == "--bare")
    );
    assert!(
        spec.allowed_tools
            .contains(&"mcp__dovetail__project_start_slice".to_string())
    );
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CODE_TASK_LIST_ID" && v == "dovetail-p-coordinator")
    );
    let mcp = std::fs::read_to_string(spec.mcp_config_path.as_ref().expect("mcp")).expect("read");
    assert!(mcp.contains("DOVETAIL_AGENT_ID") && mcp.contains("\"coordinator\""));
    // No profile in the recipe: the session inherits Dovetail's own Claude home (ADR 0009).
    assert!(!spec.env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR"));

    // The hook and the bridge are pinned to this app's state, whatever a slice's env says.
    let settings =
        std::fs::read_to_string(spec.settings_path.as_ref().expect("settings")).expect("read");
    let data = h.paths.data.to_string_lossy().into_owned();
    assert!(
        settings.contains(&format!("DOVETAIL_DATA_DIR='{data}'")),
        "{settings}"
    );
    assert!(mcp.contains("DOVETAIL_RUNTIME_DIR"), "{mcp}");

    let paths = h.paths.clone();
    let _state = h.quit();
    // Reopen on the same state: the conversation and the session id are still there.
    let fake = fixture("plain_turn");
    let h = Harness::start_on(fake.clone(), paths);
    assert!(
        h.entries(COORDINATOR)
            .contains(&(EntryKind::Assistant, "alpha".into()))
    );
    h.quit();
}

/// Task #26: Claude Code 2.1.319 can send a `rate_limit_event` with only the weekly window
/// (`rate_limit_week_only` holds the live line). The core reports the weekly window and no 5-hour
/// one; the header turns that into `5h —` (dovetail-tui `usage_header` tests).
#[test]
fn a_rate_limit_event_with_only_the_weekly_window_reaches_the_ui_as_such() {
    let mut h = Harness::start(fixture("rate_limit_week_only"), true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the usage limits", |s| s.usage.seven_day.is_some());
    assert_eq!(s_util(&h.snap.usage.seven_day), Some(0.06));
    assert_eq!(
        h.snap.usage.seven_day.as_ref().and_then(|w| w.resets_at_ms),
        Some(1_794_754_800_000)
    );
    assert_eq!(h.snap.usage.five_hour, None);
    h.quit();
}

#[test]
fn a_prompt_with_an_image_sends_an_image_block_and_keeps_the_path_after_a_restart() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let dir = h.paths.data.join("attachments");
    std::fs::create_dir_all(&dir).expect("attachments dir");
    let path = dir.join("1.png");
    let bytes = b"\x89PNG\r\n\x1a\nfake";
    std::fs::write(&path, bytes).expect("write image");
    let attachment = Attachment {
        path: path.clone(),
        media_type: ImageType::Png,
        size: bytes.len() as u64,
    };
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "what is this?".into(),
        attachments: vec![attachment.clone()],
        effort: None,
    });
    h.wait_for("the reply", |s| finished_turns(s) == 1);

    let lines = fake.user_messages();
    assert_eq!(lines.len(), 1);
    let content = lines[0]["message"]["content"]
        .as_array()
        .expect("content blocks");
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["type"], "base64");
    assert_eq!(content[0]["source"]["media_type"], "image/png");
    // base64 of the 12 bytes above.
    assert_eq!(content[0]["source"]["data"], "iVBORw0KGgpmYWtl");
    assert_eq!(content[1]["text"], "what is this?");

    let user = |h: &Harness| {
        h.snap.conversations[&AgentId(COORDINATOR.into())]
            .entries
            .iter()
            .find(|e| e.kind == EntryKind::User)
            .cloned()
            .expect("the user entry")
    };
    assert_eq!(user(&h).attachments, vec![attachment.clone()]);

    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(user(&h).attachments, vec![attachment]);
    h.quit();
}

impl Harness {
    /// A second app on the state of an earlier one (registered project `p`).
    fn start_on(driver: FakeDriver, paths: Paths) -> Harness {
        Self::start_in(driver, paths, None)
    }

    /// A second app on earlier state, in `repo` (a git repo) or a fresh folder.
    fn start_in(driver: FakeDriver, paths: Paths, repo: Option<PathBuf>) -> Harness {
        Self::start_with(driver, paths, repo, Config::default())
    }

    fn start_with(
        driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
    ) -> Harness {
        Self::start_native(driver, paths, repo, config, None)
    }

    /// [`Harness::start_with`], running `claude` (a fake) for take-over.
    fn start_native(
        driver: FakeDriver,
        paths: Paths,
        repo: Option<PathBuf>,
        config: Config,
        claude: Option<std::ffi::OsString>,
    ) -> Harness {
        Self::start_full(driver, paths, repo, config, claude, None)
    }

    /// [`Harness::start_native`], checking take-over locks with `processes` instead of `/proc`.
    fn start_full(
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
            driver: Arc::new(driver.clone()),
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
            _dir: dir,
            _run: run,
        };
        h.wait_for("the snapshot", |s| !s.agents.is_empty());
        h
    }
}

#[test]
fn a_background_subagent_means_one_finished_turn_and_none_of_its_events() {
    let mut h = Harness::start(fixture("double_result"), true, Config::default());
    prompt(&h, "start a background subagent");
    h.wait_for("the last result", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::FinishedTurn { preview } if preview.contains("finished"))
        })
    });
    // Three results, but the first came while the subagent ran, and the later ones replace.
    assert_eq!(finished_turns(&h.snap), 1);
    let entries = h.entries(COORDINATOR);
    // The subagent's Bash and handback are routed away from the parent's conversation.
    assert!(entries.contains(&(EntryKind::ToolSummary, "Agent".into())));
    assert!(!entries.iter().any(|(_, t)| t.contains("Bash")));
    h.quit();
}

#[test]
fn concurrent_permissions_are_answered_in_any_order_and_non_auto_init_warns() {
    let fake = fixture("concurrent_permissions").with_options(ReplayOptions {
        wait_for_answers: true,
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "read two files");
    h.wait_for("two permission requests", |s| {
        s.inbox
            .iter()
            .filter(|i| matches!(i.kind, InboxKind::Permission { .. }))
            .count()
            == 2
    });
    // The fixture's init reports `default`: run as manual and warn.
    assert!(h.snap.warnings.iter().any(|w| w.text.contains("not auto")));
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .unwrap();
    assert_eq!(coordinator.permission_mode, PermissionMode::Manual);

    let ids: Vec<_> = h.snap.inbox.iter().map(|i| i.id).collect();
    // The second first, then the first: neither blocks the other.
    h.send(Command::AnswerPermission {
        item: ids[1],
        answer: PermissionAnswer::Allow,
    });
    h.send(Command::AnswerPermission {
        item: ids[0],
        answer: PermissionAnswer::AllowAlways,
    });
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let answers: Vec<_> = fake
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::Answer { id, answer } => Some((id, answer)),
            _ => None,
        })
        .collect();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0].1, ControlAnswer::allow());
    // "Always allow" sends the offered rule, kept to this session (never written to the repo).
    match &answers[1].1 {
        ControlAnswer::Allow {
            updated_permissions,
            ..
        } => assert_eq!(updated_permissions[0]["destination"], "session"),
        other => panic!("{other:?}"),
    }
    assert!(
        h.entries(COORDINATOR)
            .contains(&(EntryKind::Assistant, "uno dos".into()))
    );
    h.quit();
}

#[test]
fn a_subagents_permission_request_reaches_the_inbox_marked_as_its() {
    let fake = fixture("subagent_permission").with_options(ReplayOptions {
        wait_for_answers: true,
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake, true, Config::default());
    prompt(&h, "foreground subagent");
    h.wait_for("the request", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Permission { reason: Some(r), .. } if r.starts_with("from subagent"))
        })
    });
    // The foreground subagent is a node under the coordinator while it runs.
    let sub = subagent(&h.snap).expect("a subagent node");
    assert_eq!(sub.kind, AgentKind::Subagent { background: false });
    assert_eq!(sub.name, "Read one.txt file");
    assert_eq!(sub.status, AgentStatus::Working);
    let id = h.snap.inbox[0].id;
    h.send(Command::AnswerPermission {
        item: id,
        answer: PermissionAnswer::Deny { message: None },
    });
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let sub = subagent(&h.snap).expect("a subagent node");
    assert_eq!(sub.status, AgentStatus::Done);
    let progress = sub.progress.as_ref().expect("progress");
    assert_eq!(progress.tool_uses, 1);
    assert!(
        progress
            .summary
            .as_deref()
            .is_some_and(|s| s.contains("uno")),
        "{progress:?}"
    );
    h.quit();
}

/// The one subagent node in the snapshot.
fn subagent(s: &Snapshot) -> Option<&dovetail_api::Agent> {
    s.agents
        .iter()
        .find(|a| matches!(a.kind, AgentKind::Subagent { .. }))
}

fn coordinator_of(s: &Snapshot) -> &dovetail_api::Agent {
    s.agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .expect("coordinator")
}

#[test]
fn a_background_subagent_is_a_node_with_progress_cost_and_its_summary_at_the_end() {
    // Out lines 8 and 22 are the two `result`s; 17 is `task_updated` (completed).
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![8, 17, 22],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let sub = subagent(&h.snap).expect("node").clone();
    assert_eq!(sub.kind, AgentKind::Subagent { background: true });
    assert_eq!(sub.parent, Some(AgentId(COORDINATOR.into())));
    assert_eq!(
        sub.id,
        AgentId("coordinator:toolu_01WrSGRZBAmUyjsWgzW1pdsV".into())
    );
    assert_eq!(sub.name, "Background sleep task");
    assert_eq!(sub.status, AgentStatus::Working);

    // Past the first `result` and the subagent's progress: the parent is not done.
    fake.resume();
    h.wait_for("the last progress", |s| {
        subagent(s)
            .and_then(|a| a.progress.as_ref())
            .is_some_and(|p| p.tool_uses == 2)
    });
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .expect("coordinator");
    assert!((coordinator.cost_usd - 0.0494268).abs() < 1e-6);
    assert_eq!(coordinator.status, AgentStatus::Working);
    assert_eq!(finished_turns(&h.snap), 0);
    let sub = subagent(&h.snap).expect("node");
    // M13c: a subagent's calls fill its own context, not its parent's; it shows no fill.
    assert_eq!(sub.context_pct, None);
    let progress = sub.progress.as_ref().expect("progress");
    assert_eq!(progress.total_tokens, 17164);
    assert_eq!(progress.duration_ms, 11302);
    assert_eq!(progress.last_tool.as_deref(), Some("SubagentHandback"));
    assert_eq!(sub.status, AgentStatus::Working);

    // Completed, with the hand-back as its summary, before the parent's second `result`.
    fake.resume();
    h.wait_for("the summary", |s| {
        subagent(s)
            .and_then(|a| a.progress.as_ref())
            .is_some_and(|p| p.summary.is_some())
    });
    let sub = subagent(&h.snap).expect("node");
    assert_eq!(sub.status, AgentStatus::Done);
    assert_eq!(sub.status_line, "bg-done");
    let progress = sub.progress.as_ref().expect("progress");
    assert_eq!(progress.summary.as_deref(), Some("bg-done"));
    assert_eq!(progress.total_tokens, 17175);
    assert!(
        progress
            .output_file
            .as_ref()
            .is_some_and(|f| f.ends_with("ad6b787476c37b6da.output"))
    );
    // The session's total is summed from the task events, not taken from its `result`.
    h.wait_for("the session total", |s| {
        coordinator_of(s).subagent_tokens == 17175
    });
    // Its own Bash task is not a node.
    assert_eq!(
        h.snap
            .agents
            .iter()
            .filter(|a| matches!(a.kind, AgentKind::Subagent { .. }))
            .count(),
        1
    );
    assert_eq!(finished_turns(&h.snap), 0);

    fake.resume();
    h.wait_for("the finished turn", |s| finished_turns(s) == 1);
    // The parent's cost includes the subagent's; the subagent adds nothing to the totals.
    h.wait_for("the totals", |s| (s.totals.cost_usd - 0.1).abs() < 1e-9);
    assert_eq!(subagent(&h.snap).expect("node").cost_usd, 0.0);
    // Its Bash call never reached the parent's conversation.
    assert!(
        !h.entries(COORDINATOR)
            .iter()
            .any(|(_, t)| t.contains("Bash"))
    );

    // The node survives a restart, summary and all.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    let sub = subagent(&h.snap).expect("node after restart");
    assert_eq!(coordinator_of(&h.snap).subagent_tokens, 17175);
    assert_eq!(sub.status, AgentStatus::Done);
    assert_eq!(sub.parent, Some(AgentId(COORDINATOR.into())));
    assert_eq!(
        sub.progress.as_ref().and_then(|p| p.summary.as_deref()),
        Some("bg-done")
    );
    h.quit();
}

#[test]
fn a_subagent_cut_off_by_a_restart_reopens_as_stopped() {
    // Hold before the subagent's first progress: it is still running at quit.
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![9],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    let sub = subagent(&h.snap).expect("node after restart");
    assert_eq!(sub.status, AgentStatus::Error);
    assert!(
        sub.status_line.starts_with("stopped"),
        "{}",
        sub.status_line
    );
    // Only the parent's cut-off turn is reported, not the subagent.
    assert!(
        h.snap
            .inbox
            .iter()
            .all(|i| i.agent == AgentId(COORDINATOR.into()))
    );
    h.quit();
}

/// `memory.events` text with `oom_kill` at `kills`, as the kernel writes it.
fn memory_events(kills: u64) -> String {
    format!("low 0\nhigh 0\nmax {kills}\noom {kills}\noom_kill {kills}\noom_group_kill 0\n")
}

/// The text of the coordinator's out-of-memory items.
fn oom_items(s: &Snapshot) -> Vec<String> {
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
fn wait_restart_note(fake: &FakeDriver, work: &str) {
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

#[test]
fn a_session_killed_for_memory_is_resumed_with_its_subagent_work_listed() {
    // A temp dir stands in for the cgroup slice; the coordinator's scope is in it.
    let slice = tempfile::tempdir().expect("tempdir");
    let cgroup = slice.path().join("dovetail-p-coordinator.scope");
    std::fs::create_dir(&cgroup).expect("scope dir");
    std::fs::write(cgroup.join("memory.events"), memory_events(0)).expect("events");
    // Hold before the background subagent's first progress, then die by SIGKILL there.
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![9],
        kill_first_before: Some((9, 9)),
        ..ReplayOptions::default()
    });
    let scopes = Scopes::Available {
        slice_dir: slice.path().to_path_buf(),
        escape_dollars: false,
    };
    let mut h = Harness::start_scoped(fake.clone(), true, Config::default(), Some(scopes));
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let description = subagent(&h.snap)
        .and_then(|a| a.progress.as_ref())
        .map(|p| p.description.clone())
        .expect("its description");
    // The kernel counts the kill (seen through the watch), then the scope goes with the
    // session, as `--collect` makes it.
    std::fs::write(cgroup.join("memory.events"), memory_events(1)).expect("events");
    h.drain(Duration::from_millis(300));
    std::fs::remove_dir_all(&cgroup).expect("remove scope");
    fake.resume();
    h.wait_for("the out-of-memory item", |s| !oom_items(s).is_empty());
    let text = oom_items(&h.snap).remove(0);
    assert!(text.contains("restarted from its session"), "{text}");
    assert!(text.contains(&description), "{text}");
    // Resumed from its session id, and told what was in flight.
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 2);
    assert!(specs[1].resume);
    assert_eq!(specs[1].session_id, specs[0].session_id);
    wait_restart_note(&fake, &description);
    h.wait_for("the coordinator running again", |s| {
        coordinator_of(s).status != AgentStatus::Error
    });
    // Only one kill was seen: no item about a tool.
    assert_eq!(oom_items(&h.snap).len(), 1);
    fake.resume();
    h.quit();
}

#[test]
fn an_oom_count_that_arrives_after_the_exit_still_restarts_the_session() {
    let slice = tempfile::tempdir().expect("tempdir");
    let cgroup = slice.path().join("dovetail-p-coordinator.scope");
    std::fs::create_dir(&cgroup).expect("scope dir");
    std::fs::write(cgroup.join("memory.events"), memory_events(0)).expect("events");
    let fake = fixture("subagent_background").with_options(ReplayOptions {
        pause_before: vec![9],
        kill_first_before: Some((9, 9)),
        ..ReplayOptions::default()
    });
    let scopes = Scopes::Available {
        slice_dir: slice.path().to_path_buf(),
        escape_dollars: false,
    };
    let mut h = Harness::start_scoped(fake.clone(), true, Config::default(), Some(scopes));
    prompt(&h, "start a background subagent");
    h.wait_for("the subagent node", |s| subagent(s).is_some());
    let description = subagent(&h.snap)
        .and_then(|a| a.progress.as_ref())
        .map(|p| p.description.clone())
        .expect("its description");
    // The exit is handled first, with no kill counted yet: it reads as a crash.
    fake.resume();
    let crashed = |s: &Snapshot| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Error { text } if text.starts_with("session ended unexpectedly"))
        })
    };
    h.wait_for("the crash item", crashed);
    // Then the watcher's count arrives.
    std::fs::write(cgroup.join("memory.events"), memory_events(1)).expect("events");
    h.wait_for("the out-of-memory item", |s| !oom_items(s).is_empty());
    assert!(!crashed(&h.snap), "the crash item is replaced");
    let text = oom_items(&h.snap).remove(0);
    assert!(text.contains("restarted from its session"), "{text}");
    assert!(text.contains(&description), "{text}");
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 2);
    assert!(specs[1].resume);
    wait_restart_note(&fake, &description);
    fake.resume();
    h.quit();
}

#[test]
fn report_status_sets_the_line_cut_at_120_characters() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::ReportStatus(ReportStatus {
                status: format!("{}\nsecond line", "x".repeat(200)),
            }),
        ),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("the status line", |s| {
        s.agents.iter().any(|a| a.status_line.len() == 120)
    });

    h.quit();
}

/// Waits until the fake session was sent `wanted` as a user message.
fn wait_sent(fake: &FakeDriver, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let wanted = Sent::User(wanted.into());
    while !fake.sent().contains(&wanted) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ask(h: &Harness, question: &str) -> (String, bool) {
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

fn open_questions(s: &Snapshot) -> Vec<InboxId> {
    s.inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::Question { .. }))
        .map(|i| i.id)
        .collect()
}

#[test]
fn ask_user_returns_at_once_and_an_inbox_answer_arrives_as_a_message() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    // `socket_call` times out after 5 s, so a held call would fail here.
    let (text, is_error) = ask(&h, "Which port?");
    assert!(!is_error && text.starts_with("Question sent"), "{text}");
    h.wait_for("the question", |s| open_questions(s).len() == 1);
    // It is in the conversation too.
    assert!(
        h.entries(COORDINATOR)
            .iter()
            .any(|(k, t)| *k == EntryKind::Notice && t.contains("Which port?")),
        "{:?}",
        h.entries(COORDINATOR)
    );
    let id = open_questions(&h.snap)[0];
    h.send(Command::AnswerQuestion {
        item: id,
        text: "8080".into(),
    });
    wait_sent(&fake, "Answer to your question 'Which port?': 8080");
    h.wait_for("the item to clear", |s| open_questions(s).is_empty());
    h.quit();
}

#[test]
fn a_chat_message_with_open_questions_goes_through_and_clears_them() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    ask(&h, "Which port?");
    ask(&h, "Which host?");
    h.wait_for("both questions", |s| open_questions(s).len() == 2);
    prompt(&h, "8080 on localhost");
    wait_sent(&fake, "8080 on localhost");
    h.wait_for("the items to clear", |s| open_questions(s).is_empty());
    h.quit();
}

/// Review of 2026-11-09: an image-only message answered open questions with "".
#[test]
fn an_image_only_message_leaves_open_questions_open() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    ask(&h, "Which port?");
    h.wait_for("the question", |s| open_questions(s).len() == 1);
    let dir = h.paths.data.join("attachments");
    std::fs::create_dir_all(&dir).expect("attachments dir");
    let path = dir.join("1.png");
    let bytes = b"\x89PNG\r\n\x1a\nfake";
    std::fs::write(&path, bytes).expect("write image");
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: String::new(),
        attachments: vec![Attachment {
            path,
            media_type: ImageType::Png,
            size: bytes.len() as u64,
        }],
        effort: None,
    });
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    assert_eq!(fake.user_messages().len(), 1, "the image went out");
    assert_eq!(open_questions(&h.snap).len(), 1, "still waiting for words");
    h.quit();
}

#[test]
fn slices_cannot_call_coordinator_tools_and_bad_names_are_refused() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
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
    let socket = h.socket();
    let start = |name: &str| {
        Tool::StartSlice(StartSlice {
            repo: None,
            task: TaskId(1),
            name: name.into(),
            brief: "go".into(),
        })
    };
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, start("agent-x"))));
    assert!(
        is_error && text.starts_with("denied") && text.contains("agent-"),
        "{text}"
    );

    // A folder without git: the slice shares it and is ready at once.
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, start("one"))));
    assert!(!is_error && text.starts_with("started"), "{text}");
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "one"));
    let (text, is_error) = result_text(socket_call(&socket, tool("slice-one", start("two"))));
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );
    h.wait_for("the shared-folder brief", |s| {
        s.conversations
            .get(&AgentId("slice-one".into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.text.contains("shared project folder"))
            })
    });
    h.quit();
}

#[test]
fn slices_over_the_cap_queue_until_one_is_archived() {
    let config = Config {
        max_slices: 1,
        ..Config::default()
    };
    let mut h = Harness::start(fixture("plain_turn"), true, config);
    for name in ["first", "second"] {
        h.send(Command::NewSlice {
            repo: None,
            task: None,
            name: name.into(),
            brief: "go".into(),
        });
    }
    h.wait_for("the queue", |s| {
        s.activity.iter().any(|a| a.text.contains("second queued"))
    });
    assert!(!h.snap.agents.iter().any(|a| a.name == "second"));
    h.send(Command::NewSlice {
        repo: None,
        task: None,
        name: "third".into(),
        brief: "go".into(),
    });
    h.wait_for("the second queued start", |s| {
        s.activity.iter().any(|a| a.text.contains("third queued"))
    });
    h.send(Command::CloseSlice {
        agent: AgentId("slice-first".into()),
    });
    h.wait_for("the queued slice", |s| {
        s.agents.iter().any(|a| a.name == "second") && !s.agents.iter().any(|a| a.name == "first")
    });

    // The queue survives a restart: `third` starts as soon as there is room (cap 3 here).
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the slice queued before the restart", |s| {
        s.agents.iter().any(|a| a.name == "third")
    });
    h.quit();
}

#[test]
fn starting_a_slice_on_a_next_task_moves_it_to_now_in_progress() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::CreateTask(NewTask {
        title: "t".into(),
        p_level: None,
        bucket: Some(Bucket::Next),
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    assert_eq!(h.snap.tasks[0].bucket, Some(Bucket::Next));
    h.send(Command::NewSlice {
        repo: None,
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    });
    let started = |s: &Snapshot| {
        s.tasks.iter().any(|t| {
            t.id == id && t.status == TaskStatus::InProgress && t.bucket == Some(Bucket::Now)
        })
    };
    h.wait_for("the task in Now, in progress", started);

    // Through the storage thread: still there after a restart.
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the task after the restart", started);
    h.quit();
}

#[test]
fn an_unregistered_folder_keeps_nothing_until_added() {
    let mut h = Harness::start(fixture("plain_turn"), false, Config::default());
    assert!(h.snap.warnings.iter().any(|w| w.id.0 == "unregistered"));
    let socket = h.socket();
    match socket_call(
        &socket,
        SocketRequestBody::AddTask {
            task: NewTask {
                title: "t".into(),
                p_level: None,
                bucket: None,
                notes: String::new(),
                follow_up_of: None,
                pr: None,
                related: vec![],
                effort: Default::default(),
                criterion: None,
            },
        },
    ) {
        SocketResponseBody::Error { message } => {
            assert!(message.contains("not a Dovetail project"))
        }
        other => panic!("{other:?}"),
    }
    h.send(Command::CreateTask(NewTask {
        title: "kept after adding".into(),
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
    // Only Dovetail's own built-in Tangents (ADR 0017) has a folder; nothing for this one.
    let written: Vec<String> = std::fs::read_dir(h.paths.data.join("projects"))
        .map(|dir| {
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(written, ["tangents"], "nothing written yet");
    // A slice may start before the folder is a project (decided 2026-11-06).
    h.send(Command::NewSlice {
        repo: None,
        task: None,
        name: "early".into(),
        brief: "go".into(),
    });
    h.wait_for("the early slice's brief", |s| {
        s.conversations
            .get(&AgentId("slice-early".into()))
            .is_some_and(|t| !t.entries.is_empty())
    });
    let path = h.repo.clone();
    h.send(Command::RegisterProject { path });
    h.wait_for("registration", |s| {
        s.warnings.iter().all(|w| w.id.0 != "unregistered")
    });
    assert!(h.paths.state_db("work").exists());

    // Regression: after registration everything is named after the project id, not `new-<pid>`.
    h.send(Command::NewSlice {
        repo: None,
        task: Some(TaskId(1)),
        name: "after".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice's brief", |s| {
        s.conversations
            .get(&AgentId("slice-after".into()))
            .is_some_and(|t| !t.entries.is_empty())
    });
    let fake = h.fake.clone();
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| s.name.as_deref().is_some_and(|n| n.ends_with(".after")))
        .expect("slice spec");
    let env = |key: &str| {
        spec.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        env("CLAUDE_CODE_TASK_LIST_ID").as_deref(),
        Some("dovetail-work-task-1")
    );
    assert_eq!(env("DOVETAIL_PROJECT").as_deref(), Some("work"));
    let settings = spec.settings_path.clone().expect("settings");
    assert!(
        settings.starts_with(h.paths.state.join("run").join("work")),
        "{settings:?}"
    );
    let paths = h.paths.clone();
    let _state = h.quit();
    // The slice started before registration was written to the project's state.db.
    let db = dovetail_core::db::Db::open(&paths, Some("work")).expect("db");
    let agents = db.load_agents().wait().expect("agents");
    assert!(agents.iter().any(|r| r.agent.name == "early"), "{agents:?}");
    // Sessions that ran before registration keep the list id their process used.
    for name in ["early", COORDINATOR] {
        let row = agents.iter().find(|r| r.agent.name == name).expect("agent");
        let list = row.task_list_id.as_deref().unwrap_or_default();
        assert!(list.starts_with("dovetail-new-"), "{name}: {list}");
    }
}

#[test]
fn start_slice_outside_auto_mode_returns_pending_and_the_answer_comes_as_a_message() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    // auto → acceptEdits: approval is needed now.
    h.send(Command::CyclePermissionMode {
        agent: coordinator.clone(),
    });
    h.wait_for("the mode", |s| {
        s.agents
            .iter()
            .any(|a| a.permission_mode == PermissionMode::AcceptEdits)
    });
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::StartSlice(StartSlice {
                repo: None,
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            }),
        ),
    ));
    // No task #1 yet: refused before anyone is asked.
    assert!(is_error && text.starts_with("denied"), "{text}");
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
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::StartSlice(StartSlice {
                repo: None,
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            }),
        ),
    ));
    assert!(!is_error && text.starts_with("pending"), "{text}");
    h.wait_for("the approval item", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == dovetail_api::START_SLICE_PERMISSION))
    });
    let id = h.snap.inbox[0].id;
    h.send(Command::AnswerPermission {
        item: id,
        answer: PermissionAnswer::Allow,
    });
    h.wait_for("the approval message", |s| {
        s.conversations.get(&coordinator).is_some_and(|t| {
            t.entries
                .iter()
                .any(|e| e.text.contains("project_start_slice later approved"))
        })
    });
    assert!(h.snap.agents.iter().any(|a| a.name == "later"));
    h.quit();
}

#[test]
fn interrupt_reaches_a_session_waiting_on_background_tasks() {
    // Live check 2026-11-06: `sleep 30` went to the background, the turn's result came, the
    // row said working, and interrupt was refused as "not mid-turn".
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"b1","task_type":"local_bash","description":"sleep 30"}]}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"started it","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![3],
            ..ReplayOptions::default()
        });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "run sleep 30");
    h.wait_for("the result with a background task left", |s| {
        s.agents.iter().any(|a| (a.cost_usd - 0.01).abs() < 1e-9)
    });
    let coordinator = h
        .snap
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .unwrap();
    assert_eq!(coordinator.status, dovetail_api::AgentStatus::Working);
    h.send(Command::Interrupt {
        agent: AgentId(COORDINATOR.into()),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake.sent().contains(&Sent::Interrupt) {
        assert!(
            Instant::now() < deadline,
            "no interrupt sent: {:?}",
            fake.sent()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    fake.resume();
    h.quit();
}

/// A turn that never reaches its `result`: init, some text, then the replay holds.
fn held_turn() -> FakeDriver {
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

fn cut_off_text(s: &Snapshot) -> Option<String> {
    s.inbox.iter().find_map(|i| match &i.kind {
        InboxKind::Error { text } if text.contains("cut off by a restart") => Some(text.clone()),
        _ => None,
    })
}

#[test]
fn a_turn_cut_off_by_quit_is_reported_on_reopen_and_not_continued() {
    let mut h = Harness::start(held_turn(), true, Config::default());
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    let paths = h.paths.clone();
    let _state = h.quit();
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    let text = cut_off_text(&h.snap).unwrap_or_default();
    assert!(
        text.contains("coordinator's turn") && text.contains("rename the flag"),
        "{text}"
    );
    // The coordinator's session is back, but nothing was sent to it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "the coordinator was not resumed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fake.started_specs()[0].resume, "resumed, not created");
    assert!(
        !fake.sent().iter().any(|s| matches!(s, Sent::User(_))),
        "{:?}",
        fake.sent()
    );
    h.quit();
}

#[test]
fn killing_the_app_mid_turn_loses_no_state() {
    let mut h = Harness::start(held_turn(), true, Config::default());
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.agents
            .iter()
            .any(|a| a.status == dovetail_api::AgentStatus::Working)
            && s.conversations
                .get(&AgentId(COORDINATOR.into()))
                .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    // Give the DB thread a moment, then copy the database files as a kill would leave them:
    // the app never gets to quit.
    std::thread::sleep(Duration::from_millis(200));
    let crashed = tempfile::tempdir().expect("tempdir");
    let from = h.paths.state_db("p");
    let to_paths = Paths {
        data: crashed.path().join("data"),
        ..h.paths.clone()
    };
    let to = to_paths.state_db("p");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{suffix}", from.display()));
        if src.exists() {
            std::fs::copy(&src, format!("{}{suffix}", to.display())).unwrap();
        }
    }
    let mut reopened = Harness::start_on(fixture("plain_turn"), to_paths);
    reopened.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    let entries = reopened.entries(COORDINATOR);
    assert!(
        entries.contains(&(EntryKind::User, "rename the flag".into())),
        "{entries:?}"
    );
    reopened.quit();
    h.quit();
}

#[test]
fn at_restart_permissions_expire_and_questions_stay_answerable() {
    let fake = fixture("concurrent_permissions").with_options(ReplayOptions {
        wait_for_answers: true,
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake, true, Config::default());
    prompt(&h, "read two files");
    h.wait_for("two permission requests", |s| {
        s.inbox
            .iter()
            .filter(|i| matches!(i.kind, InboxKind::Permission { .. }))
            .count()
            == 2
    });
    let socket = h.socket();
    let (text, _) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::AskUser(AskUser {
                question: "Which port?".into(),
            }),
        ),
    ));
    assert!(text.starts_with("Question sent"), "{text}");
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.wait_for("the expired requests", |s| {
        s.activity
            .iter()
            .filter(|a| a.text.contains("request expired at restart"))
            .count()
            == 2
            && s.inbox
                .iter()
                .any(|i| matches!(i.kind, InboxKind::Question { .. }))
    });
    assert!(
        h.snap
            .activity
            .iter()
            .any(|a| a.text.contains("Read one.txt"))
    );
    assert!(
        !h.snap
            .inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::Permission { .. }))
    );
    let question = h
        .snap
        .inbox
        .iter()
        .find(|i| matches!(i.kind, InboxKind::Question { .. }))
        .unwrap_or_else(|| panic!("the question stays: {:#?}", h.snap.inbox))
        .id;
    // A question does not explain the stopped turn (the agent kept working): its cut-off is
    // reported too.
    h.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    h.send(Command::AnswerQuestion {
        item: question,
        text: "8080".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let wanted = Sent::User("Answer to your question 'Which port?': 8080".into());
    while !fake.sent().contains(&wanted) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
    h.quit();
}

fn git(dir: &Path, args: &[&str]) {
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

#[test]
fn the_sweep_archives_lost_slices_and_asks_about_unowned_worktrees() {
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
    // Two slices whose worktree directories have disappeared: "lost" is gone for git too,
    // "listed" is still in git's worktree list, so it is not proof enough and is kept.
    std::fs::create_dir_all(paths.worktrees("p")).unwrap();
    let listed = paths.worktrees("p").join("listed");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "dovetail/listed",
            listed.to_str().unwrap(),
        ],
    );
    std::fs::remove_dir_all(&listed).unwrap();
    for name in ["lost", "listed"] {
        let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
        let mut agent = dovetail_api::Agent {
            id: AgentId(format!("slice-{name}")),
            name: name.into(),
            kind: AgentKind::Slice,
            parent: None,
            task: None,
            worktree: Some(dovetail_api::WorktreeRef {
                path: paths.worktrees("p").join(name),
                branch: format!("dovetail/{name}"),
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
        };
        agent.status_line.clear();
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
    // Two worktrees no agent owns, one with an unpushed commit.
    std::fs::create_dir_all(paths.worktrees("p")).unwrap();
    for name in ["orphan", "spare"] {
        let wt = paths.worktrees("p").join(name);
        let branch = format!("dovetail/{name}");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", &branch, wt.to_str().unwrap()],
        );
    }
    std::fs::write(paths.worktrees("p").join("orphan/new.txt"), "x").unwrap();
    git(&paths.worktrees("p").join("orphan"), &["add", "."]);
    git(
        &paths.worktrees("p").join("orphan"),
        &["commit", "-qm", "work"],
    );

    // Two plain folders: "junk" holds two files; "swapped" is replaced after it is inspected.
    for name in ["junk", "swapped"] {
        let dir = paths.worktrees("p").join(name);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "b").unwrap();
    }

    let mut h = Harness::start_in(fixture("plain_turn"), paths.clone(), Some(repo.clone()));
    h.wait_for("the sweep", |s| {
        s.activity.iter().any(|a| a.text.contains("lost archived: its worktree is gone"))
            && s.inbox
                .iter()
                .filter(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == dovetail_api::REMOVE_WORKTREE_PERMISSION))
                .count()
                == 4
    });
    assert!(!h.snap.agents.iter().any(|a| a.name == "lost"));
    assert!(
        h.snap.agents.iter().any(|a| a.name == "listed"),
        "kept: git still lists it"
    );
    let item = |s: &Snapshot, name: &str| {
        s.inbox
            .iter()
            .find(|i| i.agent.0 == format!("worktree:{name}"))
            .map(|i| (i.id, i.kind.clone()))
    };
    let (orphan, kind) = item(&h.snap, "orphan").expect("orphan item");
    match kind {
        InboxKind::Permission { input, .. } => assert_eq!(input["unpushed_commits"], 1),
        other => panic!("{other:?}"),
    }
    let (spare, _) = item(&h.snap, "spare").expect("spare item");
    let (junk, kind) = item(&h.snap, "junk").expect("junk item");
    match kind {
        InboxKind::Permission { input, .. } => assert_eq!(input["files"], 2),
        other => panic!("{other:?}"),
    }
    // Replaced since it was inspected: `y` leaves it alone.
    let (swapped, _) = item(&h.snap, "swapped").expect("swapped item");
    let swapped_dir = paths.worktrees("p").join("swapped");
    std::fs::remove_dir_all(&swapped_dir).unwrap();
    std::fs::create_dir_all(&swapped_dir).unwrap();
    h.send(Command::AnswerPermission {
        item: swapped,
        answer: PermissionAnswer::Allow,
    });
    h.send(Command::AnswerPermission {
        item: junk,
        answer: PermissionAnswer::Allow,
    });
    h.wait_for("the swapped folder left alone", |s| {
        s.activity
            .iter()
            .any(|a| a.text.contains("swapped: changed since Dovetail asked"))
            && s.activity.iter().any(|a| a.text == "worktree junk removed")
    });
    assert!(swapped_dir.exists());
    assert!(!paths.worktrees("p").join("junk").exists(), "removed on y");
    // y removes; d keeps and is remembered.
    h.send(Command::AnswerPermission {
        item: orphan,
        answer: PermissionAnswer::Allow,
    });
    h.send(Command::AnswerPermission {
        item: spare,
        answer: PermissionAnswer::Deny { message: None },
    });
    h.wait_for("removal and keep", |s| {
        s.activity
            .iter()
            .any(|a| a.text == "worktree orphan removed")
            && s.activity
                .iter()
                .any(|a| a.text.contains("worktree spare kept"))
    });
    assert!(!paths.worktrees("p").join("orphan").exists());
    assert!(
        paths.worktrees("p").join("spare").exists(),
        "never deleted without y"
    );
    let _state = h.quit();

    let mut h = Harness::start_in(fixture("plain_turn"), paths.clone(), Some(repo));
    h.drain(Duration::from_millis(500));
    assert!(
        item(&h.snap, "spare").is_none(),
        "a kept worktree is not asked about again"
    );
    h.quit();
}

#[test]
fn on_reopen_the_coordinator_resumes_at_once_and_a_slice_on_first_touch() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    prompt(&h, "hello");
    h.send(Command::NewSlice {
        repo: None,
        task: None,
        name: "later".into(),
        brief: "go".into(),
    });
    // Both sessions have run once their replies are in.
    h.wait_for("both sessions to have run", |s| {
        ["coordinator", "slice-later"].iter().all(|a| {
            s.conversations
                .get(&AgentId((*a).into()))
                .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
        })
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    let names = |fake: &FakeDriver| -> Vec<(String, bool)> {
        fake.started_specs()
            .iter()
            .map(|s| (s.name.clone().unwrap_or_default(), s.resume))
            .collect()
    };
    assert_eq!(names(&fake), vec![("p.coordinator".to_string(), true)]);
    h.send(Command::Prompt {
        agent: AgentId("slice-later".into()),
        text: "carry on".into(),
        attachments: Vec::new(),
        effort: None,
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while names(&fake).len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", names(&fake));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(names(&fake)[1], ("p.later".to_string(), true));
    h.quit();
}

/// ADR 0016 migration: a conversation from before the rename, resumed, keeps calling the old
/// names. They stay allowed, and a call by one answers as the new tool with a note naming it.
#[test]
fn a_resumed_conversation_calling_an_old_tool_name_still_works() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    prompt(&h, "hello");
    h.wait_for("the first run", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    // The resumed session's model still knows the tool as `report_status`.
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"carry on"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"mcp__dovetail__report_status","input":{"status":"resumed"}}]},"parent_tool_use_id":null}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"carried on"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"carried on","total_cost_usd":0.01}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n")).expect("fixture");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    let spec = &fake.started_specs()[0];
    assert!(spec.resume);
    for name in ["report_status", "core_report_status"] {
        let tool = format!("mcp__dovetail__{name}");
        assert!(spec.allowed_tools.contains(&tool), "{tool} not allowed");
    }

    prompt(&h, "carry on");
    h.wait_for("the resumed turn, its old-name tool use tallied", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries.iter().any(|e| e.text == "carried on")
                    && t.entries.iter().any(|e| {
                        e.kind == EntryKind::ToolSummary && e.text.contains("report_status")
                    })
            })
    });

    // What the bridge forwards for that call: the new tool, and the name it was called by.
    let call = Tool::parse("report_status", serde_json::json!({"status": "resumed"}))
        .expect("the old name parses");
    assert_eq!(call.name(), "core_report_status");
    let (text, is_error) = result_text(socket_call(
        &h.socket(),
        SocketRequestBody::ToolCall {
            agent: AgentId(COORDINATOR.into()),
            tool_use_id: Some("t1".into()),
            call,
            called_as: Some("report_status".into()),
        },
    ));
    assert!(!is_error, "{text}");
    assert!(
        text.ends_with("Note: `report_status` is deprecated and will be removed; call `core_report_status` instead."),
        "{text}"
    );
    h.wait_for("the status line", |s| {
        s.agents.iter().any(|a| a.status_line == "resumed")
    });
    h.quit();
}

/// Live rehearsal 2026-11-06: two "slice ready" notes sent mid-turn were folded into the
/// running turn (both echoed before its one `result`); the coordinator stayed "working".
#[test]
fn inputs_folded_into_the_running_turn_end_with_its_result() {
    // Out line 13 is the echo of the first note; hold before it so the notes can be sent.
    let fake = fixture("merged_inputs").with_options(ReplayOptions {
        pause_before: vec![13],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "create tasks and start two slices");
    h.wait_for("the turn to reach the notes", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::ToolSummary))
    });
    prompt(&h, "slice a1 ready");
    prompt(&h, "slice b1 ready");
    h.wait_for("both notes sent", |_| {
        fake.sent()
            .iter()
            .filter(|s| matches!(s, Sent::User(_)))
            .count()
            == 3
    });
    fake.resume();
    h.wait_for("the one result to end the turn", |s| {
        finished_turns(s) == 1
            && s.agents.iter().any(|a| {
                a.kind == AgentKind::Coordinator && a.status != dovetail_api::AgentStatus::Working
            })
    });
    h.quit();
}

/// Raises a pending `project_start_slice` approval: the coordinator to acceptEdits, a task, the call.
fn pending_start(h: &mut Harness) {
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
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            }),
        ),
    ));
    assert!(text.starts_with("pending"), "{text}");
}

fn sent_users(fake: &FakeDriver) -> Vec<String> {
    fake.sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::User(t) => Some(t),
            _ => None,
        })
        .collect()
}

const EXPIRED: &str = "project_start_slice later expired at restart; call it again if still needed";

#[test]
fn a_start_slice_approval_pending_at_quit_is_answered_after_reopen() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    pending_start(&mut h);
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sent_users(&fake).contains(&EXPIRED.to_string()) {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
    h.drain(Duration::from_millis(100));
    assert!(
        !h.snap
            .activity
            .iter()
            .any(|a| a.text.contains("request expired")),
        "Dovetail's own approval is not logged as an expired request"
    );
    h.quit();
}

#[test]
fn notes_for_a_coordinator_cut_off_by_a_restart_wait_for_the_users_next_message() {
    let mut h = Harness::start(held_turn(), true, Config::default());
    pending_start(&mut h);
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    let paths = h.paths.clone();
    let _state = h.quit();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    h.wait_for("the cut-off item", |s| cut_off_text(s).is_some());
    h.drain(Duration::from_millis(200));
    assert!(
        sent_users(&fake).is_empty(),
        "nothing continues it: {:?}",
        fake.sent()
    );
    prompt(&h, "carry on");
    let deadline = Instant::now() + Duration::from_secs(5);
    while sent_users(&fake).len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", fake.sent());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        sent_users(&fake),
        vec![EXPIRED.to_string(), "carry on".to_string()]
    );
    let _state = h.quit();
    // Delivered, so not sent again on the next open.
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    assert!(!sent_users(&fake).contains(&EXPIRED.to_string()));
    h.quit();
}

#[test]
fn a_turn_that_ends_normally_during_quit_is_not_reported_as_cut_off() {
    let fake = held_turn();
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "rename the flag");
    h.wait_for("the turn to start", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "working on it"))
    });
    // Quit while the turn runs; its result arrives inside the quit window.
    h.send(Command::Quit { end_sessions: true });
    std::thread::sleep(Duration::from_millis(100));
    fake.resume();
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.drain(Duration::from_millis(300));
    assert!(cut_off_text(&h.snap).is_none(), "{:#?}", h.snap.inbox);
    h.quit();
}

/// A git repo with slices `names` already open (idle, restored on start), each in a real
/// worktree on its `dovetail/<name>` branch at the main checkout's HEAD.
fn repo_with_slices(names: &[&str]) -> (TempDir, PathBuf, Paths) {
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

fn archive_slice(name: &str) -> Tool {
    Tool::ArchiveSlice(ArchiveSlice { name: name.into() })
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", branch])
        .stdout(std::process::Stdio::null())
        .status()
        .expect("git")
        .success()
}

#[test]
fn archive_slice_removes_a_clean_worktree_and_branch_and_frees_the_queue() {
    let (_dir, repo, paths) = repo_with_slices(&["done"]);
    let config = Config {
        max_slices: 1,
        ..Config::default()
    };
    let mut h = Harness::start_with(
        fixture("plain_turn"),
        paths.clone(),
        Some(repo.clone()),
        config,
    );
    h.wait_for("the restored slice", |s| {
        s.agents.iter().any(|a| a.name == "done")
    });
    h.send(Command::NewSlice {
        repo: None,
        task: None,
        name: "next".into(),
        brief: "go".into(),
    });
    h.wait_for("the queued start", |s| {
        s.activity.iter().any(|a| a.text.contains("next queued"))
    });
    let socket = h.socket();

    // A slice may not archive, not even itself.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool("slice-done", archive_slice("done")),
    ));
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("coordinator")),
    ));
    assert!(is_error && text.contains("coordinator cannot"), "{text}");
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("nope")),
    ));
    assert!(is_error && text.contains("no slice named nope"), "{text}");

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("done")),
    ));
    assert!(!is_error && text.starts_with(ARCHIVED), "{text}");
    assert!(
        !paths.worktrees("p").join("done").exists(),
        "worktree removed"
    );
    assert!(!branch_exists(&repo, "dovetail/done"), "branch removed");
    h.wait_for("the queued slice started in the freed place", |s| {
        s.agents.iter().any(|a| a.name == "next") && !s.agents.iter().any(|a| a.name == "done")
    });

    // Archived already: it is unknown now.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("done")),
    ));
    assert!(is_error && text.contains("already archived"), "{text}");
    h.quit();
}

#[test]
fn archive_slice_with_unpushed_work_asks_the_user_and_keeps_it() {
    let (_dir, repo, paths) = repo_with_slices(&["wip"]);
    let wt = paths.worktrees("p").join("wip");
    std::fs::write(wt.join("new.txt"), "x").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-qm", "unpushed"]);
    let mut h = Harness::start_in(fixture("plain_turn"), paths.clone(), Some(repo.clone()));
    h.wait_for("the restored slice", |s| {
        s.agents.iter().any(|a| a.name == "wip")
    });
    let socket = h.socket();

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("wip")),
    ));
    assert!(!is_error && text.starts_with(WAITING_FOR_USER), "{text}");
    let asked = |s: &Snapshot| {
        s.inbox
            .iter()
            .find(|i| {
                matches!(&i.kind, InboxKind::Permission { tool, .. }
                if tool == dovetail_api::REMOVE_WORKTREE_PERMISSION)
            })
            .map(|i| i.id)
    };
    h.wait_for("the Permission item", |s| asked(s).is_some());
    assert!(
        wt.exists(),
        "unsaved work is not removed without the user's yes"
    );
    assert!(branch_exists(&repo, "dovetail/wip"));
    assert!(h.snap.agents.iter().any(|a| a.name == "wip"));

    // Asking again does not raise a second item.
    let (text, _) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("wip")),
    ));
    assert!(text.starts_with(WAITING_FOR_USER), "{text}");
    h.drain(Duration::from_millis(300));
    let items = h
        .snap
        .inbox
        .iter()
        .filter(|i| matches!(&i.kind, InboxKind::Permission { .. }))
        .count();
    assert_eq!(items, 1);

    // The user says no: the slice stays and the coordinator is told.
    let item = asked(&h.snap).expect("item");
    h.send(Command::AnswerPermission {
        item,
        answer: PermissionAnswer::Deny { message: None },
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sent_users(&h.fake)
        .iter()
        .any(|t| t.contains("slice wip kept"))
    {
        assert!(Instant::now() < deadline, "{:?}", sent_users(&h.fake));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(wt.exists());
    h.quit();
}

#[test]
fn archive_task_archives_the_task_and_its_live_slice() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
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
    let id = h.snap.tasks[0].id;
    h.send(Command::NewSlice {
        repo: None,
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let archive = Tool::ArchiveTask(ArchiveTask { id });

    let (text, is_error) = result_text(socket_call(&socket, tool("slice-work", archive.clone())));
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );

    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, archive.clone())));
    assert!(
        !is_error && text.starts_with(ARCHIVED) && text.contains("work"),
        "{text}"
    );
    h.wait_for("the task archived and the slice gone", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == id && t.status == TaskStatus::Archived)
            && !s.agents.iter().any(|a| a.name == "work")
    });

    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, archive)));
    assert!(is_error && text.contains("already archived"), "{text}");
    h.quit();
}

// --- reading tasks and decisions (task #20) ---

#[test]
fn list_tasks_and_get_task_read_tasks_slices_and_decisions_for_the_coordinator_only() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    for (title, bucket, p_level) in [
        ("later one", Some(Bucket::Later), Some(PLevel::P0)),
        ("unsorted one", None, None),
        ("next one", Some(Bucket::Next), Some(PLevel::P2)),
        ("urgent next", Some(Bucket::Next), Some(PLevel::P1)),
    ] {
        h.send(Command::CreateTask(NewTask {
            title: title.into(),
            p_level,
            bucket,
            notes: format!("notes for {title}"),
            follow_up_of: None,
            pr: None,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }));
    }
    h.wait_for("the tasks", |s| s.tasks.len() == 4);
    // Task 3 gets a slice, which moves it to Now.
    h.send(Command::NewSlice {
        repo: None,
        task: Some(TaskId(3)),
        name: "work".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let call = |agent: &str, t: Tool| result_text(socket_call(&socket, tool(agent, t)));

    let decide = |text: &str, supersedes: Option<DecisionId>| {
        Tool::LogDecision(LogDecision {
            task: Some(TaskId(3)),
            decision: text.into(),
            why: Some(format!("why {text}")),
            alternatives: vec![Alternative {
                option: "other".into(),
                why_not: Some("worse".into()),
            }],
            supersedes,
        })
    };
    let (text, is_error) = call(COORDINATOR, decide("first", None));
    assert!(!is_error, "{text}");
    let (text, is_error) = call(COORDINATOR, decide("second", Some(DecisionId(1))));
    assert!(!is_error, "{text}");

    let (text, is_error) = call(COORDINATOR, Tool::ListTasks(ListTasks::default()));
    assert!(!is_error, "{text}");
    let ids: Vec<&str> = text
        .lines()
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(ids, ["#3", "#4", "#2", "#1"], "{text}");
    assert!(
        text.contains("#3 [in_progress · now · P2] next one (slice work)"),
        "{text}"
    );

    let (text, _) = call(
        COORDINATOR,
        Tool::ListTasks(ListTasks {
            bucket: Some(BucketFilter::Next),
            ..ListTasks::default()
        }),
    );
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("#4 "), "{text}");

    let (text, is_error) = call(COORDINATOR, Tool::GetTask(GetTask { id: TaskId(3) }));
    assert!(!is_error, "{text}");
    for want in [
        "# #3 next one",
        "notes for next one",
        "- slice work:",
        "- D1 (agent coordinator): first [superseded by D2]",
        "  Why: why first",
        "  Rejected: other, because worse",
        "- D2 (agent coordinator): second",
        "  Supersedes: D1",
    ] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }

    let (text, is_error) = call(COORDINATOR, Tool::GetTask(GetTask { id: TaskId(99) }));
    assert!(is_error && text.contains("no task #99"), "{text}");

    for refused in [
        Tool::ListTasks(ListTasks::default()),
        Tool::GetTask(GetTask { id: TaskId(3) }),
    ] {
        let (text, is_error) = call("slice-work", refused);
        assert!(
            is_error && text.contains("not in this session's tool suites"),
            "{text}"
        );
    }
    h.quit();
}

// --- updating tasks (task #25) ---

fn update(id: i64) -> UpdateTask {
    UpdateTask {
        id: TaskId(id),
        status: None,
        bucket: None,
        p_level: None,
        title: None,
        notes: None,
    }
}

#[test]
fn update_task_changes_only_the_given_fields_and_may_move_into_now() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    for title in ["one", "two"] {
        h.send(Command::CreateTask(NewTask {
            title: title.into(),
            p_level: Some(PLevel::P2),
            bucket: Some(Bucket::Next),
            notes: "old notes".into(),
            follow_up_of: None,
            pr: None,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }));
    }
    h.wait_for("the tasks", |s| s.tasks.len() == 2);
    // Task 2 gets a slice, which moves it to Now.
    h.send(Command::NewSlice {
        repo: None,
        task: Some(TaskId(2)),
        name: "work".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let call = |agent: &str, u: UpdateTask| {
        result_text(socket_call(&socket, tool(agent, Tool::UpdateTask(u))))
    };

    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            status: Some(TaskStatus::Done),
            ..update(1)
        },
    );
    // Status is Dovetail's to set (ADR 0024): the call is refused and nothing changes.
    assert!(is_error, "{text}");
    assert!(text.contains("Dovetail moves a task"), "{text}");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Later),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · later · P2] one");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            p_level: Some(Some(PLevel::P0)),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · later · P0] one");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            p_level: Some(None),
            bucket: Some(BucketFilter::Unsorted),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · unsorted] one");
    let (text, _) = call(
        COORDINATOR,
        UpdateTask {
            title: Some("  renamed ".into()),
            notes: Some("new notes".into()),
            ..update(1)
        },
    );
    assert_eq!(text, "#1 [todo · unsorted] renamed");
    // The TUI sees each change live.
    h.wait_for("the edits in the snapshot", |s| {
        s.tasks.iter().any(|t| {
            t.id == TaskId(1)
                && t.title == "renamed"
                && t.notes == "new notes"
                && t.bucket.is_none()
                && t.p_level.is_none()
                && t.status == TaskStatus::Todo
        })
    });

    // Into Now is allowed, and the task is flagged as new until the user sees it.
    h.send(Command::UpdateTask {
        id: TaskId(1),
        patch: vec![dovetail_api::TaskChange::Seen],
    });
    h.wait_for("task 1 seen", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == TaskId(1) && !t.coordinator_created)
    });
    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Now),
            ..update(1)
        },
    );
    assert!(!is_error, "{text}");
    assert!(text.starts_with("#1 [todo · now]"), "{text}");
    h.wait_for("task 1 unseen in Now", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == TaskId(1) && t.bucket == Some(Bucket::Now) && t.coordinator_created)
    });
    // Already in Now: a later update does not flag it again once the user has looked.
    h.send(Command::UpdateTask {
        id: TaskId(1),
        patch: vec![dovetail_api::TaskChange::Seen],
    });
    h.wait_for("task 1 seen again", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == TaskId(1) && !t.coordinator_created)
    });
    let (_, is_error) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Now),
            p_level: Some(Some(PLevel::P1)),
            ..update(1)
        },
    );
    assert!(!is_error);
    h.drain(Duration::from_millis(100));
    assert!(
        h.snap
            .tasks
            .iter()
            .any(|t| t.id == TaskId(1) && !t.coordinator_created),
        "a task already in Now was flagged again"
    );

    // Out of Now is fine, and leaves the slice running.
    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            bucket: Some(BucketFilter::Next),
            ..update(2)
        },
    );
    assert!(!is_error, "{text}");
    assert_eq!(text, "#2 [in_progress · next · P2] two (slice work)");

    let (text, is_error) = call(
        "slice-work",
        UpdateTask {
            status: Some(TaskStatus::Done),
            ..update(2)
        },
    );
    assert!(
        is_error && text.contains("not in this session's tool suites"),
        "{text}"
    );

    let (text, is_error) = call(
        COORDINATOR,
        UpdateTask {
            title: Some("gone".into()),
            ..update(99)
        },
    );
    assert!(is_error && text.contains("no task #99"), "{text}");
    let (text, is_error) = call(COORDINATOR, update(1));
    assert!(is_error && text.contains("at least one field"), "{text}");
    h.quit();
}

// --- native to-do lists (M11, task #12) ---

/// The temp dir standing in for `~/.claude/tasks` (see `Harness::start`).
fn tasks_dir(paths: &Paths) -> PathBuf {
    paths.data.with_file_name("claude-tasks")
}

/// Writes one task file the way Claude Code does.
fn write_item(list: &Path, id: &str, subject: &str, status: &str) {
    std::fs::create_dir_all(list).expect("list dir");
    let text = format!(
        "{{\n  \"id\": \"{id}\",\n  \"subject\": \"{subject}\",\n  \"description\": \"{subject}\",\n  \
         \"status\": \"{status}\",\n  \"blocks\": [],\n  \"blockedBy\": []\n}}"
    );
    std::fs::write(list.join(format!("{id}.json")), text).expect("task file");
}

fn todo(s: &Snapshot, id: &str) -> Option<dovetail_api::TodoItem> {
    s.todos.iter().find(|t| t.id == id).cloned()
}

fn owner(s: &Snapshot, id: &str) -> Option<String> {
    todo(s, id).and_then(|t| t.created_by).map(|a| a.0)
}

#[test]
fn todo_items_attach_to_the_agent_that_created_them_and_survive_a_restart() {
    let fake = fixture("todo_tools");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    // What Claude Code leaves on disk after the recorded turn, plus an item nobody created in
    // the stream (an external edit).
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "completed");
    write_item(&list, "2", "beta", "pending");
    write_item(&list, "3", "outside", "pending");
    prompt(&h, "make two to-dos");
    h.wait_for("both items attributed", |s| {
        owner(s, "1").as_deref() == Some(COORDINATOR)
            && owner(s, "2").as_deref() == Some(COORDINATOR)
    });
    assert_eq!(
        todo(&h.snap, "1").map(|t| t.status),
        Some(dovetail_api::TodoStatus::Completed)
    );
    // No TaskCreate in the stream: it falls back to the session whose list it is.
    assert_eq!(owner(&h.snap, "3").as_deref(), Some(COORDINATOR));
    assert!(
        fake.started_specs()[0]
            .env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CODE_ENABLE_TODO_TOOLS" && v == "1")
    );

    // An edit outside the stream reaches the UI through the watcher.
    write_item(&list, "2", "beta", "in_progress");
    h.wait_for("the external edit", |s| {
        todo(s, "2").map(|t| t.status) == Some(dovetail_api::TodoStatus::InProgress)
    });
    std::fs::remove_file(list.join("3.json")).expect("delete");
    h.wait_for("the deleted item gone", |s| todo(s, "3").is_none());

    // The owners are in state.db: a reopen shows the items where they were.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(owner(&h.snap, "1").as_deref(), Some(COORDINATOR));
    assert_eq!(owner(&h.snap, "2").as_deref(), Some(COORDINATOR));
    h.quit();
}

#[test]
fn a_foreground_subagents_todo_is_its_own() {
    // The recorded turn, with the second TaskCreate (call and result) made inside a subagent.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures/todo_tools.ndjson");
    let text = std::fs::read_to_string(path).expect("fixture");
    let text: String = text
        .lines()
        .map(|line| {
            let line = if line.contains("toolu_01PDiMXF9E6gftnQXY5xYUhf") {
                line.replace(
                    "\"parent_tool_use_id\":null",
                    "\"parent_tool_use_id\":\"toolu_sub\"",
                )
            } else {
                line.to_owned()
            };
            line + "\n"
        })
        .collect();
    let fake = FakeDriver::from_ndjson(&text).expect("fixture");
    let mut h = Harness::start(fake, true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "completed");
    write_item(&list, "2", "beta", "pending");
    prompt(&h, "make two to-dos");
    h.wait_for("the subagent's item", |s| {
        owner(s, "2").as_deref() == Some("coordinator:toolu_sub")
    });
    assert_eq!(owner(&h.snap, "1").as_deref(), Some(COORDINATOR));
    // The stream's attribution is in state.db, not just the list owner's fallback.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(
        owner(&h.snap, "2").as_deref(),
        Some("coordinator:toolu_sub")
    );
    h.quit();
}

#[test]
fn a_pruned_subagents_todo_falls_back_to_the_list_owner() {
    // As above, then the subagent ends, and a second one runs and ends: only the newest finished
    // subagent keeps its node, so the first one's item must not point at a gone agent.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures/todo_tools.ndjson");
    let text = std::fs::read_to_string(path).expect("fixture");
    let session = "f0e55c68-6629-4132-b963-ccb23955bc1a";
    let result = |tool: &str| {
        format!(
            "{{\"dir\":\"out\",\"msg\":{{\"message\":{{\"content\":[{{\"content\":\"ok\",\
             \"tool_use_id\":\"{tool}\",\"type\":\"tool_result\"}}],\"role\":\"user\"}},\
             \"parent_tool_use_id\":null,\"session_id\":\"{session}\",\"type\":\"user\"}},\"t\":6000}}\n"
        )
    };
    let second = format!(
        "{{\"dir\":\"out\",\"msg\":{{\"message\":{{\"content\":[{{\"text\":\"hi\",\"type\":\"text\"}}],\
         \"id\":\"msg_sub2\",\"model\":\"claude-sonnet-5-5\",\"role\":\"assistant\",\"type\":\"message\"}},\
         \"parent_tool_use_id\":\"toolu_sub2\",\"session_id\":\"{session}\",\"type\":\"assistant\"}},\"t\":6000}}\n"
    );
    let mut out = String::new();
    for line in text.lines() {
        if line.contains("\"text\":\"done\"") {
            out += &result("toolu_sub");
            out += &second;
            out += &result("toolu_sub2");
        }
        let line = if line.contains("toolu_01PDiMXF9E6gftnQXY5xYUhf") {
            line.replace(
                "\"parent_tool_use_id\":null",
                "\"parent_tool_use_id\":\"toolu_sub\"",
            )
        } else {
            line.to_owned()
        };
        out += &line;
        out.push('\n');
    }
    let fake = FakeDriver::from_ndjson(&out).expect("fixture");
    let mut h = Harness::start(fake, true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "completed");
    write_item(&list, "2", "beta", "pending");
    prompt(&h, "make two to-dos");
    h.wait_for("the first subagent pruned", |s| {
        !s.agents.iter().any(|a| a.id.0 == "coordinator:toolu_sub")
            && s.agents.iter().any(|a| a.id.0 == "coordinator:toolu_sub2")
    });
    h.wait_for("its item back under the coordinator", |s| {
        owner(s, "2").as_deref() == Some(COORDINATOR)
    });
    // A reopen agrees: the recorded creator is gone, so the list owner shows it.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(owner(&h.snap, "2").as_deref(), Some(COORDINATOR));
    h.quit();
}

#[test]
fn items_with_no_known_creator_sit_under_the_session_whose_list_it_is() {
    // No TaskCreate in the stream at all: the items were put there some other way.
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "by hand", "pending");
    prompt(&h, "hello");
    h.wait_for("the item under the coordinator", |s| {
        owner(s, "1").as_deref() == Some(COORDINATOR)
    });
    h.quit();
}

/// M11: a task file caught mid-write (truncated, empty) keeps its last good version on screen,
/// the rest of the list still updates, and the file is read again once it is whole.
#[test]
fn a_todo_file_caught_mid_write_keeps_its_last_version_until_it_is_whole() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let list = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&list, "1", "alpha", "pending");
    write_item(&list, "2", "beta", "pending");
    prompt(&h, "hello");
    h.wait_for("both items", |s| {
        todo(s, "1").is_some() && todo(s, "2").is_some()
    });

    // Item 2 is rewritten and read half way; item 3 arrives whole in the same window.
    std::fs::write(list.join("2.json"), "{\"id\": \"2\", \"subject\": \"be").expect("truncate");
    write_item(&list, "3", "gamma", "pending");
    h.wait_for("the whole item", |s| todo(s, "3").is_some());
    let kept = todo(&h.snap, "2").expect("the last good version stays");
    assert_eq!(kept.subject, "beta");
    assert_eq!(kept.status, dovetail_api::TodoStatus::Pending);

    // An empty file is the same state (a writer that truncates first).
    std::fs::write(list.join("1.json"), "").expect("empty");
    write_item(&list, "4", "delta", "pending");
    h.wait_for("the next item", |s| todo(s, "4").is_some());
    assert_eq!(todo(&h.snap, "1").map(|t| t.subject), Some("alpha".into()));

    // Once the writer finishes, the new content shows.
    write_item(&list, "2", "beta", "in_progress");
    h.wait_for("the finished write", |s| {
        todo(s, "2").map(|t| t.status) == Some(dovetail_api::TodoStatus::InProgress)
    });
    h.quit();
}

#[test]
fn archiving_a_task_deletes_its_todo_list() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
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
    let id = h.snap.tasks[0].id;
    let list = tasks_dir(&h.paths).join(format!("dovetail-p-task-{}", id.0));
    write_item(&list, "1", "alpha", "pending");
    // The coordinator's list is not the task's and stays.
    let other = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&other, "1", "keep", "pending");
    h.send(Command::NewSlice {
        repo: None,
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice's list read", |s| {
        s.todos.iter().any(|t| t.subject == "alpha")
    });
    h.send(Command::ArchiveTask { id });
    h.wait_for("the list gone", |s| {
        !s.todos.iter().any(|t| t.subject == "alpha")
    });
    assert!(!list.exists());
    assert!(other.join("1.json").exists());
    h.quit();
}

// --- Profiles (task #27, ADR 0009) ---

/// State dirs for a run whose recipe and config are written before the core starts.
fn profile_paths(dir: &Path) -> Paths {
    Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("unused"),
    }
}

fn profiles_config(dir: &Path) -> Config {
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

fn write_recipe(paths: &Paths, text: &str) {
    let file = paths.recipe("p");
    std::fs::create_dir_all(file.parent().expect("parent")).expect("projects dir");
    std::fs::write(file, text).expect("recipe");
}

fn config_dir_env(spec: &dovetail_driver::SessionSpec) -> Option<String> {
    spec.env
        .iter()
        .find(|(k, _)| k == "CLAUDE_CONFIG_DIR")
        .map(|(_, v)| v.clone())
}

fn error_texts(s: &Snapshot) -> Vec<String> {
    s.inbox
        .iter()
        .filter_map(|i| match &i.kind {
            InboxKind::Error { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn wait_started(fake: &FakeDriver, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < count {
        assert!(Instant::now() < deadline, "sessions did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_project_profile_sets_claude_config_dir_and_a_changed_one_is_not_resumed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    let work = dir.path().join("profiles/work");
    write_recipe(&paths, "profile = \"mine\"\n");

    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
    assert_eq!(h.snap.profile.as_deref(), Some("mine"));
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    h.wait_for("the usage limits", |s| s.usage.five_hour.is_some());
    let first = fake.started_specs()[0].clone();
    assert_eq!(
        config_dir_env(&first),
        Some(mine.to_string_lossy().into_owned())
    );
    let _state = h.quit();

    // The recipe now picks another account: the coordinator is not resumed under it.
    write_recipe(&paths, "profile = \"work\"\n");
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
    h.wait_for("the profile notice", |s| {
        error_texts(s)
            .iter()
            .any(|t| t.contains("Claude profile work"))
    });
    assert_eq!(h.snap.profile.as_deref(), Some("work"));
    // Limits are per account: `mine`'s figures are not shown for `work`.
    assert_eq!(h.snap.usage, dovetail_api::UsageLimits::default());
    wait_started(&fake, 1);
    let second = fake.started_specs()[0].clone();
    assert!(!second.resume, "resumed across profiles");
    assert_ne!(second.session_id, first.session_id);
    assert_eq!(
        config_dir_env(&second),
        Some(work.to_string_lossy().into_owned())
    );
    h.wait_for("the fresh-session notice", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text.contains("fresh session")))
    });
    let _state = h.quit();

    // Told once: the next open, under the same profile, resumes and says nothing new.
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths, None, config);
    wait_started(&fake, 1);
    h.drain(Duration::from_millis(200));
    assert!(fake.started_specs()[0].resume);
    let notices = error_texts(&h.snap)
        .iter()
        .filter(|t| t.contains("now runs under Claude profile"))
        .count();
    assert!(notices <= 1, "{:?}", error_texts(&h.snap));
    h.quit();
}

fn profile_errors(s: &Snapshot, needle: &str) -> usize {
    error_texts(s).iter().filter(|t| t.contains(needle)).count()
}

/// Review of PR #26: exactly one inbox error, also for a coordinator that would resume at open,
/// and not again on the next open while it is still unanswered.
#[test]
fn an_unknown_profile_is_one_inbox_error_and_starts_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    write_recipe(&paths, "profile = \"mine\"\n");
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let _state = h.quit();

    write_recipe(&paths, "profile = \"ghost\"\n");
    for _ in 0..2 {
        let fake = fixture("plain_turn");
        let mut h = Harness::start_with(fake.clone(), paths.clone(), None, config.clone());
        h.wait_for("the profile error", |s| profile_errors(s, "\"ghost\"") > 0);
        prompt(&h, "hello");
        h.drain(Duration::from_millis(300));
        assert_eq!(
            profile_errors(&h.snap, "\"ghost\""),
            1,
            "{:?}",
            error_texts(&h.snap)
        );
        assert!(fake.started_specs().is_empty());
        let _state = h.quit();
    }
}

#[test]
fn a_recipe_env_setting_claude_config_dir_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    write_recipe(&paths, "[env]\nCLAUDE_CONFIG_DIR = \"/elsewhere\"\n");
    let fake = fixture("plain_turn");
    let mut h = Harness::start_with(fake.clone(), paths, None, profiles_config(dir.path()));
    h.wait_for("the env error", |s| profile_errors(s, "profile = ") > 0);
    assert!(error_texts(&h.snap)[0].contains("CLAUDE_CONFIG_DIR"));
    prompt(&h, "hello");
    h.drain(Duration::from_millis(300));
    assert!(fake.started_specs().is_empty());
    assert_eq!(profile_errors(&h.snap, "profile = "), 1);
    h.quit();
}

#[test]
fn project_add_with_an_unknown_profile_raises_the_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::start(fixture("plain_turn"), false, profiles_config(dir.path()));
    assert_eq!(profile_errors(&h.snap, "\"ghost\""), 0);
    // `:project add` names it `work`, whose recipe picks a profile config.toml lacks.
    write_recipe_for(&h.paths, "work", "profile = \"ghost\"\n");
    let path = h.repo.clone();
    h.send(Command::RegisterProject { path });
    h.wait_for("the profile error", |s| profile_errors(s, "\"ghost\"") > 0);
    h.drain(Duration::from_millis(200));
    assert_eq!(profile_errors(&h.snap, "\"ghost\""), 1);
    // The recipe was there already: never overwritten by the starter (task #31).
    let text = std::fs::read_to_string(h.paths.recipe("work")).expect("recipe");
    assert_eq!(text, "profile = \"ghost\"\n");
    h.quit();
}

#[test]
fn project_add_writes_a_starter_recipe_and_a_picked_profile_lands_in_it() {
    use dovetail_core::recipe::Recipe;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::start(fixture("plain_turn"), false, profiles_config(dir.path()));
    let path = h.repo.clone();
    h.send(Command::RegisterProject { path });
    h.wait_for("registration", |s| {
        s.warnings.iter().all(|w| w.id.0 != "unregistered")
    });
    let file = h.paths.recipe("work");
    let recipe = Recipe::load(&file).expect("the starter parses");
    assert_eq!(recipe.name.as_deref(), Some("work"));
    assert_eq!(recipe.profile, None, "the profile starts commented out");

    h.send(Command::SetRecipeProfile {
        project: dovetail_api::ProjectId("work".into()),
        profile: "work".into(),
    });
    h.drain(Duration::from_millis(300));
    assert!(h.failures.is_empty(), "{:?}", h.failures);
    let recipe = Recipe::load(&file).expect("still parses");
    assert_eq!(recipe.profile.as_deref(), Some("work"));

    // A profile config.toml lacks is refused, and the file is left alone.
    h.send(Command::SetRecipeProfile {
        project: dovetail_api::ProjectId("work".into()),
        profile: "ghost".into(),
    });
    h.drain(Duration::from_millis(300));
    assert_eq!(h.failures.len(), 1, "{:?}", h.failures);
    assert_eq!(
        Recipe::load(&file).expect("parses").profile.as_deref(),
        Some("work")
    );
    h.quit();
}

#[test]
fn sessions_keep_their_launch_prefix_after_project_add() {
    // ADR 0022: a coordinator launched before `:project add` runs as `new-<pid>.coordinator`
    // until the next start, so slices launched after it must name it so too.
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), false, Config::default());
    let path = h.repo.clone();
    h.send(Command::RegisterProject { path });
    h.wait_for("registration", |s| {
        s.warnings.iter().all(|w| w.id.0 != "unregistered")
    });
    h.send(Command::NewSlice {
        repo: None,
        task: None,
        name: "api".into(),
        brief: "build the endpoint".into(),
    });
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    let prefix = format!("new-{}.", std::process::id());
    wait_sent_containing(
        &fake,
        &format!("The coordinator's session is named `{prefix}coordinator`"),
    );
    let names: Vec<String> = fake
        .started_specs()
        .iter()
        .filter_map(|s| s.name.clone())
        .collect();
    assert!(names.iter().all(|n| n.starts_with(&prefix)), "{names:?}");
    h.quit();
}

fn write_recipe_for(paths: &Paths, project: &str, text: &str) {
    let file = paths.recipe(project);
    std::fs::create_dir_all(file.parent().expect("parent")).expect("projects dir");
    std::fs::write(file, text).expect("recipe");
}

// --- Take-over (M14) ---

/// The transcript folder name Claude Code uses for `cwd`.
fn encoded(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A fake native `claude`: records its arguments, folder and Claude home in `out`, appends one
/// native turn to the session's transcript (under `$CLAUDE_CONFIG_DIR/projects/<folder>`), and
/// exits once `wait` exists (at once without one).
fn fake_claude(out: &Path, folder: &str, wait: Option<&Path>) -> std::ffi::OsString {
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn lock_of(s: &Snapshot, id: &str) -> dovetail_api::SessionLock {
    s.agents
        .iter()
        .find(|a| a.id.0 == id)
        .map(|a| a.lock)
        .unwrap_or_else(|| panic!("no agent {id}"))
}

fn wait_file(path: &Path) {
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

fn wait_sent_shutdown(fake: &FakeDriver) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake.sent().iter().any(|s| matches!(s, Sent::Shutdown(_))) {
        assert!(Instant::now() < deadline, "headless was not stopped");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The hand-off with a live headless coordinator: the lock is taken, headless stops, native
/// `claude` runs with headless's flags, Claude home and folder, and on exit the terminal comes
/// back, the native turn shows in the conversation and headless resumes the same session.
#[test]
fn take_over_stops_headless_runs_native_with_its_flags_and_resumes_headless() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let out = dir.path().join("native");
    std::fs::create_dir_all(&out).expect("out");
    // The coordinator runs in its `home/` folder (ADR 0021), and the fake driver reports that.
    let claude = fake_claude(&out, &encoded(&paths.project_home("p")), None);
    // Held before its `result`: the session is still running when it is taken over.
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![4],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start_native(fake.clone(), paths, None, config, Some(claude));
    prompt(&h, "hello");
    h.wait_for("the reply", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::Assistant))
    });

    h.send(Command::TakeOver {
        agent: AgentId(COORDINATOR.into()),
    });
    h.wait_for("the lock", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::SessionLock::TakenOver
    });
    wait_sent_shutdown(&fake);
    // Native waits for headless to exit.
    h.drain(Duration::from_millis(100));
    assert!(!out.join("argv").exists(), "native ran beside headless");
    fake.resume();

    // The note, then the turn; its tool summary comes last.
    h.wait_for("the native turn", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::ToolSummary && e.text == "Bash")
            })
    });
    assert_eq!(
        h.returned,
        vec![None],
        "the terminal came back once, cleanly"
    );
    let entries = h.entries(COORDINATOR);
    let at = entries
        .iter()
        .position(|(_, t)| t.contains("1 native turn"))
        .expect("note");
    assert_eq!(
        entries[at..at + 4].to_vec(),
        vec![
            (
                EntryKind::Notice,
                "Back from the native terminal: 1 native turn, shown below.".into()
            ),
            (EntryKind::User, "native hello".into()),
            (EntryKind::Assistant, "native reply".into()),
            (EntryKind::ToolSummary, "Bash".into()),
        ]
    );

    // The same flags as headless, minus the stream ones, on the same session.
    let headless = fake.started_specs()[0].clone();
    let argv: Vec<String> = read(&out.join("argv")).lines().map(String::from).collect();
    assert_eq!(argv, headless.native_args());
    let resume = argv.iter().position(|a| a == "--resume").expect("--resume");
    assert_eq!(argv[resume + 1], headless.session_id);
    for flag in [
        "--settings",
        "--mcp-config",
        "--allowedTools",
        "--permission-mode",
    ] {
        assert!(argv.contains(&flag.to_string()), "{flag} missing: {argv:?}");
    }
    assert!(!argv.contains(&"-p".to_string()));
    assert_eq!(read(&out.join("config_dir")), mine.to_string_lossy());
    assert_eq!(read(&out.join("agent")), COORDINATOR);
    // Native runs where headless did: the coordinator's `home/` (ADR 0021).
    assert_eq!(
        read(&out.join("cwd")).trim(),
        h.paths
            .project_home("p")
            .canonicalize()
            .expect("home")
            .to_string_lossy()
    );

    // Headless again, resuming the same session.
    wait_started(&fake, 2);
    let again = fake.started_specs()[1].clone();
    assert!(again.resume);
    assert_eq!(again.session_id, headless.session_id);
    h.wait_for("the lock released", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::SessionLock::Headless
    });
    fake.resume();
    h.quit();
}

/// Stores an agent row before the core opens.
fn seed(paths: &Paths, row: dovetail_core::db::AgentRow) {
    let db = dovetail_core::db::Db::open(paths, Some("p")).expect("db");
    db.upsert_agent(row).wait().expect("agent");
}

fn session_row(
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

/// While a slice is taken over nothing starts it headless, archives it or removes its
/// worktree; native runs in the worktree, and exiting it leaves the worktree and the slice
/// alone and resumes headless there.
#[test]
fn while_taken_over_headless_never_starts_and_exiting_native_keeps_the_worktree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let worktree = paths.worktrees("p").join("s4");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let worktree = worktree.canonicalize().expect("canonical");
    let folder = encoded(&worktree);
    let transcript = mine
        .join("projects")
        .join(&folder)
        .join("slice-s4-session.jsonl");
    seed(
        &paths,
        session_row(
            "slice-s4",
            AgentKind::Slice,
            Some(worktree.clone()),
            &mine,
            transcript,
            None,
        ),
    );
    let out = dir.path().join("native");
    std::fs::create_dir_all(&out).expect("out");
    let release = out.join("exit-now");
    let claude = fake_claude(&out, &folder, Some(&release));
    let fake = fixture("plain_turn");
    let slice = AgentId("slice-s4".into());
    let mut h = Harness::start_native(fake.clone(), paths, None, config, Some(claude));

    h.send(Command::TakeOver {
        agent: slice.clone(),
    });
    wait_file(&out.join("started"));
    h.wait_for("the lock", |s| {
        lock_of(s, "slice-s4") == dovetail_api::SessionLock::TakenOver
    });
    assert_eq!(read(&out.join("cwd")).trim(), worktree.to_string_lossy());

    // Every way in is refused while native runs.
    h.send(Command::Prompt {
        agent: slice.clone(),
        text: "are you there?".into(),
        attachments: Vec::new(),
        effort: None,
    });
    h.send(Command::CloseSlice {
        agent: slice.clone(),
    });
    h.send(Command::RemoveWorktree {
        agent: slice.clone(),
        confirmed: true,
    });
    h.send(Command::TakeOver {
        agent: slice.clone(),
    });
    h.drain(Duration::from_millis(300));
    assert!(fake.started_specs().is_empty(), "headless started");
    assert_eq!(h.failures.len(), 3, "{:?}", h.failures);
    assert!(h.failures[0].contains("taken over in a native terminal: write to it there"));
    assert!(h.failures[1].contains("archive it once that claude has exited"));
    assert!(h.failures[2].contains("archive it once that claude has exited"));
    assert_eq!(
        h.returned,
        vec![Some("the terminal is already lent out".into())]
    );
    assert!(worktree.exists());

    std::fs::write(&release, "").expect("let native exit");
    wait_started(&fake, 1);
    let resumed = fake.started_specs()[0].clone();
    assert!(resumed.resume);
    assert_eq!(resumed.cwd, worktree);
    assert_eq!(resumed.session_id, "slice-s4-session");
    h.wait_for("the lock released", |s| {
        lock_of(s, "slice-s4") == dovetail_api::SessionLock::Headless
    });
    h.drain(Duration::from_millis(200));
    assert!(worktree.exists(), "exiting native removed the worktree");
    assert!(
        h.snap.agents.iter().any(|a| a.id == slice),
        "slice archived"
    );
    h.quit();
}

/// Opens a project whose coordinator was taken over when Dovetail stopped: its transcript has
/// one line from before the take-over and one native turn after, and its lock names `pid`.
fn open_after_crash(pid: u32, ticks: Option<u64>) -> (Harness, FakeDriver, TempDir) {
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

/// A lock left by a crash whose native run is gone is released at open, with the native turn
/// shown, and the coordinator resumes as usual.
#[test]
fn a_crash_lock_whose_native_is_gone_is_released_at_open() {
    let mut child = std::process::Command::new("true").spawn().expect("true");
    let dead = child.id();
    child.wait().expect("reaped");
    let (mut h, fake, _dir) = open_after_crash(dead, Some(1));
    h.wait_for("the release", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::SessionLock::Headless
    });
    wait_started(&fake, 1);
    assert!(fake.started_specs()[0].resume);
    h.wait_for("the native turn", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::User && e.text == "typed while Dovetail was down")
            })
    });
    let entries = h.entries(COORDINATOR);
    assert!(
        entries
            .iter()
            .any(|(k, t)| *k == EntryKind::Notice && t.contains("lock is released: 1 native turn")),
        "{entries:?}"
    );
    h.quit();
}

/// A lock left by a crash whose native run still runs is kept: Needs you says so and nothing
/// starts headless, a message included; once native exits, the next message releases it.
#[test]
fn a_crash_lock_whose_native_still_runs_holds_until_it_exits() {
    use dovetail_core::takeover::{Proc, Processes};
    let mut native = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("sleep");
    let pid = native.id();
    let ticks = Proc.start_ticks(pid).expect("stat");
    let (mut h, fake, _dir) = open_after_crash(pid, ticks);
    h.wait_for("the still-taken-over item", |s| {
        error_texts(s)
            .iter()
            .any(|t| t.contains("is still taken over") && t.contains(&pid.to_string()))
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(
        lock_of(&h.snap, COORDINATOR),
        dovetail_api::SessionLock::TakenOver
    );
    assert!(fake.started_specs().is_empty(), "headless beside native");
    prompt(&h, "hello");
    h.drain(Duration::from_millis(200));
    assert!(fake.started_specs().is_empty(), "a prompt started headless");
    assert!(
        h.failures.iter().any(|f| f.contains("taken over")),
        "{:?}",
        h.failures
    );

    native.kill().expect("kill");
    native.wait().expect("reaped");
    prompt(&h, "hello again");
    wait_started(&fake, 1);
    h.wait_for("the release", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::SessionLock::Headless
    });
    h.quit();
}

/// Processes that cannot be checked, as when `/proc` is unreadable.
struct Blind;

impl dovetail_core::takeover::Processes for Blind {
    fn start_ticks(&self, _: u32) -> std::io::Result<Option<u64>> {
        Err(std::io::Error::other("no /proc here"))
    }
    fn find_native(&self, _: &str) -> std::io::Result<Option<u32>> {
        Err(std::io::Error::other("no /proc here"))
    }
}

/// A crash lock that cannot be checked asks the user; `y` goes through the same release as
/// every other path: the lock goes, the coordinator resumes, and the note held for it while it
/// was locked is delivered.
#[test]
fn y_on_an_undecidable_crash_lock_releases_it_and_delivers_the_held_notes() {
    use dovetail_api::RELEASE_TAKEOVER_PERMISSION;
    use dovetail_core::takeover::TakeoverLock;
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let lock = TakeoverLock {
        since_ms: 1,
        pid: None,
        start_ticks: None,
        transcript_offset: None,
    };
    seed(
        &paths,
        session_row(
            COORDINATOR,
            AgentKind::Coordinator,
            None,
            &mine,
            dovetail_core::session::launch::transcript_path(
                &mine,
                &paths.project_home("p"),
                &format!("{COORDINATOR}-session"),
            ),
            Some(lock),
        ),
    );
    let db = dovetail_core::db::Db::open(&paths, Some("p")).expect("db");
    db.add_note("slice tryit ready".into())
        .wait()
        .expect("note");
    drop(db);
    let fake = fixture("plain_turn");
    let mut h = Harness::start_full(
        fake.clone(),
        paths,
        None,
        config,
        None,
        Some(Arc::new(Blind)),
    );
    h.wait_for("the release question", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == RELEASE_TAKEOVER_PERMISSION)
        })
    });
    h.drain(Duration::from_millis(200));
    assert!(fake.started_specs().is_empty(), "started while undecided");
    let item = h
        .snap
        .inbox
        .iter()
        .find(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == RELEASE_TAKEOVER_PERMISSION))
        .map(|i| i.id)
        .expect("item");
    h.send(Command::AnswerPermission {
        item,
        answer: PermissionAnswer::Allow,
    });
    wait_started(&fake, 1);
    wait_sent(&fake, "slice tryit ready");
    h.wait_for("the lock released", |s| {
        lock_of(s, COORDINATOR) == dovetail_api::SessionLock::Headless
    });
    h.quit();
}

/// `core_ask_user` from native returns at once and leaves its question in Needs you. Answering it
/// while native still has the session is refused and the question stays; once headless is
/// back the answer reaches the session.
#[test]
fn ask_user_from_native_returns_at_once_and_is_answered_once_headless_is_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = profile_paths(dir.path());
    let config = profiles_config(dir.path());
    let mine = dir.path().join("profiles/mine");
    write_recipe(&paths, "profile = \"mine\"\n");
    let worktree = paths.worktrees("p").join("s4");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let worktree = worktree.canonicalize().expect("canonical");
    let folder = encoded(&worktree);
    seed(
        &paths,
        session_row(
            "slice-s4",
            AgentKind::Slice,
            Some(worktree.clone()),
            &mine,
            mine.join("projects")
                .join(&folder)
                .join("slice-s4-session.jsonl"),
            None,
        ),
    );
    let out = dir.path().join("native");
    std::fs::create_dir_all(&out).expect("out");
    let release = out.join("exit-now");
    let claude = fake_claude(&out, &folder, Some(&release));
    let fake = fixture("plain_turn");
    let slice = AgentId("slice-s4".into());
    let mut h = Harness::start_native(fake.clone(), paths, None, config, Some(claude));
    h.send(Command::TakeOver {
        agent: slice.clone(),
    });
    wait_file(&out.join("started"));

    // What native's MCP bridge would send: it returns at once.
    let asked = Instant::now();
    let (text, is_error) = result_text(socket_call(
        &h.socket(),
        tool(
            "slice-s4",
            Tool::AskUser(AskUser {
                question: "Which branch?".into(),
            }),
        ),
    ));
    assert!(!is_error, "{text}");
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "core_ask_user waited"
    );
    h.wait_for("the question", |s| !open_questions(s).is_empty());
    let question = open_questions(&h.snap)[0];

    h.send(Command::AnswerQuestion {
        item: question,
        text: "main".into(),
    });
    h.drain(Duration::from_millis(200));
    assert!(
        h.failures.iter().any(|f| f.contains("taken over")),
        "{:?}",
        h.failures
    );
    assert_eq!(
        open_questions(&h.snap),
        vec![question],
        "the question was lost"
    );

    std::fs::write(&release, "").expect("let native exit");
    wait_started(&fake, 1);
    h.send(Command::AnswerQuestion {
        item: question,
        text: "main".into(),
    });
    wait_sent(&fake, "Answer to your question 'Which branch?': main");
    h.wait_for("the question answered", |s| open_questions(s).is_empty());
    h.quit();
}

// --- maps_draw (M15) ---

fn draw(h: &Harness, map: serde_json::Value) -> (String, bool) {
    result_text(socket_call(
        &h.socket(),
        tool(COORDINATOR, Tool::DrawMap(DrawMap(map))),
    ))
}

fn checkout_map(title: &str) -> serde_json::Value {
    serde_json::json!({
        "map": { "title": title, "type": "structure" },
        "nodes": [
            { "id": "api", "kind": "endpoint", "label": "POST /checkout", "description": "Takes the order." },
            { "id": "orders", "kind": "data", "label": "orders", "description": "Orders table." }
        ],
        "connections": [ { "from": "api", "to": "orders", "label": "writes" } ]
    })
}

fn login_lanes(lane_node: &str) -> serde_json::Value {
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
fn load_map(h: &mut Harness, id: &str, version: Option<u32>) -> Result<Map, String> {
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

#[test]
fn draw_map_stores_a_map_on_the_shelf_and_loads_it() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let (text, is_error) = draw(&h, checkout_map("Checkout"));
    assert!(!is_error, "{text}");
    assert!(
        text.starts_with(r#"{"id":"checkout","version":1}"#),
        "{text}"
    );
    h.wait_for("the shelf entry", |s| s.maps.len() == 1);
    let shelf = &h.snap.maps[0];
    assert_eq!(shelf.kind, MapKind::Structure);
    assert_eq!(shelf.drawn_by, Some(AgentId(COORDINATOR.into())));
    assert_eq!(shelf.task, None, "the coordinator has no task");
    let map = load_map(&mut h, "checkout", None).expect("loads");
    assert_eq!((map.version, map.content.nodes.len()), (1, 2));
    assert!(load_map(&mut h, "nope", None).is_err());
    // A second map with the same title gets its own id.
    let (text, _) = draw(&h, checkout_map("Checkout"));
    assert!(
        text.starts_with(r#"{"id":"checkout-2","version":1}"#),
        "{text}"
    );
    h.wait_for("both maps", |s| s.maps.len() == 2);

    // The shelf survives a restart.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(h.snap.maps.len(), 2);
    h.quit();
}

#[test]
fn draw_map_refuses_an_invalid_map_with_every_problem() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let mut map = checkout_map("Checkout");
    map["nodes"][1]["kind"] = serde_json::json!("table");
    map["connections"][0]["to"] = serde_json::json!("ordrs");
    let (text, is_error) = draw(&h, map);
    assert!(is_error);
    assert!(
        text.starts_with("The map was not saved: 2 problems"),
        "{text}"
    );
    assert!(
        text.contains(r#"kind "table" is not a node kind"#),
        "{text}"
    );
    assert!(text.contains(r#"no node has id "ordrs""#), "{text}");
    // A misspelt field is a schema error, not silently dropped.
    let mut map = checkout_map("Checkout");
    map["nodez"] = serde_json::json!([]);
    let (text, is_error) = draw(&h, map);
    assert!(is_error && text.contains("nodez"), "{text}");
    h.drain(Duration::from_millis(100));
    assert!(h.snap.maps.is_empty());
    h.quit();
}

#[test]
fn draw_map_with_an_existing_id_adds_a_version() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    draw(&h, checkout_map("Checkout"));
    let mut redraw = checkout_map("Checkout, with Stripe");
    redraw["map"]["id"] = serde_json::json!("checkout");
    let (text, is_error) = draw(&h, redraw);
    assert!(!is_error, "{text}");
    assert!(
        text.starts_with(r#"{"id":"checkout","version":2}"#),
        "{text}"
    );
    h.wait_for("version 2", |s| {
        s.maps.first().is_some_and(|m| m.version == 2)
    });
    assert_eq!(h.snap.maps.len(), 1);
    assert_eq!(h.snap.maps[0].title, "Checkout, with Stripe");
    // The earlier version is kept.
    let first = load_map(&mut h, "checkout", Some(1)).expect("version 1");
    assert_eq!(first.content.title, "Checkout");
    assert_eq!(
        load_map(&mut h, "checkout", None).expect("latest").version,
        2
    );
    assert!(load_map(&mut h, "checkout", Some(3)).is_err());
    // A map keeps its type.
    let mut lanes = login_lanes("api");
    lanes["map"]["id"] = serde_json::json!("checkout");
    let (text, is_error) = draw(&h, lanes);
    assert!(is_error && text.contains("a map keeps its type"), "{text}");
    h.quit();
}

#[test]
fn draw_map_checks_lane_links_against_structure_maps() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let (text, is_error) = draw(&h, login_lanes("api"));
    assert!(is_error, "{text}");
    assert!(
        text.contains(r#"lanes[1] ("web"): "node" is "api", but no structure map"#),
        "{text}"
    );
    draw(&h, checkout_map("Checkout"));
    let (text, is_error) = draw(&h, login_lanes("api"));
    assert!(!is_error, "{text}");
    h.wait_for("both maps", |s| s.maps.len() == 2);
    // Most recently drawn first.
    assert_eq!(h.snap.maps[0].kind, MapKind::Swimlane);
    h.quit();
}

/// The local date of `days_ago` days back, as journal files are named (ADR 0012).
fn local_date(days_ago: i64) -> String {
    let now = jiff::Zoned::now();
    now.date()
        .checked_sub(jiff::Span::new().days(days_ago))
        .expect("a date")
        .to_string()
}

fn journal(s: &Snapshot) -> dovetail_api::JournalState {
    s.journal
        .clone()
        .expect("a registered project has a journal")
}

#[test]
fn eod_asks_the_coordinator_and_writes_its_reply_and_the_facts() {
    let lines = [
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"first"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"one","total_cost_usd":0.01}"#,
        r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
        r#"{"type":"user","message":{"role":"user","content":"eod"},"parent_tool_use_id":null,"isReplay":true}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Open: PR 20 waits on review."}]},"parent_tool_use_id":null}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"Open: PR 20 waits on review.","total_cost_usd":0.02}"#,
    ];
    let fake = FakeDriver::from_ndjson(&lines.join("\n"))
        .expect("fixture")
        .with_options(ReplayOptions {
            pause_before: vec![4],
            ..ReplayOptions::default()
        });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    assert_eq!(journal(&h.snap).latest, None);
    h.send(Command::CreateTask(NewTask {
        title: "ship the journal".into(),
        p_level: None,
        bucket: Some(Bucket::Now),
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: Default::default(),
        criterion: None,
    }));
    prompt(&h, "first");
    h.wait_for("the first reply", |s| finished_turns(s) == 1);

    h.send(Command::EndOfDay);
    h.wait_for("the handoff request", |s| {
        journal(s).closing
            && s.conversations[&AgentId(COORDINATOR.into())]
                .entries
                .iter()
                .any(|e| e.kind == EntryKind::Notice && e.text.contains("closing the day"))
    });
    fake.resume();
    let today = local_date(0);
    h.wait_for("the journal", |s| {
        let j = journal(s);
        !j.closing && j.latest.as_deref() == Some(today.as_str())
    });
    let path = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{today}.md"));
    let text = std::fs::read_to_string(&path).expect("the journal file");
    assert!(text.starts_with(&format!("# {today}\n")), "{text}");
    assert!(
        text.contains("## Handoff\n\nOpen: PR 20 waits on review.\n"),
        "{text}"
    );
    assert!(text.contains("## Facts"), "{text}");
    assert!(text.contains("**Now**\n\n- #1 ship the journal"), "{text}");
    h.quit();
}

/// Two turns of the coordinator: "first", then a handoff reply. `pause_before` holds the replay
/// before that line of the script.
fn fresh_fixture(pause_before: usize) -> FakeDriver {
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
fn no_permission_item(s: &Snapshot) -> bool {
    s.inbox
        .iter()
        .all(|i| !matches!(i.kind, InboxKind::Permission { .. }))
}

#[test]
fn fresh_appends_the_handoff_and_seeds_a_new_coordinator_with_no_item() {
    let fake = fresh_fixture(4);
    let mut h = Harness::start(fake.clone(), true, Config::default());
    // A journal file for today already exists: `:fresh` adds to it.
    let dir = h.paths.data.join("projects/p/journal");
    std::fs::create_dir_all(&dir).expect("journal dir");
    let today = local_date(0);
    let path = dir.join(format!("{today}.md"));
    std::fs::write(&path, "Hand-written notes\n").expect("a day's file");
    h.send(Command::CreateTask(NewTask {
        title: "ship the journal".into(),
        p_level: None,
        bucket: Some(Bucket::Now),
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: Default::default(),
        criterion: None,
    }));
    prompt(&h, "first");
    h.wait_for("the first reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();

    h.send(Command::Handoff);
    h.wait_for("the handoff request", |s| journal(s).closing);
    fake.resume();
    wait_started(&fake, 2);
    h.wait_for("the journal block", |s| !journal(s).closing);

    // The day's file was added to, not replaced.
    let text = std::fs::read_to_string(&path).expect("the journal file");
    assert!(text.starts_with("Hand-written notes\n\n---\n\n"), "{text}");
    assert!(text.contains("## Handoff (fresh start "), "{text}");
    assert!(text.contains("Open: PR 20 waits on review."), "{text}");
    assert!(text.contains("**Now**\n\n- #1 ship the journal"), "{text}");

    // A new session (not `--resume`), seeded with the handoff and the open tasks.
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 2);
    assert!(!specs[1].resume);
    assert_ne!(specs[1].session_id, old);
    let seed = fake
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::User(text) => Some(text),
            _ => None,
        })
        .find(|t| t.contains("This is a fresh coordinator session, started at"))
        .expect("the seed prompt");
    assert!(seed.contains("Open: PR 20 waits on review."), "{seed}");
    assert!(seed.contains("# Open tasks"), "{seed}");
    assert!(seed.contains("#1 ship the journal"), "{seed}");
    assert!(no_permission_item(&h.snap));
    h.quit();
}

#[test]
fn fresh_session_starts_a_new_coordinator_with_no_seed_and_no_item() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "first");
    h.wait_for("the first reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();
    let users_before = fake
        .sent()
        .into_iter()
        .filter(|s| matches!(s, Sent::User(_)))
        .count();

    h.send(Command::FreshSession);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < 2 {
        assert!(Instant::now() < deadline, "the new session never started");
        std::thread::sleep(Duration::from_millis(10));
    }

    // A new session, no handoff written, and no first message.
    let specs = fake.started_specs();
    assert!(!specs[1].resume);
    assert_ne!(specs[1].session_id, old);
    let users_after = fake
        .sent()
        .into_iter()
        .filter(|s| matches!(s, Sent::User(_)))
        .count();
    assert_eq!(
        users_before, users_after,
        "nothing was sent to the new session"
    );
    assert!(!journal(&h.snap).closing);
    let file = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{}.md", local_date(0)));
    assert!(!file.exists(), "nothing is written to the journal");
    assert!(no_permission_item(&h.snap));
    h.quit();
}

#[test]
fn fresh_is_refused_while_a_turn_runs() {
    // Held before the `result`: the first turn is still running.
    let fake = fresh_fixture(3);
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "first");
    h.wait_for("the reply text", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|c| {
                c.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Assistant && e.text == "one")
            })
    });
    h.send(Command::Handoff);
    h.drain(Duration::from_millis(300));
    assert_eq!(
        h.failures,
        vec!["turn running; try again when it finishes".to_owned()]
    );
    assert!(!journal(&h.snap).closing);
    assert_eq!(fake.started_specs().len(), 1);
    let file = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{}.md", local_date(0)));
    assert!(!file.exists());
    fake.resume();
    h.quit();
}

#[test]
fn eod_with_a_coordinator_that_never_ran_writes_the_facts_alone() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::EndOfDay);
    let today = local_date(0);
    h.wait_for("the journal", |s| {
        journal(s).latest.as_deref() == Some(today.as_str())
    });
    let path = h
        .paths
        .data
        .join("projects/p/journal")
        .join(format!("{today}.md"));
    let text = std::fs::read_to_string(&path).expect("the journal file");
    assert!(
        text.contains("_No handoff from the coordinator: the coordinator has not run yet._"),
        "{text}"
    );
    assert!(h.fake.started_specs().is_empty(), "no session was started");
    h.quit();
}

#[test]
fn an_unregistered_folder_has_no_journal() {
    let mut h = Harness::start(fixture("plain_turn"), false, Config::default());
    assert_eq!(h.snap.journal, None);
    h.send(Command::EndOfDay);
    h.drain(Duration::from_millis(200));
    assert_eq!(h.snap.journal, None);
    h.quit();
}

#[test]
fn the_first_open_of_a_new_day_starts_a_fresh_coordinator_with_no_item() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();
    let paths = h.paths.clone();
    let _state = h.quit();

    // Yesterday: the conversation happened then, and the coordinator wrote its handoff by hand.
    let db = rusqlite::Connection::open(paths.state_db("p")).expect("state db");
    db.execute(
        "UPDATE conversation_entries SET at_ms = at_ms - 86400000 * 2",
        [],
    )
    .expect("backdate");
    drop(db);
    let dir = paths.data.join("projects/p/journal");
    std::fs::create_dir_all(&dir).expect("journal dir");
    let yesterday = local_date(1);
    std::fs::write(
        dir.join(format!("{yesterday}.md")),
        "# Handoff\n\nFirst thing: merge PR 20.\n",
    )
    .expect("handoff");

    let fake = fixture("plain_turn");
    let h = Harness::start_on(fake.clone(), paths.clone());
    wait_started(&fake, 1);
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 1);
    assert!(!specs[0].resume, "a new session, not --resume");
    assert_ne!(specs[0].session_id, old);
    let seed = fake
        .sent()
        .into_iter()
        .find_map(|s| match s {
            Sent::User(text) => Some(text),
            _ => None,
        })
        .expect("the seed prompt");
    assert!(seed.contains("First thing: merge PR 20."), "{seed}");
    assert!(seed.contains("# Open tasks"), "{seed}");

    assert!(no_permission_item(&h.snap));
    h.quit();

    // The day is not new any more: the next open resumes as before, with no item.
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.drain(Duration::from_millis(300));
    assert!(no_permission_item(&h.snap));
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 1);
    assert!(specs[0].resume);
    h.quit();
}

/// Go-back items stored by builds before task #38 are cleared at the next open, quietly, with
/// their own Activity line (not the generic "request expired").
#[test]
fn legacy_go_back_items_in_an_old_state_db_expire_quietly() {
    let h = Harness::start(fixture("plain_turn"), true, Config::default());
    let paths = h.paths.clone();
    let _state = h.quit();

    let db = rusqlite::Connection::open(paths.state_db("p")).expect("state db");
    let legacy = [
        ("Keep yesterday's coordinator", Some("/old/yesterday.jsonl")),
        ("Fresh coordinator started", None),
    ];
    for (n, (tool, transcript)) in legacy.iter().enumerate() {
        let payload = serde_json::to_string(&InboxKind::Permission {
            tool: (*tool).into(),
            input: serde_json::json!({
                "journal": "2026-11-10",
                "session_id": "old-session",
                "transcript_path": transcript,
            }),
            reason: None,
            suggestions: Vec::new(),
        })
        .expect("payload");
        db.execute(
            "INSERT INTO inbox_items (id, agent_id, kind, payload, blocking, created_ms) \
             VALUES (?1, 'coordinator', 'permission', ?2, 1, 1)",
            rusqlite::params![9_000 + n as i64, payload],
        )
        .expect("legacy item");
    }
    drop(db);

    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("both legacy items cleared", |s| {
        s.activity
            .iter()
            .filter(|a| {
                a.text
                    .contains("an old 'go back to the previous coordinator' offer")
            })
            .count()
            == 2
    });
    let texts: Vec<&str> = h.snap.activity.iter().map(|a| a.text.as_str()).collect();
    assert!(
        texts.contains(
            &"an old 'go back to the previous coordinator' offer was cleared; its transcript \
              is kept at /old/yesterday.jsonl"
        ),
        "{texts:?}"
    );
    assert!(
        texts.contains(
            &"an old 'go back to the previous coordinator' offer was cleared; its transcript \
              is kept"
        ),
        "{texts:?}"
    );
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("request expired at restart")),
        "{texts:?}"
    );
    assert!(
        h.snap
            .inbox
            .iter()
            .all(|i| !matches!(i.kind, InboxKind::Permission { .. }))
    );
    h.quit();
}

#[test]
fn a_day_that_was_never_closed_keeps_its_coordinator_and_says_so() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    prompt(&h, "hello");
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    let old = fake.started_specs()[0].session_id.clone();
    let paths = h.paths.clone();
    let _state = h.quit();

    // The coordinator was used two days ago and never closed; the newest journal is older.
    let db = rusqlite::Connection::open(paths.state_db("p")).expect("state db");
    db.execute(
        "UPDATE conversation_entries SET at_ms = at_ms - 86400000 * 2",
        [],
    )
    .expect("backdate");
    drop(db);
    let dir = paths.data.join("projects/p/journal");
    std::fs::create_dir_all(&dir).expect("journal dir");
    let file = dir.join(format!("{}.md", local_date(4)));
    std::fs::write(&file, "# Handoff\n\nThe day before.\n").expect("old handoff");
    std::fs::File::options()
        .write(true)
        .open(&file)
        .and_then(|f| {
            f.set_modified(std::time::SystemTime::now() - Duration::from_secs(4 * 86_400))
        })
        .expect("backdate the file");
    let used_on = jiff::Zoned::now()
        .checked_sub(jiff::Span::new().days(2))
        .expect("date")
        .strftime("%A")
        .to_string();

    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    let unclosed = format!("{used_on} wasn't closed; kept that coordinator. `:eod` to close it.");
    h.wait_for("the unclosed-day item", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(&i.kind, InboxKind::Error { text } if *text == unclosed))
    });
    // The same session resumes, with no fresh seed and no item.
    wait_started(&fake, 1);
    let specs = fake.started_specs();
    assert_eq!(specs.len(), 1);
    assert!(specs[0].resume);
    assert_eq!(specs[0].session_id, old);
    assert!(no_permission_item(&h.snap));
    h.quit();
}

// --- :upgrade (ADR 0014) ---

mod upgrade {
    use std::sync::atomic::AtomicBool;

    use dovetail_api::UPGRADE_READY_PERMISSION;
    use dovetail_core::service::UpgradeOptions;
    use dovetail_core::upgrade::{Call, Exec, ExecError};

    use super::*;

    /// Plays git and cargo: a worktree add makes the folder, cargo writes a binary into
    /// `CARGO_TARGET_DIR`, `rev-parse` answers `abc1234`.
    struct FakeExec {
        fail_cargo: bool,
    }

    impl Exec for FakeExec {
        fn run(&self, call: &Call, _cancel: &AtomicBool) -> Result<String, ExecError> {
            if call.program == "cargo" {
                if self.fail_cargo {
                    if let Some(log) = &call.log {
                        std::fs::write(log, "error[E0432]: unresolved import\n").unwrap();
                    }
                    return Err(ExecError::Failed {
                        program: "cargo".into(),
                        code: Some(101),
                    });
                }
                let target = call.env.iter().find(|(k, _)| k == "CARGO_TARGET_DIR");
                let target = PathBuf::from(&target.expect("target dir").1);
                std::fs::create_dir_all(target.join("release")).unwrap();
                std::fs::write(target.join("release/dovetail"), "new binary").unwrap();
            }
            if call.args.starts_with(&["worktree".into(), "add".into()]) {
                std::fs::create_dir_all(&call.args[3]).unwrap();
            }
            Ok(if call.args.first().is_some_and(|a| a == "rev-parse") {
                "abc1234\n".into()
            } else {
                String::new()
            })
        }
    }

    struct Upgrading {
        h: Harness,
        /// Holds the fake installed binary.
        _install: TempDir,
    }

    impl Upgrading {
        fn start(driver: FakeDriver, fail_cargo: bool) -> Upgrading {
            let install_dir = tempfile::tempdir().expect("tempdir");
            let install = install_dir.path().join("dovetail");
            std::fs::write(&install, "old binary").unwrap();
            UPGRADE.with(|u| {
                *u.borrow_mut() = Some(UpgradeOptions {
                    exec: Arc::new(FakeExec { fail_cargo }),
                    install,
                });
            });
            let h = Harness::start(driver, true, Config::default());
            // A Dovetail checkout, for the build's first check.
            std::fs::create_dir_all(h.repo.join("crates/dovetail")).unwrap();
            std::fs::write(h.repo.join("crates/dovetail/Cargo.toml"), "").unwrap();
            Upgrading {
                h,
                _install: install_dir,
            }
        }

        fn ready_item(&self) -> Option<dovetail_api::InboxItem> {
            self.h.snap.inbox.iter().find(|i| is_ready(i)).cloned()
        }

        fn wait_for_item(&mut self) -> dovetail_api::InboxItem {
            self.h
                .wait_for("Upgrade ready", |s| s.inbox.iter().any(is_ready));
            self.ready_item().expect("the item")
        }

        fn answer(&self, item: &dovetail_api::InboxItem, answer: PermissionAnswer) {
            self.h.send(Command::AnswerPermission {
                item: item.id,
                answer,
            });
        }

        /// Reads until the UI would be told to quit.
        fn wait_for_restart_event(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let left = deadline
                    .checked_duration_since(Instant::now())
                    .expect("timed out waiting for UpgradeRestarting");
                match self.h.link.messages.recv_timeout(left) {
                    Ok(ServerMsg::Event {
                        event: Event::UpgradeRestarting,
                        ..
                    }) => return,
                    Ok(ServerMsg::Event { seq, event }) => self.h.fold(seq, event),
                    Ok(_) => {}
                    Err(_) => panic!("timed out waiting for UpgradeRestarting"),
                }
            }
        }
    }

    fn is_ready(item: &dovetail_api::InboxItem) -> bool {
        matches!(&item.kind, InboxKind::Permission { tool, .. } if tool == UPGRADE_READY_PERMISSION)
    }

    fn input_of(item: &dovetail_api::InboxItem) -> serde_json::Value {
        match &item.kind {
            InboxKind::Permission { input, .. } => input.clone(),
            other => panic!("not a permission item: {other:?}"),
        }
    }

    #[test]
    fn upgrade_builds_in_the_background_and_offers_the_build() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        u.h.send(Command::Upgrade { local: false });
        let item = u.wait_for_item();
        let input = input_of(&item);
        assert_eq!(input["source"], "abc1234");
        assert_eq!(input["restarting"], false);
        // The staged build is on disk, under the data directory only.
        assert!(u.h.paths.data.join("upgrade/staged/dovetail").is_file());
        // A second `:upgrade` while the item waits is refused.
        u.h.send(Command::Upgrade { local: false });
        u.h.drain(Duration::from_millis(300));
        assert!(
            u.h.failures
                .iter()
                .any(|f| f.contains("an upgrade is ready")),
            "{:?}",
            u.h.failures
        );
    }

    #[test]
    fn d_keeps_the_build_staged_and_does_not_restart() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        u.h.send(Command::Upgrade { local: false });
        let item = u.wait_for_item();
        u.answer(&item, PermissionAnswer::Deny { message: None });
        u.h.wait_for("the item to go", |s| !s.inbox.iter().any(is_ready));
        u.h.drain(Duration::from_millis(300));
        assert!(u.h.paths.data.join("upgrade/staged/ready.json").is_file());
        let core = u.h.core.take().expect("handle");
        assert!(!core.restart_requested());
        u.h.send(Command::Quit { end_sessions: true });
        assert!(core.wait(Duration::from_secs(10)));
    }

    #[test]
    fn y_with_every_agent_idle_quits_to_restart() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        u.h.send(Command::Upgrade { local: false });
        let item = u.wait_for_item();
        u.answer(&item, PermissionAnswer::Allow);
        u.wait_for_restart_event();
        let core = u.h.core.take().expect("handle");
        assert!(core.restart_requested());
        assert!(
            core.wait(Duration::from_secs(10)),
            "the core stopped on its own"
        );
    }

    #[test]
    fn y_waits_for_a_busy_agent_and_says_which() {
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s","cwd":"/tmp","permissionMode":"auto"}"#,
            r#"{"type":"user","message":{"role":"user","content":"go"},"parent_tool_use_id":null,"isReplay":true}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"working"}]},"parent_tool_use_id":null}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.01}"#,
        ];
        let fake = FakeDriver::from_ndjson(&lines.join("\n"))
            .expect("fixture")
            .with_options(ReplayOptions {
                pause_before: vec![3],
                ..ReplayOptions::default()
            });
        let mut u = Upgrading::start(fake.clone(), false);
        prompt(&u.h, "go");
        u.h.wait_for("the reply", |s| {
            s.conversations
                .get(&AgentId(COORDINATOR.into()))
                .is_some_and(|t| t.entries.iter().any(|e| e.text == "working"))
        });
        u.h.send(Command::Upgrade { local: false });
        let item = u.wait_for_item();
        u.answer(&item, PermissionAnswer::Allow);
        u.h.wait_for("the item to list who it waits for", |s| {
            s.inbox.iter().any(|i| {
                is_ready(i) && {
                    let input = input_of(i);
                    input["restarting"] == true
                        && input["waiting"] == serde_json::json!(["coordinator"])
                }
            })
        });
        // Nothing was cut off: the turn is still held.
        assert!(!u.h.core.as_ref().unwrap().restart_requested());

        fake.resume();
        u.wait_for_restart_event();
        let core = u.h.core.take().expect("handle");
        assert!(core.restart_requested());
        assert!(core.wait(Duration::from_secs(10)));
    }

    #[test]
    fn a_failed_build_is_an_error_item_with_the_tail() {
        let mut u = Upgrading::start(fixture("plain_turn"), true);
        u.h.send(Command::Upgrade { local: false });
        u.h.wait_for("the error item", |s| {
            s.inbox.iter().any(|i| {
                matches!(&i.kind, InboxKind::Error { text }
                    if text.contains("cargo build failed")
                        && text.contains("unresolved import")
                        && text.contains("Nothing was replaced"))
            })
        });
        assert!(u.ready_item().is_none());
        assert!(!u.h.paths.data.join("upgrade/staged/dovetail").exists());
        u.h.quit();
    }

    #[test]
    fn upgrade_is_refused_when_it_is_off() {
        let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
        h.send(Command::Upgrade { local: false });
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.failures.is_empty() {
            assert!(Instant::now() < deadline, "no refusal");
            h.drain(Duration::from_millis(50));
        }
        assert!(h.failures[0].contains("not available"), "{:?}", h.failures);
        h.quit();
    }

    #[test]
    fn a_build_handed_over_the_socket_raises_the_item() {
        let mut u = Upgrading::start(fixture("plain_turn"), false);
        // Nothing staged yet.
        let refused = socket_call(&u.h.socket(), SocketRequestBody::StagedUpgrade);
        assert!(
            matches!(refused, SocketResponseBody::Error { .. }),
            "{refused:?}"
        );

        // `dovetail upgrade` builds into the staging folder, then hands it over.
        let staged = u.h.paths.data.join("upgrade/staged");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("dovetail"), "new binary").unwrap();
        std::fs::write(
            staged.join("ready.json"),
            r#"{"source":"def5678","differing":["roles/slice.md"],"built_at_ms":1}"#,
        )
        .unwrap();
        let reply = socket_call(&u.h.socket(), SocketRequestBody::StagedUpgrade);
        assert_eq!(reply, SocketResponseBody::UpgradeStaged);
        let input = input_of(&u.wait_for_item());
        assert_eq!(input["source"], "def5678");
        assert_eq!(input["differing"], serde_json::json!(["roles/slice.md"]));
        u.h.quit();
    }
}

#[test]
fn a_side_chat_gets_only_the_core_suite_whatever_the_config_says() {
    let fake = fixture("plain_turn");
    let mut config = Config::default();
    config.suites.side_chat = dovetail_api::tools::Suite::ALL.to_vec();
    let mut h = Harness::start(fake.clone(), true, config);
    h.send(Command::NewSideChat {
        question: "what does the reducer do?".into(),
    });
    h.wait_for("the side chat", |s| {
        s.agents.iter().any(|a| a.name == "side-1")
    });
    wait_started(&fake, 1);
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| s.name.as_deref().is_some_and(|n| n.ends_with(".side-1")))
        .expect("side chat spec");
    let mcp = std::fs::read_to_string(spec.mcp_config_path.as_ref().expect("mcp")).expect("read");
    assert!(mcp.contains(r#""DOVETAIL_SUITES": "core""#), "{mcp}");
    // The config's list is reported once, as a warning.
    assert!(
        h.snap
            .warnings
            .iter()
            .any(|w| w.id.0 == "config-suites" && w.text.contains("ignored")),
        "{:?}",
        h.snap.warnings
    );
    h.quit();
}

#[test]
fn a_side_chat_runs_read_only_in_dont_ask_mode_and_keeps_it() {
    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    h.send(Command::NewSideChat {
        question: "what does the reducer do?".into(),
    });
    h.wait_for("the side chat", |s| {
        s.agents.iter().any(|a| a.name == "side-1")
    });
    wait_started(&fake, 1);
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| s.name.as_deref().is_some_and(|n| n.ends_with(".side-1")))
        .expect("side chat spec");
    let args = spec.claude_args();
    // Anything not allowed is denied without asking (decided 2026-11-11).
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "dontAsk"),
        "{args:?}"
    );
    let allowed = args
        .windows(2)
        .find(|w| w[0] == "--allowedTools")
        .map(|w| w[1].clone())
        .expect("--allowedTools");
    for rule in [
        "Bash(git log *)",
        "Bash(rg *)",
        "Bash(cargo tree --locked *)",
        "WebFetch",
    ] {
        assert!(allowed.contains(rule), "{rule} in {allowed}");
    }
    assert!(!allowed.split(',').any(|r| r == "Bash"), "{allowed}");
    let denied = args
        .windows(2)
        .find(|w| w[0] == "--disallowedTools")
        .map(|w| w[1].clone())
        .expect("--disallowedTools");
    for rule in ["Edit", "Write", "NotebookEdit", "Bash(find *-exec*)"] {
        assert!(denied.contains(rule), "{rule} in {denied}");
    }

    // Its mode never cycles: auto would let the classifier approve other commands.
    h.send(Command::CyclePermissionMode {
        agent: AgentId("side-1".into()),
    });
    h.drain(Duration::from_millis(300));
    assert!(
        h.failures.iter().any(|f| f.contains("read-only")),
        "{:?}",
        h.failures
    );
    let side = h
        .snap
        .agents
        .iter()
        .find(|a| a.name == "side-1")
        .expect("side-1");
    assert_eq!(side.permission_mode, PermissionMode::Manual);
    h.quit();
}

/// The model selector (task #39): a live switch changes the agent only once the session
/// confirms it, a refused one keeps the old model and says why, and the choice is what the
/// next start passes as `--model`/`--effort`.
#[test]
fn a_model_switch_is_confirmed_live_refused_safely_and_used_on_the_next_start() {
    use dovetail_api::ReasoningEffort;
    // Paused before the first line, so the session stays live and reports nothing itself.
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    prompt(&h, "hi");
    h.wait_for("the selector's models", |s| {
        s.models.len() == dovetail_api::BUILTIN_MODELS.len()
    });
    h.send(Command::SetModel {
        agent: coordinator.clone(),
        model: "sonnet".into(),
        effort: Some(ReasoningEffort::High),
    });
    h.wait_for("the confirmed model", |s| {
        let c = coordinator_of(s);
        c.model.as_deref() == Some("sonnet") && c.effort == Some(ReasoningEffort::High)
    });
    assert!(fake.sent().contains(&Sent::Model {
        model: Some("sonnet".into()),
        effort: Some(ReasoningEffort::High),
    }));
    // The same model at another effort: no `set_model`, only the effort.
    h.send(Command::SetModel {
        agent: coordinator.clone(),
        model: "sonnet".into(),
        effort: Some(ReasoningEffort::Low),
    });
    h.wait_for("the new effort", |s| {
        coordinator_of(s).effort == Some(ReasoningEffort::Low)
    });
    assert!(fake.sent().contains(&Sent::Model {
        model: None,
        effort: Some(ReasoningEffort::Low),
    }));

    h.send(Command::SetModel {
        agent: coordinator.clone(),
        model: "bogus-model".into(),
        effort: None,
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.failures.iter().any(|f| f.contains("bogus-model")) {
        assert!(Instant::now() < deadline, "no failure: {:?}", h.failures);
        h.drain(Duration::from_millis(20));
    }
    let failure = h.failures.iter().find(|f| f.contains("bogus-model"));
    assert!(
        failure.is_some_and(|f| f.contains("keeps sonnet")),
        "{failure:?}"
    );
    assert_eq!(coordinator_of(&h.snap).model.as_deref(), Some("sonnet"));

    let paths = h.paths.clone();
    let _state = h.quit();
    let fake = fixture("plain_turn");
    let h = Harness::start_on(fake.clone(), paths);
    prompt(&h, "again");
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "never restarted");
        std::thread::sleep(Duration::from_millis(10));
    }
    let spec = &fake.started_specs()[0];
    assert_eq!(spec.model.as_deref(), Some("sonnet"));
    assert_eq!(spec.effort, Some(ReasoningEffort::Low));
    h.quit();
}

/// Review of task #39: a settings answer confirms only its own switch. A late start-up read, or
/// an earlier switch's answer arriving out of order, changes nothing; and a second switch is
/// refused while the first waits for its answer.
#[test]
fn a_stale_or_out_of_order_settings_answer_does_not_confirm_a_switch() {
    use dovetail_api::ReasoningEffort;
    use dovetail_driver::{DriverEvent, SwitchId};
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![0],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let coordinator = AgentId(COORDINATOR.into());
    prompt(&h, "hi");
    h.wait_for("the session started", |_| !fake.started_specs().is_empty());
    fake.hold_model_answers();
    let set = |model: &str, effort| Command::SetModel {
        agent: coordinator.clone(),
        model: model.into(),
        effort: Some(effort),
    };
    let models = |f: &FakeDriver| {
        f.sent()
            .iter()
            .filter(|s| matches!(s, Sent::Model { .. }))
            .count()
    };
    // A held switch makes no event, so poll for it.
    let wait_sent = |h: &mut Harness, n: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while models(&fake) < n {
            assert!(Instant::now() < deadline, "switch {n} never sent");
            h.drain(Duration::from_millis(20));
        }
    };
    h.send(set("sonnet", ReasoningEffort::High));
    wait_sent(&mut h, 1);

    // A second switch while the first is unanswered is refused, and never sent.
    h.send(set("haiku", ReasoningEffort::Max));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.failures.iter().any(|f| f.contains("still switching")) {
        assert!(Instant::now() < deadline, "no refusal: {:?}", h.failures);
        h.drain(Duration::from_millis(20));
    }
    assert_eq!(models(&fake), 1);

    // The start-up read answers late, after the switch was sent: ignored.
    fake.send_event(DriverEvent::ModelSettings {
        switch: None,
        model: Some("claude-opus-5-5".into()),
        effort: Some(ReasoningEffort::Low),
    });
    h.drain(Duration::from_millis(200));
    let c = coordinator_of(&h.snap);
    assert_eq!((c.model.as_deref(), c.effort), (None, None));

    // The switch's own answer confirms it.
    fake.release_model_answers(&[0]);
    h.wait_for("the confirmed switch", |s| {
        coordinator_of(s).effort == Some(ReasoningEffort::High)
    });

    // A later switch, held; then the first switch's answer again, out of order: ignored.
    fake.hold_model_answers();
    h.send(set("sonnet", ReasoningEffort::Max));
    wait_sent(&mut h, 2);
    fake.send_event(DriverEvent::ModelSettings {
        switch: Some(SwitchId(0)),
        model: Some("sonnet".into()),
        effort: Some(ReasoningEffort::Low),
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(coordinator_of(&h.snap).effort, Some(ReasoningEffort::High));
    assert!(
        !h.failures.iter().any(|f| f.contains("kept effort")),
        "{:?}",
        h.failures
    );
    fake.release_model_answers(&[0]);
    h.wait_for("the second switch confirmed", |s| {
        coordinator_of(s).effort == Some(ReasoningEffort::Max)
    });
    h.quit();
}

// --- efforts (docs/design-v2.md §4) ---

fn effort_named<'a>(s: &'a Snapshot, name: &str) -> Option<&'a dovetail_api::Effort> {
    s.efforts.iter().find(|e| e.name == name)
}

fn create_effort(h: &mut Harness, typed: &str, stored: &str) -> dovetail_api::EffortId {
    h.send(Command::CreateEffort(dovetail_api::NewEffort {
        name: typed.into(),
        kind: dovetail_api::EffortKind::Feature,
        question_or_goal: " A buyer can pay without an account ".into(),
    }));
    h.wait_for("the effort", |s| effort_named(s, stored).is_some());
    effort_named(&h.snap, stored).expect("effort").id
}

fn wait_sent_containing(fake: &FakeDriver, wanted: &str) {
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

/// Commands are handled in order, so once a later command's effect shows, an earlier one that
/// was refused has reported its failure.
#[test]
fn an_effort_is_named_in_kebab_case_and_its_criterion_goes_claimed_then_met() {
    use dovetail_api::{CriterionState, EffortChange, EffortKind, UNSORTED_EFFORT, WhereWasI};

    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    h.wait_for("the first snapshot", |s| !s.efforts.is_empty());
    assert_eq!(h.snap.efforts[0].id, UNSORTED_EFFORT);

    let id = create_effort(&mut h, "Guest Checkout!", "guest-checkout");
    let effort = effort_named(&h.snap, "guest-checkout").expect("effort");
    assert_eq!(
        effort.question_or_goal,
        "A buyer can pay without an account"
    );

    // The same name typed another way is the same name: refused. So is renaming Unsorted.
    h.send(Command::CreateEffort(dovetail_api::NewEffort {
        name: "guest_checkout".into(),
        kind: EffortKind::Fix,
        question_or_goal: String::new(),
    }));
    h.send(Command::RenameEffort {
        id: UNSORTED_EFFORT,
        name: "misc".into(),
    });
    h.send(Command::RenameEffort {
        id,
        name: "Guest Pay".into(),
    });
    h.wait_for("the rename", |s| effort_named(s, "guest-pay").is_some());
    assert_eq!(
        h.failures,
        vec![
            "there is already an effort named guest-checkout".to_string(),
            "Unsorted cannot be renamed".to_string(),
        ]
    );
    assert_eq!(h.snap.efforts.len(), 2);

    let card = WhereWasI {
        last_done: "API endpoint merged (PR #41)".into(),
        waits_on_you: "Approve merge of PR #42".into(),
        next_step: "Order email".into(),
    };
    h.send(Command::SetWhereWasI {
        id,
        where_was_i: card.clone(),
    });
    h.send(Command::UpdateEffort {
        id,
        patch: vec![EffortChange::Kind(EffortKind::Experiment)],
    });
    h.send(Command::AddCriterion {
        effort: id,
        text: "pay as guest end to end".into(),
    });
    h.wait_for("the criterion", |s| {
        effort_named(s, "guest-pay").is_some_and(|e| !e.criteria.is_empty())
    });
    let effort = effort_named(&h.snap, "guest-pay").expect("effort").clone();
    assert_eq!(effort.where_was_i, card);
    assert_eq!(effort.kind, EffortKind::Experiment);
    let criterion = effort.criteria[0].id;
    assert_eq!(effort.criteria[0].state, CriterionState::Open);

    let state_of = move |s: &Snapshot| effort_named(s, "guest-pay").map(|e| e.criteria[0].state);
    h.send(Command::ClaimCriterion {
        id: criterion,
        evidence: "PR #41".into(),
    });
    h.wait_for("the claim", |s| {
        state_of(s) == Some(CriterionState::Claimed)
    });
    // A claim alone tells the coordinator nothing and ticks nothing.
    assert!(fake.sent().is_empty(), "{:?}", fake.sent());

    h.send(Command::ReturnCriterion {
        id: criterion,
        reason: Some("the order email is missing".into()),
    });
    h.wait_for("the return", |s| state_of(s) == Some(CriterionState::Open));
    wait_sent_containing(
        &fake,
        "sent criterion 1 of effort guest-pay back as not met (\"pay as guest end to end\"): \
         the order email is missing",
    );

    h.send(Command::ClaimCriterion {
        id: criterion,
        evidence: "PR #44".into(),
    });
    h.send(Command::ApproveCriterion { id: criterion });
    h.wait_for("the approval", |s| state_of(s) == Some(CriterionState::Met));
    wait_sent_containing(&fake, "approved criterion 1 of effort guest-pay");
    let met = effort_named(&h.snap, "guest-pay").expect("effort").clone();
    assert_eq!(met.criteria[0].evidence.as_deref(), Some("PR #44"));

    // Reopen: efforts, the card and the criterion come back in the first snapshot.
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the efforts", |s| s.efforts.len() == 2);
    assert_eq!(effort_named(&h.snap, "guest-pay"), Some(&met));
    h.quit();
}

/// Decided 2026-11-16: an agent claims a criterion with `coordination_claim_criterion`; the
/// claim waits for the user, and no tool call can make a criterion met.
#[test]
fn an_agent_claims_a_criterion_and_only_the_user_approves_it() {
    use dovetail_api::CriterionState;
    use dovetail_api::tools::ClaimCriterion;

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    h.send(Command::AddCriterion {
        effort,
        text: "fees compared".into(),
    });
    h.wait_for("the criterion", |s| {
        effort_named(s, "providers").is_some_and(|e| !e.criteria.is_empty())
    });
    let criterion = effort_named(&h.snap, "providers").expect("effort").criteria[0].id;
    let state_of = move |s: &Snapshot| effort_named(s, "providers").map(|e| e.criteria[0].state);
    let claim = |evidence: &str| {
        Tool::ClaimCriterion(ClaimCriterion {
            criterion,
            evidence: evidence.into(),
        })
    };
    let socket = h.socket();

    let (text, is_error) =
        result_text(socket_call(&socket, tool(COORDINATOR, claim("finding 41"))));
    assert!(!is_error, "{text}");
    assert!(text.contains("waits for the user's approval"), "{text}");
    h.wait_for("claimed", |s| state_of(s) == Some(CriterionState::Claimed));
    let evidence = effort_named(&h.snap, "providers").expect("effort").criteria[0]
        .evidence
        .clone();
    assert_eq!(evidence.as_deref(), Some("finding 41"));

    // Empty evidence and an unknown criterion are refused.
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, claim("  "))));
    assert!(is_error && text.contains("evidence"), "{text}");
    let unknown = Tool::ClaimCriterion(ClaimCriterion {
        criterion: dovetail_api::CriterionId(999),
        evidence: "x".into(),
    });
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, unknown)));
    assert!(is_error, "{text}");

    // Only the user makes it met; a claim of a met criterion is refused and changes nothing.
    h.send(Command::ApproveCriterion { id: criterion });
    h.wait_for("met", |s| state_of(s) == Some(CriterionState::Met));
    let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, claim("again"))));
    assert!(is_error && text.contains("already met"), "{text}");
    h.drain(Duration::from_millis(100));
    assert_eq!(state_of(&h.snap), Some(CriterionState::Met));
    h.quit();
}

// --- blocks and cards (ADR 0025, ADR 0027) ---

fn idea(effort: dovetail_api::EffortId, text: &str) -> dovetail_api::NewCard {
    dovetail_api::NewCard {
        effort,
        block: None,
        kind: dovetail_api::CardKind::Idea,
        reference: None,
        text: text.into(),
    }
}

fn card_titled<'a>(s: &'a Snapshot, title: &str) -> Option<&'a dovetail_api::Card> {
    s.cards.iter().find(|c| c.title == title)
}

/// Blocks placed by the core, a card made incoming, placed, moved and linked, all still there
/// after a restart; the commands that make no sense are refused and change nothing.
#[test]
fn blocks_cards_and_links_are_core_state_that_survives_a_restart() {
    use dovetail_api::Layout;
    use dovetail_api::geometry::{BLOCK_GAP, CARD_W, HEADER, PAD};

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    let other = create_effort(&mut h, "other", "other");
    h.send(Command::CreateBlock {
        effort,
        title: String::new(),
        layout: Layout::QuestionTree,
        near: None,
    });
    h.send(Command::CreateBlock {
        effort: other,
        title: "Elsewhere".into(),
        layout: Layout::FreeArea,
        near: None,
    });
    h.send(Command::CreateBlock {
        effort,
        title: "Which provider?".into(),
        layout: Layout::OptionsCompare,
        near: None,
    });
    // Where the user looks: centred there when it is empty, else the nearest empty spot.
    h.send(Command::CreateBlock {
        effort,
        title: "Notes".into(),
        layout: Layout::FreeArea,
        near: Some(dovetail_api::Point { x: 145, y: 1000 }),
    });
    h.send(Command::CreateBlock {
        effort,
        title: "Crowded".into(),
        layout: Layout::FreeArea,
        near: Some(dovetail_api::Point { x: 145, y: 80 }),
    });
    h.wait_for("five blocks", |s| s.blocks.len() == 5);
    let tree = h.snap.blocks[0].clone();
    let elsewhere = h.snap.blocks[1].id;
    let compare = h.snap.blocks[2].clone();
    let (w, hh) = (
        2 * PAD + CARD_W,
        HEADER + dovetail_api::geometry::CARD_H + PAD,
    );
    assert_eq!(
        (h.snap.blocks[3].x, h.snap.blocks[3].y),
        (145 - w / 2, 1000 - hh / 2)
    );
    // Centred on the tree block it would cover: moved off it, the tree did not move.
    let crowded = h.snap.blocks[4].clone();
    assert_ne!((crowded.x, crowded.y), (145 - w / 2, 80 - hh / 2));
    assert!(
        crowded.x >= tree.x + w + BLOCK_GAP / 2
            || crowded.y >= tree.y + hh + BLOCK_GAP / 2
            || crowded.x + w + BLOCK_GAP / 2 <= tree.x
            || crowded.y + hh + BLOCK_GAP / 2 <= tree.y,
        "{crowded:?}"
    );
    assert_eq!(h.snap.blocks[0], tree);
    assert_eq!(
        tree.title, "Question tree",
        "an empty title is the layout's name"
    );
    assert_eq!((tree.x, tree.y), (0, 0));
    // The second block of an effort goes right of the first; another effort's canvas starts empty.
    assert_eq!(h.snap.blocks[1].x, 0);
    assert_eq!((compare.x, compare.y), (2 * PAD + CARD_W + BLOCK_GAP, 0));

    h.send(Command::AddCard {
        card: idea(effort, "Ask Mollie about EU-only pricing"),
        link_from: None,
    });
    h.wait_for("the idea", |s| {
        card_titled(s, "Ask Mollie about EU-only pricing").is_some()
    });
    let first = card_titled(&h.snap, "Ask Mollie about EU-only pricing")
        .expect("card")
        .clone();
    assert_eq!((first.block, first.added_by.as_str()), (None, "user"));

    // Refused: moving an incoming card, placing it in another effort's block, dropping a card
    // that does not exist, a link to itself.
    h.send(Command::MoveCard {
        id: first.id,
        x: 1,
        y: 1,
    });
    h.send(Command::PlaceCard {
        id: first.id,
        block: Some(elsewhere),
    });
    h.send(Command::DropIncoming {
        id: dovetail_api::CardId(999),
    });
    h.send(Command::LinkCards {
        from: first.id,
        to: first.id,
        label: String::new(),
    });
    // Placed with no block named: the effort's newest block, in its first cell.
    h.send(Command::PlaceCard {
        id: first.id,
        block: None,
    });
    h.wait_for("placed", |s| {
        s.cards.iter().any(|c| c.block == Some(crowded.id))
    });
    let placed = card_titled(&h.snap, "Ask Mollie about EU-only pricing").expect("card");
    assert_eq!(
        (placed.x, placed.y, placed.seen),
        (crowded.x + PAD, crowded.y + HEADER, true)
    );
    assert_eq!(h.failures.len(), 4, "{:?}", h.failures);
    assert!(h.failures[0].contains("incoming"), "{:?}", h.failures);
    assert!(h.failures[1].contains("another effort"), "{:?}", h.failures);
    h.send(Command::PlaceCard {
        id: first.id,
        block: None,
    });
    h.send(Command::DropIncoming { id: first.id });
    h.send(Command::MoveCard {
        id: first.id,
        x: 41,
        y: 62,
    });
    h.wait_for("moved", |s| s.cards.iter().any(|c| (c.x, c.y) == (41, 62)));
    assert_eq!(h.failures.len(), 6, "{:?}", h.failures);

    // A second card linked from the first on creation; a card of another effort cannot be linked.
    h.send(Command::AddCard {
        card: idea(effort, "Compare Stripe and Adyen"),
        link_from: Some(first.id),
    });
    h.send(Command::AddCard {
        card: idea(other, "Unrelated"),
        link_from: None,
    });
    h.wait_for("three cards", |s| s.cards.len() == 3);
    let second = card_titled(&h.snap, "Compare Stripe and Adyen")
        .expect("card")
        .id;
    let third = card_titled(&h.snap, "Unrelated").expect("card").id;
    assert_eq!(h.snap.card_links.len(), 1);
    h.send(Command::LinkCards {
        from: first.id,
        to: third,
        label: String::new(),
    });
    h.send(Command::LinkCards {
        from: first.id,
        to: second,
        label: "next".into(),
    });
    h.wait_for("relabelled", |s| {
        s.card_links.first().is_some_and(|l| l.label == "next")
    });
    assert_eq!(h.snap.card_links.len(), 1, "linking again only relabels");
    assert!(
        h.failures
            .last()
            .expect("failure")
            .contains("different efforts")
    );

    // Everything is the same after a restart, and the ids go on from where they were.
    let before = (
        h.snap.blocks.clone(),
        h.snap.cards.clone(),
        h.snap.card_links.clone(),
    );
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the first snapshot", |s| !s.efforts.is_empty());
    assert_eq!(
        (
            h.snap.blocks.clone(),
            h.snap.cards.clone(),
            h.snap.card_links.clone()
        ),
        before
    );
    h.send(Command::DeleteCard { id: second });
    h.wait_for("deleted", |s| s.cards.len() == 2);
    assert!(h.snap.card_links.is_empty(), "a card's links go with it");
    h.send(Command::RenameBlock {
        id: tree.id,
        title: "Providers".into(),
    });
    h.wait_for("renamed", |s| s.blocks[0].title == "Providers");
    h.quit();
}

/// A chat reply copied into an idea card; a reference checked against its target; a card whose
/// target is gone reads "missing" and does not stop the snapshot.
#[test]
fn a_reply_becomes_an_idea_and_a_reference_must_exist() {
    use dovetail_api::{CardKind, CardRef, NewCard, RefKind};

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "hello".into(),
        attachments: vec![],
        effort: None,
    });
    h.wait_for("a reply", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Assistant && !e.text.is_empty())
            })
    });
    let reply = h.snap.conversations[&AgentId(COORDINATOR.into())]
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::Assistant)
        .cloned()
        .expect("reply");
    h.send(Command::CardFromReply {
        agent: AgentId(COORDINATOR.into()),
        entry: reply.id,
        effort,
    });
    h.wait_for("the idea", |s| {
        s.cards.iter().any(|c| c.kind == CardKind::Idea)
    });
    assert_eq!(h.snap.cards[0].text, reply.text, "the words are copied");
    assert_eq!(h.snap.cards[0].block, None);

    let reference = |kind: RefKind, id: &str| NewCard {
        effort,
        block: None,
        kind: kind.card_kind(),
        reference: Some(CardRef {
            kind,
            id: id.into(),
        }),
        text: String::new(),
    };
    // A target that does not exist is refused.
    h.send(Command::AddCard {
        card: reference(RefKind::Task, "77"),
        link_from: None,
    });
    h.send(Command::AddCard {
        card: reference(RefKind::Map, "no-such-map"),
        link_from: None,
    });
    h.send(Command::CreateTask(NewTask {
        title: "Pick a provider".into(),
        p_level: None,
        bucket: None,
        notes: "Notes on the pick".into(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Some(effort),
        criterion: None,
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    h.send(Command::AddCard {
        card: reference(RefKind::Task, &id.0.to_string()),
        link_from: None,
    });
    h.wait_for("the task card", |s| {
        card_titled(s, "Pick a provider").is_some()
    });
    let card = card_titled(&h.snap, "Pick a provider")
        .expect("card")
        .clone();
    assert!(!card.missing && card.text.is_empty());
    assert_eq!(h.failures.len(), 2, "{:?}", h.failures);
    assert!(
        h.failures[0].contains("there is no task 77"),
        "{:?}",
        h.failures
    );
    h.quit();
}

/// The coordinator's canvas tools add blocks, cards and links; no tool moves or deletes. A block
/// goes in empty space, a card under its parent, marked unseen; nothing already placed moves.
#[test]
fn the_canvas_tools_add_and_link_and_nothing_else() {
    use dovetail_api::geometry::{CARD_H, GAP};
    use dovetail_api::tools::{CanvasAddBlock, CanvasAddCard, CanvasLinkCards, Suite, TOOLS};
    use dovetail_api::{BlockId, CardId, CardKind, Layout};

    let canvas_tools: Vec<&str> = TOOLS
        .iter()
        .filter(|t| t.suite == Suite::Canvas)
        .map(|t| t.name)
        .collect();
    assert_eq!(
        canvas_tools,
        ["canvas_add_block", "canvas_add_card", "canvas_link_cards"]
    );

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    let socket = h.socket();
    let add_to = |kind: CardKind,
                  text: &str,
                  reference: Option<&str>,
                  block: Option<BlockId>,
                  under: Option<CardId>| {
        Tool::CanvasAddCard(CanvasAddCard {
            effort: Some(effort),
            block,
            under,
            kind,
            text: text.into(),
            reference: reference.map(Into::into),
        })
    };
    let add = |kind: CardKind, text: &str, reference: Option<&str>| {
        add_to(kind, text, reference, None, None)
    };
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            add(CardKind::Question, "Which provider?", None),
        ),
    ));
    assert!(!is_error && text.contains("incoming"), "{text}");
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, add(CardKind::Idea, "Use Stripe", None)),
    ));
    assert!(!is_error);
    h.wait_for("two cards", |s| s.cards.len() == 2);
    assert!(
        h.snap
            .cards
            .iter()
            .all(|c| c.block.is_none() && c.added_by == COORDINATOR && c.seen)
    );
    let (from, to) = (h.snap.cards[0].id, h.snap.cards[1].id);
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::CanvasLinkCards(CanvasLinkCards {
                from,
                to,
                label: "answers".into(),
            }),
        ),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("the link", |s| s.card_links.len() == 1);

    // A block, a root question in it and a finding-like idea under the root.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::CanvasAddBlock(CanvasAddBlock {
                effort: Some(effort),
                layout: Layout::QuestionTree,
                title: "Which provider?".into(),
            }),
        ),
    ));
    assert!(!is_error && text.contains("block"), "{text}");
    h.wait_for("the block", |s| s.blocks.len() == 1);
    let block = h.snap.blocks[0].clone();
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            add_to(CardKind::Question, "Fees?", None, Some(block.id), None),
        ),
    ));
    assert!(!is_error);
    h.wait_for("the root", |s| card_titled(s, "Fees?").is_some());
    let root = card_titled(&h.snap, "Fees?").expect("root").clone();
    assert_eq!((root.block, root.seen), (Some(block.id), false));
    for text in ["Stripe 1.5 %", "Adyen: no list price"] {
        let (_, is_error) = result_text(socket_call(
            &socket,
            tool(
                COORDINATOR,
                add_to(CardKind::Idea, text, None, Some(block.id), Some(root.id)),
            ),
        ));
        assert!(!is_error);
    }
    h.wait_for("the answers and their links", |s| {
        s.cards.len() == 5 && s.card_links.len() == 3
    });
    let a = card_titled(&h.snap, "Stripe 1.5 %").expect("a").clone();
    let b = card_titled(&h.snap, "Adyen: no list price")
        .expect("b")
        .clone();
    // One row below the root, side by side, linked from it; the root did not move.
    assert_eq!((a.x, a.y), (root.x, root.y + CARD_H + GAP));
    assert_eq!(b.y, a.y);
    assert!(b.x > a.x);
    assert!(
        h.snap
            .card_links
            .iter()
            .any(|l| l.from == root.id && l.to == b.id)
    );
    assert_eq!(card_titled(&h.snap, "Fees?").expect("root"), &root);

    // Refused: a reference that does not exist, text on a reference card, an idea without text,
    // `under` with no block.
    for (call, wanted) in [
        (
            add(CardKind::Finding, "", Some("41")),
            "there is no finding 41",
        ),
        (add(CardKind::Finding, "words", Some("41")), "no `text`"),
        (add(CardKind::Idea, " ", None), "`text`"),
        (
            add_to(CardKind::Idea, "x", None, None, Some(root.id)),
            "name the `block`",
        ),
    ] {
        let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, call)));
        assert!(is_error && text.contains(wanted), "{text}");
    }
    h.drain(Duration::from_millis(100));
    assert_eq!(h.snap.cards.len(), 5);
    h.quit();
}

#[test]
fn a_task_is_created_in_unsorted_and_moved_to_an_effort_and_its_criterion() {
    use dovetail_api::{NewTask, UNSORTED_EFFORT};

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    h.send(Command::AddCriterion {
        effort,
        text: "fees compared".into(),
    });
    h.send(Command::CreateTask(NewTask {
        title: "Compare three providers".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: None,
        criterion: None,
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let task = h.snap.tasks[0].clone();
    assert_eq!(task.effort, UNSORTED_EFFORT);
    let criterion = effort_named(&h.snap, "providers").expect("effort").criteria[0].id;

    // A criterion of another effort is refused; the right pair goes through.
    h.send(Command::MoveTask {
        id: task.id,
        effort: UNSORTED_EFFORT,
        criterion: Some(criterion),
    });
    h.send(Command::MoveTask {
        id: task.id,
        effort,
        criterion: Some(criterion),
    });
    h.wait_for("the move", |s| s.tasks[0].effort == effort);
    assert_eq!(h.snap.tasks[0].criterion, Some(criterion));
    assert_eq!(
        h.failures,
        vec!["criterion 1 belongs to effort 2, not effort 1".to_string()]
    );
    assert_eq!(h.snap.tasks[0].title, task.title);
    h.quit();
}

#[test]
fn a_tagged_message_tags_the_reply_and_the_tag_survives_a_restart() {
    use dovetail_api::EffortId;

    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");

    // An effort this project does not have: nothing is sent.
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "lost".into(),
        attachments: Vec::new(),
        effort: Some(EffortId(99)),
    });
    h.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "Reply with exactly the word: alpha".into(),
        attachments: Vec::new(),
        effort: Some(effort),
    });
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    assert_eq!(h.failures, vec!["no effort 99".to_string()]);
    assert_eq!(
        fake.sent(),
        vec![Sent::User("Reply with exactly the word: alpha".into())]
    );
    let tags = |s: &Snapshot| -> Vec<(EntryKind, Option<EffortId>)> {
        s.conversations[&AgentId(COORDINATOR.into())]
            .entries
            .iter()
            .map(|e| (e.kind, e.effort))
            .collect()
    };
    let before = tags(&h.snap);
    assert_eq!(before[0], (EntryKind::User, Some(effort)));
    assert!(before.contains(&(EntryKind::Assistant, Some(effort))));
    // Dovetail's own notices carry no tag.
    assert!(
        before
            .iter()
            .all(|(kind, tag)| *kind != EntryKind::Notice || tag.is_none())
    );

    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the conversation", |s| {
        s.conversations.contains_key(&AgentId(COORDINATOR.into()))
    });
    assert_eq!(tags(&h.snap), before);

    // An untagged message after the restart is untagged; the thread's tag does not leak onto it.
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the untagged message", |s| {
        tags(s).contains(&(EntryKind::User, None))
    });
    h.quit();
}

// --- session names and slice reports (ADR 0022), automatic task status (ADR 0024) ---

fn task_status(s: &Snapshot, id: i64) -> Option<TaskStatus> {
    s.tasks
        .iter()
        .find(|t| t.id == TaskId(id))
        .map(|t| t.status)
}

fn report(state: dovetail_api::tools::ReportState, summary: &str, pr: Option<u32>) -> Tool {
    Tool::Report(dovetail_api::tools::Report {
        state,
        summary: summary.into(),
        pr,
    })
}

#[test]
fn sessions_are_named_after_the_project_and_a_slice_reports_through_dovetail() {
    use dovetail_api::NewTask;
    use dovetail_api::tools::ReportState;

    let fake = fixture("plain_turn");
    let mut h = Harness::start(fake.clone(), true, Config::default());
    let effort = create_effort(&mut h, "guest checkout", "guest-checkout");
    h.send(Command::CreateTask(NewTask {
        title: "API: guest order endpoint".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: Vec::new(),
        effort: Some(effort),
        criterion: None,
    }));
    h.send(Command::NewSlice {
        repo: None,
        task: Some(TaskId(1)),
        name: "api".into(),
        brief: "build the endpoint".into(),
    });
    h.wait_for("the slice working on its task", |s| {
        s.agents.iter().any(|a| a.name == "api")
            && task_status(s, 1) == Some(TaskStatus::InProgress)
    });
    // The slice is told the coordinator's session name, and its own carries the project.
    wait_sent_containing(&fake, "The coordinator's session is named `p.coordinator`");
    let names: Vec<String> = fake
        .started_specs()
        .iter()
        .filter_map(|s| s.name.clone())
        .collect();
    assert!(names.contains(&"p.api".to_string()), "{names:?}");
    wait_sent_containing(&fake, "slice api ready; its session is named `p.api`");
    assert!(names.iter().all(|n| n.starts_with("p.")), "{names:?}");

    let socket = h.socket();
    // Only a slice reports.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, report(ReportState::Done, "all good", None)),
    ));
    assert!(is_error && text.contains("is for slices"), "{text}");

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-api",
            report(ReportState::Progress, "endpoint drafted", None),
        ),
    ));
    assert!(!is_error, "{text}");
    assert_eq!(text, "Reported to the coordinator.");
    h.drain(Duration::from_millis(100));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-api",
            report(ReportState::Done, " PR open, tests pass ", Some(41)),
        ),
    ));
    assert!(!is_error, "{text}");
    assert_eq!(
        text,
        "Reported to the coordinator. Task #1 is now in review."
    );
    h.wait_for("the task in review", |s| {
        task_status(s, 1) == Some(TaskStatus::Review)
    });

    // One line in the coordinator's chat, tagged with the effort of the slice's task.
    let line = h.snap.conversations[&AgentId(COORDINATOR.into())]
        .entries
        .iter()
        .find(|e| e.text.contains("[done]"))
        .expect("the report line")
        .clone();
    assert_eq!(line.kind, EntryKind::AgentMessage);
    assert_eq!(line.effort, Some(effort));
    let parts = line.agent_message().expect("an agent message");
    assert_eq!(
        (parts.from, parts.body),
        ("api", "[done] PR open, tests pass (PR #41)")
    );
    // And the coordinator's session got it as a message.
    wait_sent_containing(
        &fake,
        "Report from slice api on task #1 API: guest order endpoint: [done] PR open, tests \
         pass (PR #41)",
    );
    h.quit();
}

/// A fake `gh`: `gh pr list --head <branch> ...` prints `<dir>/<branch with _ for />.json`, or
/// `[]` when there is no such file.
fn fake_gh(dir: &Path) -> PathBuf {
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

fn set_pr(dir: &Path, slice: &str, state: &str) {
    let text = format!(
        "[{{\"number\": 42, \"state\": \"{state}\", \"url\": \"https://example.test/pr/42\"}}]"
    );
    std::fs::write(dir.join(format!("dovetail_{slice}.json")), text).expect("pr file");
}

/// Slices `names` in a git repo, each on its own task (ids from 1, in order), all in progress.
/// With `shared`, every slice works on task 1.
fn repo_with_task_slices(names: &[&str], shared: bool) -> (TempDir, PathBuf, Paths) {
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

#[test]
fn a_task_follows_its_slices_pull_request_to_review_done_and_back() {
    let (dir, repo, paths) = repo_with_task_slices(&["api"], false);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    // No pull request: nothing moves.
    h.send(Command::RefreshWorktrees);
    h.wait_for("the first lookup", |s| {
        s.worktrees
            .iter()
            .any(|w| w.pr == dovetail_api::PrLookup::None)
    });
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    // Open: review.
    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::RefreshWorktrees);
    h.wait_for("review", |s| task_status(s, 1) == Some(TaskStatus::Review));

    // Closed without merging: back to in progress.
    set_pr(dir.path(), "api", "CLOSED");
    h.send(Command::RefreshWorktrees);
    h.wait_for("back in progress", |s| {
        task_status(s, 1) == Some(TaskStatus::InProgress)
    });

    // Reopened, then merged: review, then done.
    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::RefreshWorktrees);
    h.wait_for("review again", |s| {
        task_status(s, 1) == Some(TaskStatus::Review)
    });
    set_pr(dir.path(), "api", "MERGED");
    h.send(Command::RefreshWorktrees);
    h.wait_for("done", |s| task_status(s, 1) == Some(TaskStatus::Done));

    // The user's own word is never overruled: marked to do, a merged PR does not move it.
    h.send(Command::UpdateTask {
        id: TaskId(1),
        patch: vec![dovetail_api::TaskChange::Status(TaskStatus::Todo)],
    });
    h.wait_for("to do", |s| task_status(s, 1) == Some(TaskStatus::Todo));
    h.send(Command::RefreshWorktrees);
    h.drain(Duration::from_millis(300));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::Todo));
    h.quit();
}

/// Decided 2026-11-16: a task the user sends back from review stays in progress until
/// something new happens: its pull request changes, or its slice reports again.
#[test]
fn a_task_sent_back_from_review_stays_in_progress_until_something_new_happens() {
    use dovetail_api::tools::ReportState;
    let (dir, repo, paths) = repo_with_task_slices(&["api"], false);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::RefreshWorktrees);
    h.wait_for("review", |s| task_status(s, 1) == Some(TaskStatus::Review));

    // Sent back while the PR stays open: a new lookup of the same open PR moves nothing.
    h.send(Command::UpdateTask {
        id: TaskId(1),
        patch: vec![dovetail_api::TaskChange::Status(TaskStatus::InProgress)],
    });
    h.wait_for("sent back", |s| {
        task_status(s, 1) == Some(TaskStatus::InProgress)
    });
    h.send(Command::RefreshWorktrees);
    h.drain(Duration::from_millis(300));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    // The slice reports again: the hold ends, and done moves it to review.
    let (text, is_error) = result_text(socket_call(
        &h.socket(),
        tool("slice-api", report(ReportState::Done, "fixed", Some(42))),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("review again", |s| {
        task_status(s, 1) == Some(TaskStatus::Review)
    });

    // Sent back again; then the PR is merged, which is new: done.
    h.send(Command::UpdateTask {
        id: TaskId(1),
        patch: vec![dovetail_api::TaskChange::Status(TaskStatus::InProgress)],
    });
    h.wait_for("sent back again", |s| {
        task_status(s, 1) == Some(TaskStatus::InProgress)
    });
    set_pr(dir.path(), "api", "MERGED");
    h.send(Command::RefreshWorktrees);
    h.wait_for("done", |s| task_status(s, 1) == Some(TaskStatus::Done));
    h.quit();
}

#[test]
fn a_task_with_two_slices_follows_the_one_furthest_behind() {
    let (dir, repo, paths) = repo_with_task_slices(&["api", "web"], true);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slices", |s| {
        s.agents
            .iter()
            .filter(|a| a.kind == AgentKind::Slice)
            .count()
            == 2
    });
    let looked_up = |s: &Snapshot, state: dovetail_api::PrState| {
        s.worktrees
            .iter()
            .filter(|w| matches!(&w.pr, dovetail_api::PrLookup::Found(pr) if pr.state == state))
            .count()
    };

    set_pr(dir.path(), "api", "OPEN");
    h.send(Command::RefreshWorktrees);
    h.wait_for("one open PR seen", |s| {
        looked_up(s, dovetail_api::PrState::Open) == 1 && s.worktrees.len() == 2
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::InProgress));

    set_pr(dir.path(), "web", "OPEN");
    h.send(Command::RefreshWorktrees);
    h.wait_for("review", |s| task_status(s, 1) == Some(TaskStatus::Review));

    set_pr(dir.path(), "api", "MERGED");
    h.send(Command::RefreshWorktrees);
    h.wait_for("one merged PR seen", |s| {
        looked_up(s, dovetail_api::PrState::Merged) == 1
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(task_status(&h.snap, 1), Some(TaskStatus::Review));

    set_pr(dir.path(), "web", "MERGED");
    h.send(Command::RefreshWorktrees);
    h.wait_for("done", |s| task_status(s, 1) == Some(TaskStatus::Done));
    h.quit();
}

/// The coordinator merges and archives in one turn, so no slice turn ends in between: the
/// lookup made as the slice is archived is what shows the merge.
#[test]
fn a_merge_is_seen_when_the_slice_is_archived() {
    let (dir, repo, paths) = repo_with_task_slices(&["api"], false);
    GH.with(|g| *g.borrow_mut() = Some(fake_gh(dir.path())));
    let mut h = Harness::start_in(fixture("plain_turn"), paths, Some(repo));
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "api"));
    set_pr(dir.path(), "api", "MERGED");
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("api")),
    ));
    assert!(!is_error && text.starts_with(ARCHIVED), "{text}");
    h.wait_for("done, and the slice gone", |s| {
        task_status(s, 1) == Some(TaskStatus::Done) && !s.agents.iter().any(|a| a.name == "api")
    });
    h.quit();
}

// --- multi-repo projects (ADR 0021) ---

/// A git repo with one commit at `<dir>/<name>`, by its canonical path.
fn committed_repo(dir: &Path, name: &str) -> PathBuf {
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
fn registered_project(dir: &Path, names: &[&str]) -> (Paths, Vec<PathBuf>) {
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

fn replied(s: &Snapshot) -> bool {
    s.conversations
        .get(&AgentId(COORDINATOR.into()))
        .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
}

#[test]
fn the_coordinator_attaches_every_repo_and_a_slice_names_its_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, repos) = registered_project(dir.path(), &["api", "web"]);
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    prompt(&h, "hello");
    h.wait_for("the reply", replied);
    let spec = fake.started_specs()[0].clone();
    assert_eq!(spec.cwd, paths.project_home("p"));
    assert_eq!(spec.add_dirs, repos);
    let args = spec.claude_args();
    let attached: Vec<&String> = args
        .iter()
        .zip(args.iter().skip(1))
        .filter(|(flag, _)| *flag == "--add-dir")
        .map(|(_, dir)| dir)
        .collect();
    assert_eq!(attached.len(), 2, "{args:?}");
    assert_eq!(h.snap.project.as_ref().map(|p| p.repos.len()), Some(2));

    // Two repos: a slice must name one, and one the project has.
    let slice = |repo: Option<&str>| Command::NewSlice {
        task: None,
        name: "ui".into(),
        brief: "go".into(),
        repo: repo.map(Into::into),
    };
    h.send(slice(None));
    h.send(slice(Some("docs")));
    h.drain(Duration::from_millis(200));
    assert_eq!(h.failures.len(), 2, "{:?}", h.failures);
    assert!(h.failures[0].contains("several repos"), "{:?}", h.failures);
    assert!(h.failures[1].contains("no repo `docs`"), "{:?}", h.failures);

    h.send(slice(Some("web")));
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < 2 {
        assert!(Instant::now() < deadline, "the slice never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    let spec = fake.started_specs()[1].clone();
    assert_eq!(spec.cwd, repos[1]);
    assert_eq!(spec.worktree.as_deref(), Some("ui"));
    assert!(spec.add_dirs.is_empty());
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == dovetail_api::tools::env::REPO && v == "web"),
        "{:?}",
        spec.env
    );
    h.quit();
}

/// ADR 0021 point 4: a repo added while the coordinator works is attached once its turn ends:
/// it stops, resumes with the new `--add-dir`, and is told.
#[test]
fn adding_a_repo_restarts_the_coordinator_with_it_attached_after_its_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, repos) = registered_project(dir.path(), &["api"]);
    let docs = committed_repo(dir.path(), "docs");
    // Held before its `result`: the coordinator is mid-turn when the repo is added.
    let fake = fixture("plain_turn").with_options(ReplayOptions {
        pause_before: vec![4],
        ..ReplayOptions::default()
    });
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    prompt(&h, "hello");
    h.wait_for("the reply", replied);

    h.send(Command::AddRepo {
        project: dovetail_api::ProjectId("p".into()),
        path: docs.clone(),
        name: None,
    });
    h.wait_for("the project with two repos", |s| {
        s.project.as_ref().is_some_and(|p| p.repos.len() == 2)
            && s.projects
                .iter()
                .any(|p| p.id.0 == "p" && p.repos.len() == 2)
    });
    // Working: not stopped yet.
    h.drain(Duration::from_millis(100));
    assert!(!fake.sent().iter().any(|s| matches!(s, Sent::Shutdown(_))));
    assert_eq!(fake.started_specs().len(), 1);

    fake.resume();
    wait_sent_shutdown(&fake);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().len() < 2 {
        assert!(
            Instant::now() < deadline,
            "the coordinator was not restarted"
        );
        h.drain(Duration::from_millis(20));
    }
    let again = fake.started_specs()[1].clone();
    assert!(again.resume, "the same session, resumed");
    assert_eq!(again.add_dirs, vec![repos[0].clone(), docs.clone()]);
    wait_sent_containing(&fake, "repo `docs` attached at");
    h.quit();
}

#[test]
fn a_repo_with_an_open_slice_cannot_be_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, _repos) = registered_project(dir.path(), &["api", "web"]);
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths);
    h.send(Command::NewSlice {
        task: None,
        name: "ui".into(),
        brief: "go".into(),
        repo: Some("web".into()),
    });
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "ui"));
    let project = h.snap.project.clone().expect("open project");
    let id = |name: &str| {
        project
            .repos
            .iter()
            .find(|r| r.name == name)
            .expect("repo")
            .id
    };
    h.send(Command::RemoveRepo {
        project: project.id.clone(),
        repo: id("web"),
    });
    h.drain(Duration::from_millis(200));
    assert!(
        h.failures.iter().any(|f| f.contains("open slices (ui)")),
        "{:?}",
        h.failures
    );
    h.send(Command::RemoveRepo {
        project: project.id.clone(),
        repo: id("api"),
    });
    h.wait_for("api removed", |s| {
        s.project
            .as_ref()
            .is_some_and(|p| p.repos.iter().map(|r| r.name.as_str()).eq(["web"]))
    });
    h.quit();
}

/// ADR 0021 point 8: a coordinator from before, filed under its repo's folder, starts a fresh
/// session in `home/` at the first open, seeded, and Needs you says how to reopen the old one.
#[test]
fn a_coordinator_recorded_in_its_repo_starts_fresh_in_home() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (paths, repos) = registered_project(dir.path(), &["api"]);
    let claude = dir.path().join("claude");
    let old = dovetail_core::session::launch::transcript_path(
        &claude,
        &repos[0],
        &format!("{COORDINATOR}-session"),
    );
    seed(
        &paths,
        session_row(
            COORDINATOR,
            AgentKind::Coordinator,
            None,
            &claude,
            old.clone(),
            None,
        ),
    );
    let fake = fixture("plain_turn");
    let mut h = Harness::start_on(fake.clone(), paths.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "the coordinator never started");
        h.drain(Duration::from_millis(20));
    }
    let spec = fake.started_specs()[0].clone();
    assert!(!spec.resume, "a fresh session");
    assert_ne!(spec.session_id, format!("{COORDINATOR}-session"));
    assert_eq!(spec.cwd, paths.project_home("p"));
    wait_sent_containing(&fake, "Dovetail now runs the coordinator in its own folder");
    h.wait_for("the note in Needs you", |s| {
        s.inbox.iter().any(|i| {
            matches!(&i.kind, InboxKind::Error { text }
                if text.contains("moved to its own folder") && text.contains(&format!("{COORDINATOR}-session")))
        })
    });
    h.quit();
}

// ---- ADR 0026: slice previews and declared variants

/// The preview state of slice `name` in the snapshot.
fn preview_of(s: &Snapshot, agent: &str) -> Option<dovetail_api::PreviewState> {
    s.previews
        .iter()
        .find(|p| p.agent.0 == agent)
        .map(|p| p.state.clone())
}

/// `GET /` of a preview URL, as plain HTTP on a socket: the body, or `None` if nothing answers.
fn fetch(url: &str) -> Option<String> {
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
fn gone(url: &str) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if fetch(url).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn start_viewer_harness(recipe: Option<&str>) -> (Harness, TempDir, PathBuf) {
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

#[test]
fn a_preview_runs_in_the_worktree_serves_the_page_and_stops_with_the_slice() {
    let (mut h, _dir, _repo) = start_viewer_harness(Some(
        "preview = \"python3 -m http.server {port} --bind 127.0.0.1\"\n",
    ));
    assert_eq!(preview_of(&h.snap, "slice-web"), None);
    h.send(Command::StartPreview {
        agent: AgentId("slice-web".into()),
    });
    h.wait_for("the preview to run", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::PreviewState::Running { .. })
        )
    });
    let Some(dovetail_api::PreviewState::Running { url }) = preview_of(&h.snap, "slice-web") else {
        unreachable!()
    };
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    let page = fetch(&url).expect("the preview answers");
    // The server lists the worktree's files: it runs there.
    assert!(page.contains("README"), "{page}");

    // Asking again while it runs changes nothing.
    h.send(Command::StartPreview {
        agent: AgentId("slice-web".into()),
    });
    h.drain(Duration::from_millis(200));
    assert_eq!(
        preview_of(&h.snap, "slice-web"),
        Some(dovetail_api::PreviewState::Running { url: url.clone() })
    );

    // Archiving the slice stops it.
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("web")),
    ));
    assert!(!is_error && text.starts_with(ARCHIVED), "{text}");
    assert!(gone(&url), "the preview still answers after archive");
    h.wait_for("the slice and its preview to go", |s| {
        preview_of(s, "slice-web").is_none() && !s.agents.iter().any(|a| a.name == "web")
    });
    h.quit();
}

#[test]
fn restarting_a_preview_starts_it_again_and_quitting_stops_it() {
    let (mut h, _dir, _repo) = start_viewer_harness(Some(
        "preview = \"python3 -m http.server {port} --bind 127.0.0.1\"\n",
    ));
    let agent = AgentId("slice-web".into());
    h.send(Command::StartPreview {
        agent: agent.clone(),
    });
    h.wait_for("running", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::PreviewState::Running { .. })
        )
    });
    let Some(dovetail_api::PreviewState::Running { url: first }) = preview_of(&h.snap, "slice-web")
    else {
        unreachable!()
    };
    h.send(Command::RestartPreview {
        agent: agent.clone(),
    });
    h.wait_for("running again on a new port", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::PreviewState::Running { url }) if url != first
        )
    });
    assert!(gone(&first), "the first run was stopped");
    let Some(dovetail_api::PreviewState::Running { url }) = preview_of(&h.snap, "slice-web") else {
        unreachable!()
    };
    assert!(fetch(&url).is_some());
    h.quit();
    assert!(gone(&url), "quitting stops the preview");
}

#[test]
fn a_slice_of_a_repo_with_no_preview_line_says_so_and_a_failing_command_shows_its_output() {
    let (mut h, _dir, _repo) = start_viewer_harness(None);
    h.send(Command::StartPreview {
        agent: AgentId("slice-web".into()),
    });
    h.wait_for("no command", |s| {
        preview_of(s, "slice-web") == Some(dovetail_api::PreviewState::NoCommand)
    });
    h.quit();

    let (mut h, _dir, _repo) =
        start_viewer_harness(Some("preview = \"echo boom on port {port}; exit 3\"\n"));
    h.send(Command::StartPreview {
        agent: AgentId("slice-web".into()),
    });
    h.wait_for("failed", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::PreviewState::Failed { .. })
        )
    });
    let Some(dovetail_api::PreviewState::Failed { output }) = preview_of(&h.snap, "slice-web")
    else {
        unreachable!()
    };
    assert!(output.contains("exited before it listened"), "{output}");
    assert!(output.contains("boom on port"), "{output}");
    h.quit();
}

#[test]
fn a_repo_override_replaces_the_projects_preview_line() {
    // The project line would fail; the repo's own (named after the repo) serves.
    let (mut h, _dir, _repo) = start_viewer_harness(Some(
        "preview = \"exit 1\"\n[repos.p]\npreview = \"python3 -m http.server {port} --bind 127.0.0.1\"\n",
    ));
    h.send(Command::StartPreview {
        agent: AgentId("slice-web".into()),
    });
    h.wait_for("a preview state", |s| preview_of(s, "slice-web").is_some());
    h.wait_for("running", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(
                dovetail_api::PreviewState::Running { .. }
                    | dovetail_api::PreviewState::Failed { .. }
            )
        ) && !matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::PreviewState::Starting)
        )
    });
    assert!(
        matches!(
            preview_of(&h.snap, "slice-web"),
            Some(dovetail_api::PreviewState::Running { .. })
        ),
        "{:?}",
        preview_of(&h.snap, "slice-web")
    );
    h.quit();
}

fn declare(slots: Vec<dovetail_api::VariantSlot>) -> Tool {
    Tool::DeclareVariants(dovetail_api::tools::DeclareVariants { slots })
}

fn slot(
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

#[test]
fn a_slice_declares_variants_the_user_picks_and_keeps_and_the_slice_is_prompted() {
    use dovetail_api::VariantLevel::{Component, Global};
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::NewSlice {
        repo: None,
        task: None,
        name: "later".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice to have run", |s| {
        s.conversations
            .get(&AgentId("slice-later".into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
    });
    let socket = h.socket();
    let agent = AgentId("slice-later".into());

    // Only a slice may declare; the coordinator is refused.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, declare(vec![slot("theme", Global, &["a"])])),
    ));
    assert!(is_error && text.contains("for slices"), "{text}");
    // The whole list is checked: every problem comes back.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-later",
            declare(vec![
                slot("theme", Global, &["a", "a"]),
                slot("theme", Global, &[]),
            ]),
        ),
    ));
    assert!(
        is_error
            && text.contains("twice")
            && text.contains("needs 1 to 8 options")
            && text.contains("appears twice"),
        "{text}"
    );

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-later",
            declare(vec![
                slot("theme", Global, &["light", "dark"]),
                slot("header", Component, &["plain", "bold"]),
            ]),
        ),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("the declared set", |s| s.variants.len() == 1);
    let set = h.snap.variants[0].clone();
    assert_eq!(set.selection["theme"], "light");
    assert_eq!(set.selection["header"], "plain");

    h.send(Command::SelectVariant {
        agent: agent.clone(),
        slot: "theme".into(),
        option: "dark".into(),
    });
    h.wait_for("the pick", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.selection["theme"] == "dark")
    });
    h.send(Command::SelectVariant {
        agent: agent.clone(),
        slot: "theme".into(),
        option: "neon".into(),
    });
    h.drain(Duration::from_millis(300));
    assert!(
        h.failures.iter().any(|f| f.contains("not an option")),
        "{:?}",
        h.failures
    );

    // Declaring again keeps a pick that is still valid and drops one that is not.
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-later",
            declare(vec![
                slot("theme", Global, &["light", "dark", "sepia"]),
                slot("header", Component, &["compact"]),
            ]),
        ),
    ));
    assert!(!is_error);
    h.wait_for("the redeclared set", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.slots.len() == 2 && v.slots[1].options == ["compact"])
    });
    assert_eq!(h.snap.variants[0].selection["theme"], "dark");
    assert_eq!(h.snap.variants[0].selection["header"], "compact");

    // Keep: the picks become the kept options and the slice is prompted.
    h.send(Command::KeepVariants {
        agent: agent.clone(),
    });
    h.wait_for("the kept options", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.slots[0].kept.as_deref() == Some("dark"))
    });
    h.wait_for("the prompt in the slice's conversation", |s| {
        s.conversations
            .get(&agent)
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::User && e.text.contains("Keep this combination of variants: theme (global): dark; header (component): compact")))
    });

    // An empty declaration clears the list.
    let (_, is_error) = result_text(socket_call(&socket, tool("slice-later", declare(vec![]))));
    assert!(!is_error);
    h.wait_for("the cleared list", |s| s.variants.is_empty());
    h.quit();
}

#[test]
fn declared_variants_and_picks_survive_a_restart() {
    use dovetail_api::VariantLevel::Global;
    let (mut h, dir, repo) = start_viewer_harness(None);
    let paths = h.paths.clone();
    let socket = h.socket();
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-web",
            declare(vec![slot("theme", Global, &["light", "dark"])]),
        ),
    ));
    assert!(!is_error);
    h.send(Command::SelectVariant {
        agent: AgentId("slice-web".into()),
        slot: "theme".into(),
        option: "dark".into(),
    });
    h.wait_for("the pick", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.selection["theme"] == "dark")
    });
    h.quit();
    let h = Harness::start_with(fixture("plain_turn"), paths, Some(repo), Config::default());
    assert_eq!(h.snap.variants.len(), 1, "{:?}", h.snap.variants);
    assert_eq!(h.snap.variants[0].selection["theme"], "dark");
    drop(dir);
    h.quit();
}

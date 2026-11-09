//! The real core driven by `FakeDriver` replaying recorded fixtures: the stream rules that are
//! cheap to replay (double `result`, concurrent permissions, subagent routing), the inbox round
//! trips, the tool handlers over the real socket, and ADR 0005's unregistered folders.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::archive_result::{ARCHIVED, WAITING_FOR_USER};
use dovetail_api::tools::{
    ArchiveSlice, ArchiveTask, AskUser, BucketFilter, GetTask, ListTasks, LogDecision,
    ReportStatus, StartSlice, Tool,
};
use dovetail_api::{
    AgentId, AgentKind, AgentStatus, Alternative, Attachment, Bucket, Command, CoreLink,
    DecisionId, EntryKind, ImageType, InboxId, InboxKind, NewTask, PLevel, PermissionAnswer,
    PermissionMode, ServerMsg, Snapshot, TaskId, TaskStatus, apply,
};
use dovetail_core::config::Config;
use dovetail_core::paths::Paths;
use dovetail_core::projects::{Candidate, Project, Resolution};
use dovetail_core::{CoreHandle, CoreOptions};
use dovetail_driver::{ControlAnswer, FakeDriver, ReplayOptions, Sent};
use tempfile::TempDir;

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
    _dir: TempDir,
    _run: TempDir,
}

const COORDINATOR: &str = "coordinator";

impl Harness {
    fn start(driver: FakeDriver, registered: bool, config: Config) -> Harness {
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
            Resolution::Registered(Project {
                id: "p".into(),
                name: "work".into(),
                repo_path: repo.clone(),
                git_common_dir: None,
                last_opened_ms: None,
            })
        } else {
            Resolution::Candidate(Candidate {
                name: "work".into(),
                repo_path: repo.clone(),
                git_common_dir: None,
            })
        };
        let options = CoreOptions {
            driver: Arc::new(driver.clone()),
            exe: PathBuf::from("/usr/bin/dovetail"),
            tasks_dir: Some(paths.data.with_file_name("claude-tasks")),
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
                Ok(ServerMsg::Event { seq, event }) => {
                    apply(&mut self.snap, seq, event);
                }
                Err(_) => panic!("timed out waiting for {what}: {:#?}", self.snap),
            }
        }
    }

    /// Applies whatever arrives within `window`, for checking that something did not happen.
    fn drain(&mut self, window: Duration) {
        let until = Instant::now() + window;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match self.link.messages.recv_timeout(left) {
                Ok(ServerMsg::Snapshot(s)) => self.snap = s,
                Ok(ServerMsg::Event { seq, event }) => {
                    apply(&mut self.snap, seq, event);
                }
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
            .contains(&"mcp__dovetail__start_slice".to_string())
    );
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CODE_TASK_LIST_ID" && v == "dovetail-p-coordinator")
    );
    let mcp = std::fs::read_to_string(spec.mcp_config_path.as_ref().expect("mcp")).expect("read");
    assert!(mcp.contains("DOVETAIL_AGENT_ID") && mcp.contains("\"coordinator\""));

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

/// The header's first row, drawn by the real TUI from the core's snapshot, an hour before the
/// recorded 5-hour window resets.
fn header_row(snap: &Snapshot) -> String {
    let now_ms = 1_793_584_200_000 - 3_600_000;
    let mut app = dovetail_tui::App::new(jiff::tz::TimeZone::UTC);
    app.set_now(now_ms);
    app.on_server(ServerMsg::Hello {
        protocol: dovetail_api::PROTOCOL_VERSION,
    });
    app.on_server(ServerMsg::Snapshot(Snapshot {
        now_ms,
        ..snap.clone()
    }));
    let backend = ratatui::backend::TestBackend::new(120, 12);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.width)
        .map(|x| buffer[(x, 0)].symbol().to_string())
        .collect()
}

/// Task #26: Claude Code 2.1.319 can send a `rate_limit_event` with only the weekly window
/// (`rate_limit_week_only` holds the live line). Driver, core and header together show the 5-hour
/// figure when it is reported and `5h —` when it is not, never nothing.
#[test]
fn the_header_shows_the_5h_window_or_a_dash_when_the_event_leaves_it_out() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the usage limits", |s| s.usage.five_hour.is_some());
    let row = header_row(&h.snap);
    assert!(row.contains("5h 11%") && row.contains("week 17%"), "{row}");
    h.quit();

    let mut h = Harness::start(fixture("rate_limit_week_only"), true, Config::default());
    prompt(&h, "Reply with exactly the word: alpha");
    h.wait_for("the usage limits", |s| s.usage.seven_day.is_some());
    assert_eq!(h.snap.usage.five_hour, None);
    let row = header_row(&h.snap);
    assert!(row.contains("5h — · week 6%"), "{row}");
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
        let repo = repo.unwrap_or_else(|| dir.path().to_path_buf());
        let resolution = Resolution::Registered(Project {
            id: "p".into(),
            name: "work".into(),
            repo_path: repo.clone(),
            git_common_dir,
            last_opened_ms: None,
        });
        let options = CoreOptions {
            driver: Arc::new(driver.clone()),
            exe: PathBuf::from("/usr/bin/dovetail"),
            tasks_dir: Some(paths.data.with_file_name("claude-tasks")),
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
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let socket = h.socket();
    let start = |name: &str| {
        Tool::StartSlice(StartSlice {
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
    assert!(is_error && text.contains("coordinator only"), "{text}");
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
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    assert_eq!(h.snap.tasks[0].bucket, Some(Bucket::Next));
    h.send(Command::NewSlice {
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
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    assert!(
        !h.paths.data.join("projects").exists(),
        "nothing written yet"
    );
    // A slice may start before the folder is a project (decided 2026-11-06).
    h.send(Command::NewSlice {
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
        .find(|s| s.name.as_deref() == Some("after"))
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
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::StartSlice(StartSlice {
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
            .any(|i| matches!(&i.kind, InboxKind::Permission { tool, .. } if tool == "start_slice"))
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
                .any(|e| e.text.contains("start_slice later approved"))
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
            subagent_tokens: 0,
        };
        agent.status_line.clear();
        db.upsert_agent(dovetail_core::db::AgentRow {
            agent,
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
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
    assert_eq!(names(&fake), vec![("coordinator".to_string(), true)]);
    h.send(Command::Prompt {
        agent: AgentId("slice-later".into()),
        text: "carry on".into(),
        attachments: Vec::new(),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while names(&fake).len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", names(&fake));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(names(&fake)[1], ("later".to_string(), true));
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

/// Raises a pending `start_slice` approval: the coordinator to acceptEdits, a task, the call.
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

const EXPIRED: &str = "start_slice later expired at restart; call it again if still needed";

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
            subagent_tokens: 0,
        };
        db.upsert_agent(dovetail_core::db::AgentRow {
            agent,
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
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
    assert!(is_error && text.contains("coordinator only"), "{text}");
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
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    h.send(Command::NewSlice {
        task: Some(id),
        name: "work".into(),
        brief: "go".into(),
    });
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "work"));
    let socket = h.socket();
    let archive = Tool::ArchiveTask(ArchiveTask { id });

    let (text, is_error) = result_text(socket_call(&socket, tool("slice-work", archive.clone())));
    assert!(is_error && text.contains("coordinator only"), "{text}");

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
        }));
    }
    h.wait_for("the tasks", |s| s.tasks.len() == 4);
    // Task 3 gets a slice, which moves it to Now.
    h.send(Command::NewSlice {
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
        assert!(is_error && text.contains("coordinator only"), "{text}");
    }
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
    }));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    let list = tasks_dir(&h.paths).join(format!("dovetail-p-task-{}", id.0));
    write_item(&list, "1", "alpha", "pending");
    // The coordinator's list is not the task's and stays.
    let other = tasks_dir(&h.paths).join("dovetail-p-coordinator");
    write_item(&other, "1", "keep", "pending");
    h.send(Command::NewSlice {
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

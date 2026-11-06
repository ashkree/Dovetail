//! The real core driven by `FakeDriver` replaying recorded fixtures: the stream rules that are
//! cheap to replay (double `result`, concurrent permissions, subagent routing), the inbox round
//! trips, the tool handlers over the real socket, and ADR 0005's unregistered folders.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::{AskUser, ReportStatus, StartSlice, Tool};
use dovetail_api::{
    AgentId, AgentKind, Command, CoreLink, EntryKind, InboxKind, NewTask, PermissionAnswer,
    PermissionMode, ServerMsg, Snapshot, TaskId, apply,
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
    });
}

fn finished_turns(s: &Snapshot) -> usize {
    s.inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::FinishedTurn { .. }))
        .count()
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

impl Harness {
    /// A second app on the state of an earlier one (registered project `p`).
    fn start_on(driver: FakeDriver, paths: Paths) -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = tempfile::Builder::new()
            .prefix("dvt.")
            .tempdir_in("/tmp")
            .expect("short tempdir");
        let paths = Paths {
            runtime: run.path().to_path_buf(),
            ..paths
        };
        let repo = dir.path().to_path_buf();
        let resolution = Resolution::Registered(Project {
            id: "p".into(),
            name: "work".into(),
            repo_path: repo.clone(),
            git_common_dir: None,
            last_opened_ms: None,
        });
        let options = CoreOptions {
            driver: Arc::new(driver.clone()),
            exe: PathBuf::from("/usr/bin/dovetail"),
        };
        let (link, core) =
            dovetail_core::spawn(resolution, paths.clone(), Config::default(), options)
                .expect("core");
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
    let id = h.snap.inbox[0].id;
    h.send(Command::AnswerPermission {
        item: id,
        answer: PermissionAnswer::Deny { message: None },
    });
    h.wait_for("the reply", |s| finished_turns(s) == 1);
    h.quit();
}

#[test]
fn ask_user_blocks_until_answered_and_report_status_sets_the_line() {
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

    let asker = std::thread::spawn(move || {
        socket_call(
            &socket,
            tool(
                COORDINATOR,
                Tool::AskUser(AskUser {
                    question: "Which port?".into(),
                    blocking: true,
                }),
            ),
        )
    });
    h.wait_for("the question", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(&i.kind, InboxKind::Question { blocking: true, .. }))
    });
    let id = h
        .snap
        .inbox
        .iter()
        .find(|i| matches!(i.kind, InboxKind::Question { .. }))
        .unwrap()
        .id;
    h.send(Command::AnswerQuestion {
        item: id,
        text: "8080".into(),
    });
    let (text, _) = result_text(asker.join().expect("asker"));
    assert_eq!(text, "The user answered: 8080");
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
    h.send(Command::CloseSlice {
        agent: AgentId("slice-first".into()),
    });
    h.wait_for("the queued slice", |s| {
        s.agents.iter().any(|a| a.name == "second") && !s.agents.iter().any(|a| a.name == "first")
    });
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
    h.quit();
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

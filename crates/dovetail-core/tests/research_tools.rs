//! The Research suite's tools (R2, `docs/research.md` §6–§7) through the real core, the app
//! socket and `FakeDriver`: a round opened by tool, a report with citations that must resolve,
//! the specified errors, and the accept and drop flows of `research_propose`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::research::{
    Evidence, FindingId, ProposalKind, ResearchContent, ResearchWhat, SourceId,
};
use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::{
    ResearchGet, ResearchPropose, ResearchStartRound, ResearchWriteReport, Tool,
};
use dovetail_api::{
    AgentId, Alternative, Bucket, Command, CoreLink, Event, InboxKind, NewTask, PermissionAnswer,
    ServerMsg, Snapshot, TaskId, apply,
};
use dovetail_core::config::Config;
use dovetail_core::paths::Paths;
use dovetail_core::projects::{Project, Resolution};
use dovetail_core::{CoreHandle, CoreOptions};
use dovetail_driver::{FakeDriver, Sent};
use serde_json::Value;
use tempfile::TempDir;

const COORDINATOR: &str = "coordinator";

fn fixture(name: &str) -> FakeDriver {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures")
        .join(format!("{name}.ndjson"));
    FakeDriver::from_file(path).expect("fixture")
}

struct App {
    link: CoreLink,
    core: Option<CoreHandle>,
    snap: Snapshot,
    failures: Vec<String>,
    loaded: Vec<(TaskId, ResearchWhat, ResearchContent)>,
    fake: FakeDriver,
    runtime: TempDir,
}

impl App {
    fn start(fake: &FakeDriver, paths: &Paths, repo: &Path) -> App {
        let run = tempfile::Builder::new()
            .prefix("dvt.")
            .tempdir_in("/tmp")
            .expect("short tempdir");
        let paths = Paths {
            runtime: run.path().to_path_buf(),
            ..paths.clone()
        };
        let resolution = Resolution::Registered(Project {
            id: "p".into(),
            name: "work".into(),
            kind: dovetail_api::ProjectKind::Code,
            repo_path: Some(repo.to_path_buf()),
            git_common_dir: None,
            last_opened_ms: None,
        });
        let options = CoreOptions {
            driver: Arc::new(fake.clone()),
            exe: PathBuf::from("/usr/bin/dovetail"),
            tasks_dir: Some(paths.data.with_file_name("claude-tasks")),
            editor: None,
            claude: None,
            processes: None,
            memory_caps: false,
            check_claude: false,
            scopes: None,
            upgrade: None,
        };
        let (link, core) =
            dovetail_core::spawn(resolution, paths, Config::default(), options).expect("core");
        let mut app = App {
            link,
            core: Some(core),
            snap: Snapshot::default(),
            failures: Vec::new(),
            loaded: Vec::new(),
            fake: fake.clone(),
            runtime: run,
        };
        app.wait_for("the snapshot", |s| !s.agents.is_empty());
        app
    }

    fn send(&self, command: Command) {
        self.link.commands.send(command).expect("core is running");
    }

    fn fold(&mut self, message: ServerMsg) {
        match message {
            ServerMsg::Hello { .. } => {}
            ServerMsg::Snapshot(s) => self.snap = s,
            ServerMsg::Event { seq, event } => {
                match &event {
                    Event::CommandFailed { message } => self.failures.push(message.clone()),
                    Event::ResearchLoaded {
                        task,
                        what,
                        content,
                    } => self.loaded.push((*task, what.clone(), content.clone())),
                    _ => {}
                }
                apply(&mut self.snap, seq, event);
            }
        }
    }

    fn wait_for(&mut self, what: &str, done: impl Fn(&Snapshot) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&self.snap) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}: {:#?}", self.failures));
            match self.link.messages.recv_timeout(left) {
                Ok(m) => self.fold(m),
                Err(_) => panic!("timed out waiting for {what}: {:#?}", self.failures),
            }
        }
    }

    /// Folds in what arrives for `window`.
    fn drain(&mut self, window: Duration) {
        let until = Instant::now() + window;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match self.link.messages.recv_timeout(left) {
                Ok(m) => self.fold(m),
                Err(_) => return,
            }
        }
    }

    /// Waits until the coordinator was sent a user message containing `needle`.
    fn wait_for_note(&mut self, needle: &str) -> String {
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

    fn wait_for_proposal(&mut self) {
        self.wait_for("a proposal in Needs you", |s| {
            s.inbox
                .iter()
                .any(|i| matches!(i.kind, InboxKind::Proposal { .. }))
        });
    }

    fn socket(&self) -> PathBuf {
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.runtime.path())
            .expect("runtime dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        found.sort();
        found.pop().expect("a socket")
    }

    /// One tool call from the coordinator, as the bridge makes it.
    fn call(&self, tool: Tool) -> (String, bool) {
        let stream = UnixStream::connect(self.socket()).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let body = SocketRequestBody::ToolCall {
            agent: AgentId(COORDINATOR.into()),
            tool_use_id: None,
            call: tool,
        };
        let mut line = serde_json::to_string(&SocketRequest { id: 7, body }).expect("encode");
        line.push('\n');
        (&stream).write_all(line.as_bytes()).expect("write");
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply).expect("read");
        let response: SocketResponse = serde_json::from_str(&reply).expect("decode");
        match response.body {
            SocketResponseBody::ToolResult { text, is_error } => (text, is_error),
            other => panic!("not a tool result: {other:?}"),
        }
    }

    fn summary(&self, task: TaskId) -> Option<&dovetail_api::research::ResearchSummary> {
        self.snap.research.iter().find(|r| r.task == task)
    }

    fn quit(mut self) {
        self.send(Command::Quit { end_sessions: true });
        let core = self.core.take().expect("core handle");
        assert!(core.wait(Duration::from_secs(10)), "core stopped");
    }
}

fn paths(dir: &Path) -> Paths {
    Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: dir.join("run"),
    }
}

fn new_task(app: &mut App, title: &str) -> TaskId {
    let before = app.snap.tasks.len();
    app.send(Command::CreateTask(NewTask {
        title: title.into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
    }));
    app.wait_for("the task", |s| s.tasks.len() > before);
    app.snap
        .tasks
        .iter()
        .find(|t| t.title == title)
        .expect("task")
        .id
}

fn start_round(app: &App, task: TaskId, brief: &str) -> (String, bool) {
    app.call(Tool::ResearchStartRound(ResearchStartRound {
        task,
        brief: brief.into(),
    }))
}

fn write_report(app: &App, task: TaskId, body: &str) -> (String, bool) {
    app.call(Tool::ResearchWriteReport(ResearchWriteReport {
        task,
        body: body.into(),
    }))
}

fn propose(
    app: &App,
    task: TaskId,
    kind: ProposalKind,
    text: &str,
    evidence: &[&str],
) -> (String, bool) {
    app.call(Tool::ResearchPropose(ResearchPropose {
        task,
        kind,
        text: text.into(),
        evidence: evidence.iter().map(|e| (*e).to_owned()).collect(),
        why: Some("because".into()),
        alternatives: vec![Alternative {
            option: "the other way".into(),
            why_not: Some("slower".into()),
        }],
    }))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The JSON on the first line of a tool result.
fn head(text: &str) -> Value {
    serde_json::from_str(text.lines().next().unwrap_or("")).unwrap_or_else(|e| {
        panic!("no JSON head ({e}) in {text}");
    })
}

fn open_repo(dir: &Path) -> (PathBuf, Paths) {
    let repo = dir.join("work");
    std::fs::create_dir_all(&repo).unwrap();
    (repo, paths(dir))
}

/// A task with one round run through the S7 recording: one finding (F-id, first listed) that
/// reached two sources. The round is still open.
struct Researched {
    app: App,
    task: TaskId,
    finding: i64,
    sources: Vec<i64>,
}

fn researched(dir: &Path) -> Researched {
    let (repo, paths) = open_repo(dir);
    let mut app = App::start(&fixture("research_round_foreground"), &paths, &repo);
    let task = new_task(&mut app, "Example research");
    let (text, is_error) = start_round(&app, task, "What is the codeword?");
    assert!(!is_error, "{text}");
    app.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "go".into(),
        attachments: Vec::new(),
    });
    app.wait_for("the finding", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.findings == 1 && r.sources == 2)
    });
    app.send(Command::LoadResearch {
        task,
        what: ResearchWhat::Listing,
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.loaded.is_empty() {
        assert!(Instant::now() < deadline, "no listing");
        app.drain(Duration::from_millis(20));
    }
    let Some((_, _, ResearchContent::Listing(listing))) = app.loaded.first() else {
        panic!("no listing: {:?}", app.loaded);
    };
    let finding = listing.findings[0].id.0;
    let sources = listing.sources.iter().map(|s| s.id.0).collect();
    Researched {
        app,
        task,
        finding,
        sources,
    }
}

fn answer(app: &mut App, answer: PermissionAnswer) {
    let item = app
        .snap
        .inbox
        .iter()
        .find(|i| matches!(i.kind, InboxKind::Proposal { .. }))
        .expect("a proposal in Needs you")
        .id;
    app.send(Command::AnswerPermission { item, answer });
    app.wait_for("the proposal to leave Needs you", |s| {
        !s.inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::Proposal { .. }))
    });
}

#[test]
fn a_round_by_tool_and_a_report_write_brief_and_report_and_every_citation_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let Researched {
        mut app,
        task,
        finding,
        sources,
    } = researched(dir.path());
    let summary = app.summary(task).expect("summary").clone();
    let round = &summary.rounds[0];
    assert!(round.closed_ms.is_none());
    assert!(read(&summary.dir.join(&round.dir).join("brief.md")).contains("What is the codeword?"));

    let body = format!(
        "# Codeword\n\nThe codeword is MARIGOLD-42 [F{finding}], read from [S{}] and [S{}].\n\n\
         **Evidence:** 40 sources, 40 independent\n",
        sources[0], sources[1]
    );
    let (text, is_error) = write_report(&app, task, &body);
    assert!(!is_error, "{text}");
    let head = head(&text);
    assert_eq!(head["written"], true, "{text}");
    assert_eq!(head["citations_resolved"], 3, "{text}");
    assert_eq!(head["uncited"], serde_json::json!([]), "{text}");

    // report.md holds the body with Dovetail's own count, and the round is closed.
    let report = read(&summary.dir.join("report.md"));
    assert!(
        report.contains("**Evidence:** 2 sources, 2 independent"),
        "{report}"
    );
    assert!(!report.contains("40"), "{report}");
    app.wait_for("the round to close", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.open_round().is_none() && r.report_ms.is_some())
    });

    // research_get reads it back, and a finding and a source by the ids the report cites.
    let (got, is_error) = app.call(Tool::ResearchGet(ResearchGet {
        task,
        what: "report".into(),
    }));
    assert!(!is_error, "{got}");
    assert_eq!(got, report);
    let (got, is_error) = app.call(Tool::ResearchGet(ResearchGet {
        task,
        what: format!("finding:{finding}"),
    }));
    assert!(!is_error && got.contains("MARIGOLD-42"), "{got}");
    let (got, is_error) = app.call(Tool::ResearchGet(ResearchGet {
        task,
        what: format!("source:{}", sources[0]),
    }));
    assert!(!is_error && got.contains("MARIGOLD-42"), "{got}");
    let (got, _) = app.call(Tool::ResearchGet(ResearchGet {
        task,
        what: "round:1".into(),
    }));
    assert!(got.contains("What is the codeword?"), "{got}");

    // A second report rewrites the first; the earlier round is enough to allow it.
    let (text, is_error) = write_report(&app, task, "# Codeword again\n\nNothing new.\n");
    assert!(!is_error, "{text}");
    assert_eq!(
        read(&summary.dir.join("report.md")),
        "# Codeword again\n\nNothing new.\n"
    );
    assert!(app.failures.is_empty(), "{:?}", app.failures);
    app.quit();
}

#[test]
fn the_specified_errors_are_returned_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    let task = new_task(&mut app, "Question");

    // No open round and no earlier round: nothing to report on.
    let (text, is_error) = write_report(&app, task, "# Report\n");
    assert!(
        is_error && text.contains("no open round and no earlier round"),
        "{text}"
    );

    // Start-round errors.
    let (text, is_error) = start_round(&app, TaskId(99), "brief");
    assert!(is_error && text.contains("there is no task #99"), "{text}");
    let (text, is_error) = start_round(&app, task, "  ");
    assert!(
        is_error && text.contains("`brief` must be a non-empty string"),
        "{text}"
    );
    let (text, is_error) = start_round(&app, task, "criteria");
    assert!(!is_error, "{text}");
    assert_eq!(head(&text)["status"], "open");
    assert_eq!(head(&text)["round"], 1);
    let (text, is_error) = start_round(&app, task, "again");
    assert!(
        is_error && text.contains("a round is already open for task"),
        "{text}"
    );

    // An unresolved citation refuses the write, lists what exists, and leaves the round open.
    let (text, is_error) = write_report(&app, task, "A claim [F9].\n");
    assert!(
        is_error && text.contains("[F9] does not resolve; findings in this task: none"),
        "{text}"
    );
    app.wait_for("the open round", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.open_round().is_some())
    });
    let dir_of_task = app.summary(task).expect("summary").dir.clone();
    assert!(!dir_of_task.join("report.md").exists());
    assert!(app.summary(task).unwrap().open_round().is_some());
    assert!(app.summary(task).unwrap().report_ms.is_none());
    let (text, is_error) = app.call(Tool::ResearchGet(ResearchGet {
        task,
        what: "report".into(),
    }));
    assert!(is_error && text.contains("has no report yet"), "{text}");

    // A decision without evidence, with a bad id, and with an id of no finding.
    let (text, is_error) = propose(&app, task, ProposalKind::Decision, "Use SQLite", &[]);
    assert!(is_error && text.contains("needs `evidence`"), "{text}");
    let (text, is_error) = propose(&app, task, ProposalKind::Decision, "Use SQLite", &["x1"]);
    assert!(
        is_error && text.contains("not a finding or source id"),
        "{text}"
    );
    let (text, is_error) = propose(&app, task, ProposalKind::Decision, "Use SQLite", &["F9"]);
    assert!(
        is_error && text.contains("F9 does not resolve; findings in this task: none"),
        "{text}"
    );
    let (text, is_error) = propose(&app, TaskId(99), ProposalKind::Idea, "Look at X", &[]);
    assert!(is_error && text.contains("there is no task #99"), "{text}");
    app.drain(Duration::from_millis(100));
    assert!(
        !app.snap
            .inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::Proposal { .. })),
        "a refused proposal raised nothing"
    );

    // An archived task takes no round.
    app.send(Command::UpdateTask {
        id: task,
        patch: vec![dovetail_api::TaskChange::Status(
            dovetail_api::TaskStatus::Archived,
        )],
    });
    app.wait_for("the archive", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == task && t.status == dovetail_api::TaskStatus::Archived)
    });
    let (text, is_error) = start_round(&app, task, "criteria");
    assert!(is_error && text.contains("archived"), "{text}");
    app.quit();
}

#[test]
fn an_accepted_decision_is_logged_with_its_evidence_and_never_before() {
    let dir = tempfile::tempdir().unwrap();
    let Researched {
        mut app,
        task,
        finding,
        sources,
    } = researched(dir.path());
    let f = format!("F{finding}");
    let s = format!("S{}", sources[0]);
    let (text, is_error) = propose(
        &app,
        task,
        ProposalKind::Decision,
        "The codeword is MARIGOLD-42",
        &[&f, &s],
    );
    assert!(!is_error && text.starts_with("pending"), "{text}");
    app.wait_for_proposal();
    // Nothing is written until `y`.
    app.drain(Duration::from_millis(200));
    assert!(app.snap.decisions.is_empty());

    answer(&mut app, PermissionAnswer::Allow);
    app.wait_for("the decision", |s| s.decisions.len() == 1);
    let d = &app.snap.decisions[0];
    assert_eq!(d.task, task);
    assert_eq!(d.text, "The codeword is MARIGOLD-42");
    assert_eq!(d.why.as_deref(), Some("because"));
    assert_eq!(d.alternatives.len(), 1);
    assert_eq!(
        d.evidence,
        [
            Evidence::Finding(FindingId(finding)),
            Evidence::Source(SourceId(sources[0]))
        ]
    );
    let note = app.wait_for_note("research_propose accepted");
    assert!(note.contains("decision D1"), "{note}");

    // get_task shows the links to the coordinator too.
    let (text, _) = app.call(Tool::GetTask(dovetail_api::tools::GetTask { id: task }));
    assert!(text.contains(&format!("Evidence: {f}, {s}")), "{text}");

    // The evidence survives a restart.
    let paths = paths(dir.path());
    app.quit();
    let repo = dir.path().join("work");
    let app = App::start(&fixture("plain_turn"), &paths, &repo);
    assert_eq!(app.snap.decisions.len(), 1);
    assert_eq!(app.snap.decisions[0].evidence.len(), 2);
    app.quit();
}

#[test]
fn a_dropped_proposal_writes_nothing_and_an_accepted_idea_files_a_follow_up() {
    let dir = tempfile::tempdir().unwrap();
    let Researched {
        mut app,
        task,
        finding,
        ..
    } = researched(dir.path());
    let tasks_before = app.snap.tasks.len();
    let f = format!("F{finding}");

    // Dropped: no decision, no task, and the coordinator hears so.
    let (text, _) = propose(&app, task, ProposalKind::Decision, "Use X", &[&f]);
    assert!(text.starts_with("pending"), "{text}");
    app.wait_for_proposal();
    answer(&mut app, PermissionAnswer::Deny { message: None });
    let note = app.wait_for_note("research_propose dropped");
    assert!(note.contains("Use X"), "{note}");
    app.drain(Duration::from_millis(200));
    assert!(app.snap.decisions.is_empty());
    assert_eq!(app.snap.tasks.len(), tasks_before);

    // An idea accepted with `y` is a follow-up task in Unsorted.
    let (text, _) = propose(
        &app,
        task,
        ProposalKind::Idea,
        "What do competitors charge?",
        &[],
    );
    assert!(text.starts_with("pending"), "{text}");
    app.wait_for_proposal();
    assert_eq!(app.snap.tasks.len(), tasks_before, "not before y");
    answer(&mut app, PermissionAnswer::Allow);
    app.wait_for("the follow-up", |s| s.tasks.len() == tasks_before + 1);
    let follow_up = app
        .snap
        .tasks
        .iter()
        .find(|t| t.title == "What do competitors charge?")
        .expect("follow-up task");
    assert_eq!(follow_up.follow_up_of, Some(task));
    assert_eq!(follow_up.bucket, None::<Bucket>);
    assert!(!follow_up.coordinator_created);
    let note = app.wait_for_note("research_propose accepted: created task");
    assert!(note.contains("follow-up of"), "{note}");

    // The round is still open, so an idea cannot open a new one yet; after the report it can.
    let (text, _) = write_report(&app, task, "# Done\n");
    assert!(text.contains("The open round is closed"), "{text}");
    let (text, _) = propose(
        &app,
        task,
        ProposalKind::Idea,
        "Check the enterprise tier",
        &[],
    );
    assert!(text.starts_with("pending"), "{text}");
    app.wait_for_proposal();
    answer(&mut app, PermissionAnswer::AllowAlways);
    app.wait_for("round 2", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.rounds.len() == 2 && r.open_round().is_some())
    });
    let summary = app.summary(task).unwrap();
    assert_eq!(summary.rounds[1].brief, "Check the enterprise tier");
    assert!(app.failures.is_empty(), "{:?}", app.failures);
    app.quit();
}

#[test]
fn a_pending_proposal_expires_at_restart_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    let task = new_task(&mut app, "Question");
    let (text, _) = propose(
        &app,
        task,
        ProposalKind::Idea,
        "Look at the pricing page",
        &[],
    );
    assert!(text.starts_with("pending"), "{text}");
    app.wait_for_proposal();
    app.quit();

    let fake = fixture("plain_turn");
    let mut app = App::start(&fake, &paths, &repo);
    assert!(
        !app.snap
            .inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::Proposal { .. }))
    );
    app.drain(Duration::from_millis(300));
    assert!(
        app.snap
            .tasks
            .iter()
            .all(|t| t.title != "Look at the pricing page")
    );
    app.quit();
}

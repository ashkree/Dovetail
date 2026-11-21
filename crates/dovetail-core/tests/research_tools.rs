//! The Research suite's tools (R2, `docs/research.md` §6–§7) through the real core, the app
//! socket and `FakeDriver`: a round opened by tool, a report with citations that must resolve,
//! the specified errors, and the accept and drop flows of `research_propose`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::{
    ArchiveTask, ResearchGet, ResearchPropose, ResearchStartRound, ResearchWriteReport, Tool,
};
use dovetail_api::{
    Command, CoreLink, Event, ServerMsg, Snapshot,
    agent::AgentId,
    apply,
    decision::Alternative,
    inbox::{InboxKind, PermissionAnswer},
    task::{Bucket, NewTask, TaskId},
};
use dovetail_api::{
    app::{AppCommand, AppEvent},
    conversation::ConversationCommand,
    inbox::InboxCommand,
    library::{LibraryCommand, LibraryEvent},
    research::{
        Evidence, FindingId, ProposalKind, ResearchCommand, ResearchContent, ResearchEvent,
        ResearchKind, ResearchWhat, SourceId,
    },
    task::TaskCommand,
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
        let resolution =
            Resolution::Registered(Project::with_repo("p", "work", repo.to_path_buf(), None));
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
            // `gx` (ADR 0018): a script the reading-mode test writes, next to the data folder.
            opener: Some(paths.data.with_file_name("opener.sh").into()),
            gh: None,
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
                    Event::App(AppEvent::CommandFailed { message }) => {
                        self.failures.push(message.clone())
                    }
                    Event::Research(ResearchEvent::Loaded {
                        task,
                        what,
                        content,
                    }) => self.loaded.push((*task, what.clone(), content.clone())),
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
            called_as: None,
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
        self.send(Command::App(AppCommand::Quit { end_sessions: true }));
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
    app.send(Command::Task(TaskCommand::Create(NewTask {
        title: title.into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
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
        kind: None,
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
    app.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "go".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    app.wait_for("the finding", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.findings == 1 && r.sources == 2)
    });
    app.send(Command::Research(ResearchCommand::Load {
        task,
        what: ResearchWhat::Listing,
    }));
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
    app.send(Command::Inbox(InboxCommand::AnswerPermission {
        item,
        answer,
    }));
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
    // A second request while a round is open is queued, not refused.
    let (text, is_error) = start_round(&app, task, "again");
    assert!(!is_error, "{text}");
    assert_eq!(head(&text)["status"], "queued");
    assert_eq!(head(&text)["position"], 2);

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
    app.send(Command::Task(TaskCommand::Update {
        id: task,
        patch: vec![dovetail_api::task::TaskChange::Status(
            dovetail_api::task::TaskStatus::Archived,
        )],
    }));
    app.wait_for("the archive", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == task && t.status == dovetail_api::task::TaskStatus::Archived)
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

    // tasks_get shows the links to the coordinator too.
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

// --- follow-ups of 2026-11-11: the queue, archiving, unfiled findings, kind ----------------

fn brief_file(app: &App, task: TaskId, n: u32) -> PathBuf {
    let summary = app.summary(task).expect("summary");
    let round = summary.rounds.iter().find(|r| r.n == n).expect("round");
    summary.dir.join(&round.dir).join("brief.md")
}

fn activity_has(app: &App, needle: &str) -> bool {
    app.snap.activity.iter().any(|a| a.text.contains(needle))
}

#[test]
fn a_round_requested_while_one_is_open_is_queued_and_opens_in_request_order() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    let a = new_task(&mut app, "Alpha");
    let b = new_task(&mut app, "Beta");
    let c = new_task(&mut app, "Gamma");

    let (text, is_error) = start_round(&app, a, "alpha criteria");
    assert!(!is_error && head(&text)["status"] == "open", "{text}");
    // Others wait: brief.md is written at once and the call says where they stand.
    let (text, is_error) = start_round(&app, b, "beta criteria");
    assert!(!is_error, "{text}");
    assert_eq!(head(&text)["status"], "queued");
    assert_eq!(head(&text)["position"], 2);
    let (text, _) = start_round(&app, c, "gamma criteria");
    assert_eq!(head(&text)["position"], 3);
    let (text, _) = start_round(&app, a, "alpha again");
    assert_eq!(head(&text)["position"], 4);
    assert_eq!(head(&text)["round"], 2);
    app.wait_for("the queue in the snapshot", |s| {
        s.research
            .iter()
            .filter(|r| r.queued_rounds().count() > 0)
            .count()
            == 3
    });
    assert!(read(&brief_file(&app, b, 1)).contains("beta criteria"));
    assert!(read(&brief_file(&app, a, 2)).contains("alpha again"));
    assert!(app.summary(b).unwrap().open_round().is_none());
    assert_eq!(
        dovetail_api::research::queue_position(&app.snap.research, c, 1),
        Some(3)
    );
    assert!(activity_has(&app, "queued on #") && activity_has(&app, "(2nd in line)"));

    // Closing the open round opens the next in request order and tells the coordinator.
    app.send(Command::Research(ResearchCommand::CloseRound { task: a }));
    let note = app.wait_for_note("is now open");
    assert_eq!(
        note,
        format!("Round 1 for #{} is now open; send its subagents.", b.0)
    );
    app.wait_for("beta to open", |s| {
        s.research
            .iter()
            .any(|r| r.task == b && r.open_round().is_some())
    });
    assert_eq!(
        dovetail_api::research::queue_position(&app.snap.research, c, 1),
        Some(2)
    );

    // A queued round can be dropped: its row and brief.md go, and the line moves up.
    let gamma_brief = brief_file(&app, c, 1);
    app.send(Command::Research(ResearchCommand::DropRound {
        task: c,
        round: None,
    }));
    app.wait_for("gamma's drop", |s| {
        s.research
            .iter()
            .find(|r| r.task == c)
            .is_none_or(|r| r.rounds.is_empty())
    });
    assert!(!gamma_brief.exists());
    // An open round cannot be dropped.
    app.send(Command::Research(ResearchCommand::DropRound {
        task: b,
        round: None,
    }));
    app.drain(Duration::from_millis(100));
    assert!(
        app.failures.iter().any(|f| f.contains("no queued round")),
        "{:?}",
        app.failures
    );

    app.send(Command::Research(ResearchCommand::CloseRound { task: b }));
    app.wait_for_note(&format!("Round 2 for #{} is now open", a.0));
    app.wait_for("alpha round 2 to open", |s| {
        s.research.iter().any(|r| {
            r.task == a
                && r.open_round().is_some_and(|o| o.n == 2)
                && r.queued_rounds().count() == 0
        })
    });
    app.quit();
}

#[test]
fn the_queue_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    let a = new_task(&mut app, "Alpha");
    let b = new_task(&mut app, "Beta");
    let c = new_task(&mut app, "Gamma");
    start_round(&app, a, "alpha criteria");
    start_round(&app, b, "beta criteria");
    start_round(&app, c, "gamma criteria");
    app.wait_for("the queue", |s| {
        s.research
            .iter()
            .map(|r| r.queued_rounds().count())
            .sum::<usize>()
            == 2
    });
    app.quit();

    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    app.wait_for("the queue after the restart", |s| {
        s.research
            .iter()
            .map(|r| r.queued_rounds().count())
            .sum::<usize>()
            == 2
    });
    assert!(app.summary(a).unwrap().open_round().is_some());
    assert_eq!(
        dovetail_api::research::queue_position(&app.snap.research, b, 1),
        Some(2)
    );
    assert_eq!(
        dovetail_api::research::queue_position(&app.snap.research, c, 1),
        Some(3)
    );
    // A new request still queues behind both, and closing opens them in order.
    let (text, _) = start_round(&app, a, "alpha again");
    assert_eq!(head(&text)["position"], 4);
    app.send(Command::Research(ResearchCommand::CloseRound { task: a }));
    app.wait_for_note(&format!("Round 1 for #{} is now open", b.0));
    app.send(Command::Research(ResearchCommand::CloseRound { task: b }));
    app.wait_for_note(&format!("Round 1 for #{} is now open", c.0));
    app.quit();
}

#[test]
fn archiving_a_task_closes_its_round_drops_its_queue_and_opens_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    let a = new_task(&mut app, "Alpha");
    let b = new_task(&mut app, "Beta");
    start_round(&app, a, "alpha criteria");
    start_round(&app, b, "beta criteria");
    start_round(&app, a, "alpha again");
    app.wait_for("the queue", |s| {
        s.research
            .iter()
            .map(|r| r.queued_rounds().count())
            .sum::<usize>()
            == 2
    });
    let alpha_second = brief_file(&app, a, 2);
    assert!(alpha_second.exists());

    // The coordinator may not do it on the user's behalf.
    let (text, is_error) = app.call(Tool::ArchiveTask(ArchiveTask { id: a }));
    assert!(
        is_error && text.contains("an open research round"),
        "{text}"
    );

    app.send(Command::Task(TaskCommand::Archive { id: a }));
    app.wait_for("alpha to be archived", |s| {
        s.tasks
            .iter()
            .any(|t| t.id == a && t.status == dovetail_api::task::TaskStatus::Archived)
    });
    // Beta's round opens in its place; alpha's round 1 is closed, round 2 is gone.
    app.wait_for_note(&format!("Round 1 for #{} is now open", b.0));
    app.wait_for("beta to open and alpha to settle", |s| {
        let alpha = s.research.iter().find(|r| r.task == a);
        s.research
            .iter()
            .any(|r| r.task == b && r.open_round().is_some())
            && alpha.is_some_and(|r| {
                r.rounds.len() == 1 && r.rounds[0].closed_ms.is_some() && !r.rounds[0].queued
            })
    });
    assert!(!alpha_second.exists());
    assert!(activity_has(&app, "closed because #"));
    assert!(activity_has(&app, "dropped from the queue"));
    app.quit();
}

/// The coordinator researches with no round open: the S7 recording's subagent fetches and reads.
fn unfiled(dir: &Path) -> (App, PathBuf, PathBuf) {
    let (repo, paths) = open_repo(dir);
    let mut app = App::start(&fixture("research_round_foreground"), &paths, &repo);
    app.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "go".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    app.wait_for("an unfiled finding in Needs you", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::UnfiledFinding { .. }))
    });
    let root = paths.data.join("projects/p/research");
    (app, root, repo)
}

fn unfiled_item(app: &App) -> (dovetail_api::inbox::InboxId, String, u32) {
    app.snap
        .inbox
        .iter()
        .find_map(|i| match &i.kind {
            InboxKind::UnfiledFinding { label, sources, .. } => {
                Some((i.id, label.clone(), *sources))
            }
            _ => None,
        })
        .expect("an unfiled finding")
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

#[test]
fn a_coordinator_finding_with_no_round_is_unfiled_and_filing_moves_it_and_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let (app, root, repo) = unfiled(dir.path());
    let (item, label, sources) = unfiled_item(&app);
    assert_eq!(sources, 2);
    assert!(label.len() > 2, "{label}");
    // Stored with no task: files sit under unfiled/, and nothing is filed under any task.
    let before = files_under(&root.join("unfiled"));
    assert!(
        before
            .iter()
            .any(|p| p.ends_with(format!("findings/{}", before_name(&before)))),
        "{before:?}"
    );
    assert!(
        before
            .iter()
            .any(|p| p.to_string_lossy().contains("/sources/"))
    );
    assert!(app.snap.research.iter().all(|r| r.findings == 0));
    assert!(activity_has(&app, "unfiled finding"));

    // It survives a restart, still waiting in Needs you.
    app.quit();
    let (_, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    app.wait_for("the item after the restart", |s| {
        s.inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::UnfiledFinding { .. }))
    });
    assert_eq!(unfiled_item(&app).2, 2);
    let task = new_task(&mut app, "Pricing research");

    // Filing under a task moves the row and its files to the task's research folder.
    app.send(Command::Research(ResearchCommand::FileUnfiled {
        item,
        task,
    }));
    app.wait_for("the item to leave Needs you", |s| {
        !s.inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::UnfiledFinding { .. }))
    });
    app.wait_for("the finding under the task", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.findings == 1 && r.quick_findings == 1 && r.sources == 2)
    });
    let task_dir = app.summary(task).unwrap().dir.clone();
    let filed = files_under(&task_dir);
    assert!(
        filed
            .iter()
            .any(|p| p.parent().is_some_and(|d| d.ends_with("findings"))
                && p.to_string_lossy().contains(&label)),
        "{filed:?}"
    );
    assert_eq!(
        filed
            .iter()
            .filter(|p| p.to_string_lossy().contains("/sources/"))
            .count(),
        2,
        "{filed:?}"
    );
    assert_eq!(files_under(&root.join("unfiled")), Vec::<PathBuf>::new());
    assert!(!root.join("unfiled").exists());
    // The finding opens like any other, by the id it has now.
    app.send(Command::Research(ResearchCommand::Load {
        task,
        what: ResearchWhat::Listing,
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.loaded.is_empty() {
        assert!(Instant::now() < deadline, "no listing");
        app.drain(Duration::from_millis(20));
    }
    let Some((_, _, ResearchContent::Listing(listing))) = app.loaded.first() else {
        panic!("{:?}", app.loaded);
    };
    assert_eq!(listing.findings.len(), 1);
    assert_eq!(listing.findings[0].round, None);
    assert_eq!(listing.findings[0].sources.len(), 2);
    app.quit();
}

fn before_name(files: &[PathBuf]) -> String {
    files
        .iter()
        .find(|p| p.parent().is_some_and(|d| d.ends_with("findings")))
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[test]
fn dropping_an_unfiled_finding_deletes_it_and_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, root, _repo) = unfiled(dir.path());
    let (item, _, _) = unfiled_item(&app);
    assert!(!files_under(&root.join("unfiled")).is_empty());
    app.send(Command::Inbox(InboxCommand::Dismiss { item }));
    app.wait_for("the item to leave Needs you", |s| {
        !s.inbox
            .iter()
            .any(|i| matches!(i.kind, InboxKind::UnfiledFinding { .. }))
    });
    app.drain(Duration::from_millis(200));
    assert!(files_under(&root.join("unfiled")).is_empty());
    assert!(activity_has(&app, "unfiled finding") && activity_has(&app, "dropped"));
    // Filing something already dropped is a visible failure, not a silent no-op.
    let task = new_task(&mut app, "Later");
    app.send(Command::Research(ResearchCommand::FileUnfiled {
        item,
        task,
    }));
    app.drain(Duration::from_millis(200));
    assert!(!app.failures.is_empty());
    assert!(app.summary(task).is_none_or(|r| r.findings == 0));
    app.quit();
}

#[test]
fn the_kind_comes_from_the_round_or_the_user_and_a_decision_is_held_to_its_template() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let mut app = App::start(&fixture("plain_turn"), &paths, &repo);
    let task = new_task(&mut app, "Should we?");
    let (text, is_error) = app.call(Tool::ResearchStartRound(ResearchStartRound {
        task,
        brief: "criteria".into(),
        kind: Some(ResearchKind::Decision),
    }));
    assert!(!is_error, "{text}");
    app.wait_for("the kind", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.kind == Some(ResearchKind::Decision))
    });
    // The decision template check runs: a plain report is refused for its missing sections.
    let (text, is_error) = write_report(&app, task, "# Should we?\n\nYes.\n");
    assert!(
        is_error && text.contains("Key figures") && text.contains("Options"),
        "{text}"
    );
    // The user overrides it, and the same report is written.
    app.send(Command::Research(ResearchCommand::SetKind {
        task,
        kind: ResearchKind::Question,
    }));
    app.wait_for("the override", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.kind == Some(ResearchKind::Question))
    });
    let (text, is_error) = write_report(&app, task, "# Should we?\n\nYes.\n");
    assert!(!is_error, "{text}");
    app.quit();
}

// --- Reading mode (M21, ADR 0018) ---

/// Waits for the next `CommandFailed` containing `needle`.
fn wait_for_failure(app: &mut App, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.failures.iter().any(|f| f.contains(needle)) {
        assert!(
            Instant::now() < deadline,
            "no failure with {needle:?}: {:?}",
            app.failures
        );
        app.drain(Duration::from_millis(20));
    }
}

#[test]
fn ask_is_a_read_only_session_in_the_research_folder_grounded_in_the_report() {
    use dovetail_api::{agent::AgentKind, conversation::EntryKind};
    let dir = tempfile::tempdir().unwrap();
    let Researched {
        mut app,
        task,
        finding,
        sources,
    } = researched(dir.path());
    // No report yet: nothing to ask about.
    app.send(Command::Research(ResearchCommand::StartAsk { task }));
    wait_for_failure(&mut app, "no report yet");
    app.failures.clear();
    let body = format!(
        "# Codeword\n\nThe codeword is MARIGOLD-42 [F{finding}], read from [S{}].\n",
        sources[0]
    );
    let (text, is_error) = write_report(&app, task, &body);
    assert!(!is_error, "{text}");

    // `i`: the agent is made, nothing is started yet.
    let starts = app.fake.started_specs().len();
    app.send(Command::Research(ResearchCommand::StartAsk { task }));
    let ask = AgentId(format!("ask-{}", task.0));
    app.wait_for("the Ask agent", |s| s.agents.iter().any(|a| a.id == ask));
    let agent = app.snap.agents.iter().find(|a| a.id == ask).expect("ask");
    assert_eq!(agent.kind, AgentKind::Ask);
    assert_eq!(agent.task, Some(task));
    assert_eq!(app.fake.started_specs().len(), starts);

    // The first question starts it in the research folder, read-only, with no Dovetail suites,
    // and carries the report and the index; the conversation shows only the question.
    app.send(Command::Conversation(ConversationCommand::Prompt {
        agent: ask.clone(),
        text: "Where does the codeword come from?".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    let sent = app.wait_for_note("## The report");
    assert!(sent.contains("MARIGOLD-42"), "{sent}");
    assert!(sent.contains(&format!("- F{finding} `")), "{sent}");
    assert!(sent.contains(&format!("- S{} ", sources[0])), "{sent}");
    assert!(
        sent.ends_with("## Question\n\nWhere does the codeword come from?"),
        "{sent}"
    );
    let spec = app
        .fake
        .started_specs()
        .into_iter()
        .find(|s| {
            s.name
                .as_deref()
                .is_some_and(|n| n.ends_with(&format!(".{}", ask.0)))
        })
        .expect("the Ask session started");
    let research_dir = app.summary(task).expect("summary").dir.clone();
    assert_eq!(spec.cwd, research_dir);
    assert_eq!(
        spec.permission_mode,
        dovetail_driver::PermissionMode::DontAsk
    );
    assert!(
        !spec
            .claude
            .allowed_tools
            .iter()
            .any(|t| t.starts_with("mcp__dovetail__")),
        "{:?}",
        spec.claude.allowed_tools
    );
    assert!(
        spec.claude
            .allowed_tools
            .contains(&"Bash(rg *)".to_string())
    );
    let denied = spec.claude.extra_args.join(" ");
    assert!(
        denied.contains("--disallowedTools") && denied.contains("Write"),
        "{denied}"
    );
    app.wait_for("the question in the conversation", |s| {
        s.conversations
            .get(&ask)
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::User))
    });
    let shown: Vec<String> = app.snap.conversations[&ask]
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::User)
        .map(|e| e.text.clone())
        .collect();
    assert_eq!(shown, ["Where does the codeword come from?"]);
    // The question went to the Ask session only, with its grounding.
    let plain_question = app
        .fake
        .sent()
        .into_iter()
        .any(|s| matches!(s, Sent::User(t) if t == "Where does the codeword come from?"));
    assert!(!plain_question, "{:#?}", app.fake.sent());

    // A new report closes it: the next question starts on the new one.
    let (text, is_error) = write_report(&app, task, "# Codeword again\n\nNothing new.\n");
    assert!(!is_error, "{text}");
    app.wait_for("the Ask session to close", |s| {
        !s.agents.iter().any(|a| a.id == ask)
    });
    app.quit();
}

#[test]
fn open_external_runs_the_opener_on_web_pages_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    let opened = dir.path().join("opened.txt");
    let script = paths.data.with_file_name("opener.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", opened.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut app = App::start(&fixture("research_round_foreground"), &paths, &repo);
    app.send(Command::App(AppCommand::OpenExternal {
        url: "file:///etc/passwd".into(),
    }));
    wait_for_failure(&mut app, "only web pages");
    assert!(!opened.exists());
    app.send(Command::App(AppCommand::OpenExternal {
        url: "https://docs.rs/nucleo".into(),
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !opened.exists() || read(&opened).is_empty() {
        assert!(Instant::now() < deadline, "the opener did not run");
        app.drain(Duration::from_millis(20));
    }
    assert_eq!(read(&opened), "https://docs.rs/nucleo");
    app.quit();
}

#[test]
fn the_repo_docs_are_listed_and_read_and_nothing_outside_them() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, paths) = open_repo(dir.path());
    std::fs::create_dir_all(repo.join("docs/adr")).unwrap();
    std::fs::write(
        repo.join("docs/adr/0018-reading.md"),
        "# Reading mode\n\nText.\n",
    )
    .unwrap();
    std::fs::write(repo.join("README.md"), "# Not a doc\n").unwrap();
    let mut app = App::start(&fixture("research_round_foreground"), &paths, &repo);
    app.send(Command::Library(LibraryCommand::ListDocs));
    app.send(Command::Library(LibraryCommand::LoadDoc {
        path: "adr/0018-reading.md".into(),
    }));
    app.send(Command::Library(LibraryCommand::LoadDoc {
        path: "../README.md".into(),
    }));
    let mut docs = None;
    let mut doc = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while (docs.is_none() || doc.is_none()) && Instant::now() < deadline {
        let Ok(message) = app.link.messages.recv_timeout(Duration::from_millis(50)) else {
            continue;
        };
        if let ServerMsg::Event { event, .. } = &message {
            match event {
                Event::Library(LibraryEvent::DocsListed { docs: d }) => docs = Some(d.clone()),
                Event::Library(LibraryEvent::DocLoaded { document, .. }) => {
                    doc = Some(document.markdown.clone())
                }
                _ => {}
            }
        }
        app.fold(message);
    }
    wait_for_failure(&mut app, "is not one of the project's docs");
    let docs = docs.expect("docs listed");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "Reading mode");
    assert!(doc.is_some_and(|d| d.contains("Text.")));
    app.quit();
}

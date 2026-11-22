//! Tangents (ADR 0017, amended 2026-11-11; M22) through the real core and `FakeDriver`: the
//! built-in project, a tangent's read-only session in Tangents' folder, its answer and its
//! subagent's sources captured into Tangents, the destinations (attach, file, start, drop), the
//! morning's "new tangents" item, and a restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::{
    Command, CoreLink, Event, ServerMsg, Snapshot,
    agent::{AgentCommand, AgentId, AgentKind},
    app::{AppCommand, AppEvent},
    apply,
    conversation::{ConversationEntry, EntryId, EntryKind},
    inbox::InboxKind,
    project::{ProjectId, ProjectKind},
    tangent::{TANGENTS_PROJECT, TangentCommand},
    task::{NewTask, TaskCommand, TaskId, TaskStatus},
};
use dovetail_core::config::Config;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::projects::{self, Candidate, Project, Resolution};
use dovetail_core::{CoreHandle, CoreOptions, Drivers};
use dovetail_driver::FakeDriver;
use tempfile::TempDir;

const COORDINATOR: &str = "coordinator";
const QUESTION: &str = "What does IANA say about example.com?";

fn fixture(name: &str) -> FakeDriver {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dovetail-driver/tests/fixtures")
        .join(format!("{name}.ndjson"));
    FakeDriver::from_file(path).expect("fixture")
}

fn paths(dir: &Path, run: &Path) -> Paths {
    Paths {
        data: dir.join("data"),
        config: dir.join("config"),
        state: dir.join("state"),
        runtime: run.to_path_buf(),
    }
}

/// Registers a research project and returns it as `dovetail --project` opens it.
fn register(paths: &Paths, name: &str) -> Project {
    let db = Db::open(paths, None).expect("registry");
    let made = db
        .register_research(projects::slug(name), name.into())
        .wait()
        .expect("register")
        .expect("a new name");
    projects::open(&db, &made.id).expect("open by id")
}

/// One app on a project.
struct App {
    link: CoreLink,
    core: Option<CoreHandle>,
    snap: Snapshot,
    failures: Vec<String>,
}

impl App {
    fn start(driver: &FakeDriver, paths: &Paths, project: Project) -> App {
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
            opener: None,
            gh: None,
        };
        let (link, core) = dovetail_core::spawn(
            Resolution::Registered(project),
            paths.clone(),
            Config::default(),
            options,
        )
        .expect("core");
        let mut app = App {
            link,
            core: Some(core),
            snap: Snapshot::default(),
            failures: Vec::new(),
        };
        app.wait_for("the snapshot", |s| !s.agents.is_empty());
        app
    }

    fn send(&self, command: Command) {
        self.link.commands.send(command).expect("core is running");
    }

    fn fold(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Hello { .. } => {}
            ServerMsg::Snapshot(s) => self.snap = s,
            ServerMsg::Event { seq, event } => {
                if let Event::App(AppEvent::CommandFailed { message }) = &event {
                    self.failures.push(message.clone());
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
                Ok(msg) => self.fold(msg),
                Err(_) => panic!("timed out waiting for {what}: {:#?}", self.failures),
            }
        }
    }

    fn drain(&mut self, window: Duration) {
        let until = Instant::now() + window;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match self.link.messages.recv_timeout(left) {
                Ok(msg) => self.fold(msg),
                Err(_) => return,
            }
        }
    }

    fn quit(mut self) {
        self.send(Command::App(AppCommand::Quit { end_sessions: true }));
        let core = self.core.take().expect("core handle");
        assert!(core.wait(Duration::from_secs(10)), "core stopped");
    }

    /// Asks a tangent and waits until its answer and its subagent's finding are filed.
    fn ask(&mut self, question: &str) -> TaskId {
        let before = self.snap.tangents.len();
        self.send(Command::Tangent(TangentCommand::New {
            question: question.into(),
        }));
        self.wait_for("the tangent's answer and its subagent's finding", |s| {
            s.tangents.len() > before
                && s.tangents
                    .last()
                    .and_then(|t| t.research.as_ref())
                    .is_some_and(|r| r.findings >= 2)
        });
        self.snap.tangents.last().expect("the tangent").task.id
    }
}

/// Short runtime dirs: unix socket paths are limited to about 108 bytes.
fn dirs() -> (TempDir, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = tempfile::Builder::new()
        .prefix("dvt.")
        .tempdir_in("/tmp")
        .expect("short tempdir");
    (dir, run)
}

#[test]
fn tangents_is_a_built_in_research_project_whose_id_nobody_else_gets() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let db = Db::open(&paths, None).expect("registry");
    let tangents = db.ensure_tangents().wait().expect("created");
    assert_eq!(tangents.id, TANGENTS_PROJECT);
    assert_eq!(tangents.name, "Tangents");
    assert_eq!(tangents.kind(), ProjectKind::Research);
    assert!(tangents.repos.is_empty());
    // Created once: the second call finds it.
    assert_eq!(db.ensure_tangents().wait().expect("again"), tangents);
    assert_eq!(
        projects::list(&db)
            .expect("list")
            .iter()
            .filter(|p| p.id == TANGENTS_PROJECT)
            .count(),
        1
    );
    // A folder of the user's named "tangents" is numbered, and the name is taken.
    let folder = dir.path().join("tangents");
    std::fs::create_dir_all(&folder).unwrap();
    let mine = projects::register(
        &db,
        &Candidate {
            name: "tangents".into(),
            repo_path: folder,
            git_common_dir: None,
        },
    )
    .expect("registered");
    assert_eq!(mine.id, "tangents-2");
    assert!(
        db.register_research("tangents".into(), "Tangents".into())
            .wait()
            .expect("refused, not failed")
            .is_none()
    );
    drop(db);

    // Any app registers it at start, and lists it.
    let other = register(&paths, "Market sizing");
    let fake = fixture("plain_turn");
    let mut app = App::start(&fake, &paths, other);
    assert!(
        app.snap
            .projects
            .iter()
            .any(|p| p.id == ProjectId(TANGENTS_PROJECT.into()) && p.kind == ProjectKind::Research)
    );
    assert!(paths.project_dir(TANGENTS_PROJECT).is_dir());
    assert!(app.snap.warnings.iter().all(|w| w.id.0 != "tangents"));
    app.drain(Duration::from_millis(50));
    app.quit();
}

#[test]
fn a_tangent_runs_read_only_in_tangents_and_is_saved_there_with_its_sources() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let role = paths.role_prompt("tangent");
    std::fs::create_dir_all(role.parent().unwrap()).unwrap();
    std::fs::write(&role, "You are a tangent.\n").unwrap();
    let project = register(&paths, "Market sizing");
    let fake = fixture("research_websearch");
    let mut app = App::start(&fake, &paths, project.clone());

    let tangent = app.ask(QUESTION);
    let saved = app.snap.tangents.last().expect("saved").clone();
    assert_eq!(saved.task.title, QUESTION);
    assert!(saved.is_new(), "{saved:?}");
    assert!(
        saved.task.notes.contains("Asked from Market sizing"),
        "{}",
        saved.task.notes
    );
    let research = saved.research.expect("research");
    assert!(research.sources > 0, "{research:?}");
    // Not a task of the open project.
    assert!(app.snap.tasks.is_empty(), "{:?}", app.snap.tasks);

    // Its session: named after its task, in the panel (not the tree), read-only like a side
    // chat, in Tangents' folder, with its own role prompt and the Core suite only.
    let agent = AgentId(format!("tangent-{}", tangent.0));
    let row = app
        .snap
        .agents
        .iter()
        .find(|a| a.id == agent)
        .expect("tangent agent");
    assert_eq!(row.kind, AgentKind::Tangent);
    assert_eq!(row.task, None);
    let spec = fake
        .started_specs()
        .into_iter()
        .find(|s| {
            s.name
                .as_deref()
                .is_some_and(|n| n.ends_with(&format!(".{}", agent.0)))
        })
        .expect("tangent spec");
    assert_eq!(spec.cwd, paths.project_dir(TANGENTS_PROJECT));
    assert_eq!(
        spec.append_system_prompt_file.as_deref(),
        Some(role.as_path())
    );
    let args = spec.claude_args();
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "dontAsk"),
        "{args:?}"
    );
    let denied = args
        .windows(2)
        .find(|w| w[0] == "--disallowedTools")
        .map(|w| w[1].clone())
        .expect("--disallowedTools");
    for rule in ["Edit", "Write", "NotebookEdit"] {
        assert!(denied.contains(rule), "{rule} in {denied}");
    }
    let mcp =
        std::fs::read_to_string(spec.claude.mcp_config_path.as_ref().expect("mcp")).expect("read");
    assert!(mcp.contains(r#""DOVETAIL_SUITES": "core""#), "{mcp}");

    // Its rows are in Tangents' own state.db, its files in Tangents' research folder.
    let db = Db::open(&paths, Some(TANGENTS_PROJECT)).expect("tangents db");
    let rows = db.task_research(tangent).wait().expect("research");
    let answer = rows
        .findings
        .iter()
        .find(|f| f.label == "answer")
        .expect("the answer");
    assert!(answer.body.starts_with("done"), "{}", answer.body);
    assert_eq!(answer.body_from, "turn_result");
    assert!(
        rows.findings
            .iter()
            .any(|f| f.label != "answer" && !f.sources.is_empty()),
        "the subagent's finding with its sources: {:?}",
        rows.findings
    );
    let folder = paths
        .project_dir(TANGENTS_PROJECT)
        .join("research")
        .join(format!(
            "{}-what-does-iana-say-about-example-com",
            tangent.0
        ));
    assert!(folder.join(&answer.path).is_file(), "{}", folder.display());
    // The open project got nothing.
    let own = Db::open(&paths, Some(&project.id)).expect("own db");
    assert!(own.research_summaries().wait().expect("rows").is_empty());
    drop((db, own));

    // Read-only for real: its mode never cycles.
    app.send(Command::Agent(AgentCommand::CyclePermissionMode {
        agent: agent.clone(),
    }));
    app.drain(Duration::from_millis(200));
    assert!(
        app.failures.iter().any(|f| f.contains("read-only")),
        "{:?}",
        app.failures
    );
    app.quit();

    // A restart keeps it: the tangent, its research and its session.
    let fake = fixture("plain_turn");
    let db = Db::open(&paths, None).expect("registry");
    let again = projects::open(&db, &project.id).expect("open");
    drop(db);
    let mut app = App::start(&fake, &paths, again);
    let back = app
        .snap
        .tangents
        .iter()
        .find(|t| t.task.id == tangent)
        .expect("still in Tangents");
    assert!(back.research.as_ref().is_some_and(|r| r.findings >= 2));
    assert!(
        app.snap
            .agents
            .iter()
            .any(|a| a.id == agent && a.kind == AgentKind::Tangent)
    );
    app.drain(Duration::from_millis(50));
    app.quit();
}

#[test]
fn a_tangent_attaches_to_a_task_files_into_a_project_starts_a_task_or_drops() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let other = register(&paths, "Pricing");
    let project = register(&paths, "Market sizing");
    let fake = fixture("research_websearch");
    let mut app = App::start(&fake, &paths, project.clone());
    app.send(Command::Task(TaskCommand::Create(NewTask {
        title: "Size the market".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    })));
    app.wait_for("the task", |s| !s.tasks.is_empty());
    let task = app.snap.tasks[0].id;

    // Attach: its answers and sources become quick findings of the task; it leaves Tangents.
    let first = app.ask(QUESTION);
    let findings = app
        .snap
        .tangents
        .last()
        .unwrap()
        .research
        .as_ref()
        .unwrap()
        .findings;
    app.send(Command::Tangent(TangentCommand::Attach {
        tangent: first,
        task,
    }));
    app.wait_for("the findings on the task", |s| {
        s.research
            .iter()
            .any(|r| r.task == task && r.quick_findings == findings && r.sources > 0)
    });
    app.wait_for("the tangent gone", |s| {
        s.tangents.iter().all(|t| t.task.id != first)
    });
    let agent = AgentId(format!("tangent-{}", first.0));
    app.wait_for("its session closed", |s| {
        s.agents.iter().all(|a| a.id != agent)
    });
    let own = Db::open(&paths, Some(&project.id)).expect("own db");
    let rows = own.task_research(task).wait().expect("rows");
    assert!(rows.findings.iter().any(|f| f.label == "answer"));
    assert!(!rows.sources.is_empty());
    drop(own);

    // File into another project: a task there with the question, answers and sources.
    let second = app.ask("And who runs example.org?");
    app.send(Command::Tangent(TangentCommand::File {
        tangent: second,
        project: ProjectId(other.id.clone()),
    }));
    app.wait_for("the tangent filed", |s| {
        s.tangents.iter().all(|t| t.task.id != second)
    });
    let theirs = Db::open(&paths, Some(&other.id)).expect("their db");
    let tasks = theirs.load_tasks().wait().expect("tasks");
    let filed = tasks
        .iter()
        .find(|t| t.title == "And who runs example.org?")
        .expect("filed as a task");
    assert_eq!(filed.bucket, None, "in their Unsorted");
    assert!(
        filed.notes.contains("Filed from Tangents"),
        "{}",
        filed.notes
    );
    let rows = theirs.task_research(filed.id).wait().expect("rows");
    assert!(rows.findings.len() >= 2 && !rows.sources.is_empty());
    drop(theirs);

    // Start a task: a new task here linked to it; the tangent stays, done.
    let third = app.ask("Is example.net reserved too?");
    app.send(Command::Tangent(TangentCommand::StartFrom {
        tangent: third,
        project: ProjectId(project.id.clone()),
    }));
    app.wait_for("the started task", |s| {
        s.tasks.iter().any(|t| {
            t.title == "Is example.net reserved too?"
                && t.notes.contains("Started from the tangent")
        })
    });
    app.wait_for("the tangent done", |s| {
        s.tangents
            .iter()
            .any(|t| t.task.id == third && t.task.status == TaskStatus::Done && !t.is_new())
    });

    // Drop: archived in Tangents, nothing deleted.
    let fourth = app.ask("What is RFC 2606?");
    app.send(Command::Tangent(TangentCommand::Drop { tangent: fourth }));
    app.wait_for("the tangent dropped", |s| {
        s.tangents.iter().all(|t| t.task.id != fourth)
    });
    let db = Db::open(&paths, Some(TANGENTS_PROJECT)).expect("tangents db");
    let tasks = db.load_tasks().wait().expect("tasks");
    let archived = |id: TaskId, note: &str| {
        tasks
            .iter()
            .any(|t| t.id == id && t.status == TaskStatus::Archived && t.notes.contains(note))
    };
    assert!(archived(first, "Attached to #"), "{tasks:?}");
    assert!(archived(second, "Filed into Pricing"), "{tasks:?}");
    assert!(archived(fourth, "Dropped on"), "{tasks:?}");
    assert!(!db.task_research(fourth).wait().unwrap().findings.is_empty());
    drop(db);

    // Tangents itself is no destination.
    app.send(Command::Tangent(TangentCommand::File {
        tangent: third,
        project: ProjectId(TANGENTS_PROJECT.into()),
    }));
    app.drain(Duration::from_millis(200));
    assert!(
        app.failures
            .iter()
            .any(|f| f.contains("in Tangents already")),
        "{:?}",
        app.failures
    );
    app.quit();
}

#[test]
fn the_first_open_of_a_day_lists_new_tangents_in_needs_you_until_triaged() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let project = register(&paths, "Market sizing");
    // Yesterday's coordinator: one message two days back.
    let fake = fixture("plain_turn");
    let app = App::start(&fake, &paths, project.clone());
    app.quit();
    let own = Db::open(&paths, Some(&project.id)).expect("own db");
    let two_days_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 2 * 86_400_000;
    own.upsert_entry(
        AgentId(COORDINATOR.into()),
        ConversationEntry {
            id: EntryId(1),
            at_ms: two_days_ago,
            kind: EntryKind::User,
            text: "yesterday".into(),
            attachments: vec![],
            effort: None,
        },
    )
    .wait()
    .expect("entry");
    drop(own);
    // Two tangents nobody triaged, and one already kept in a bucket.
    let tangents = Db::open(&paths, Some(TANGENTS_PROJECT)).expect("tangents db");
    for (title, bucket) in [
        ("one", None),
        ("two", None),
        ("kept", Some(dovetail_api::task::Bucket::Later)),
    ] {
        tangents
            .create_task(
                NewTask {
                    title: title.into(),
                    p_level: None,
                    bucket,
                    notes: String::new(),
                    follow_up_of: None,
                    pr: None,
                    related: vec![],
                    effort: Default::default(),
                    criterion: None,
                },
                false,
            )
            .wait()
            .expect("tangent");
    }
    drop(tangents);

    let db = Db::open(&paths, None).expect("registry");
    let again = projects::open(&db, &project.id).expect("open");
    drop(db);
    let mut app = App::start(&fake, &paths, again);
    app.wait_for("the morning item", |s| {
        s.inbox
            .iter()
            .any(|i| i.kind == InboxKind::NewTangents { count: 2 })
    });
    assert_eq!(app.snap.tangents.len(), 3);
    let one = app
        .snap
        .tangents
        .iter()
        .find(|t| t.task.title == "one")
        .unwrap()
        .task
        .id;
    let two = app
        .snap
        .tangents
        .iter()
        .find(|t| t.task.title == "two")
        .unwrap()
        .task
        .id;
    app.send(Command::Tangent(TangentCommand::Drop { tangent: one }));
    app.wait_for("the count down", |s| {
        s.inbox
            .iter()
            .any(|i| i.kind == InboxKind::NewTangents { count: 1 })
    });
    app.quit();

    // A restart keeps the open item (not raised twice).
    let db = Db::open(&paths, None).expect("registry");
    let again = projects::open(&db, &project.id).expect("open");
    drop(db);
    let mut app = App::start(&fake, &paths, again);
    app.drain(Duration::from_millis(100));
    let items: Vec<_> = app
        .snap
        .inbox
        .iter()
        .filter(|i| matches!(i.kind, InboxKind::NewTangents { .. }))
        .collect();
    assert_eq!(items.len(), 1, "{:?}", app.snap.inbox);
    assert_eq!(items[0].kind, InboxKind::NewTangents { count: 1 });
    app.send(Command::Tangent(TangentCommand::StartFrom {
        tangent: two,
        project: ProjectId(project.id.clone()),
    }));
    app.wait_for("the item gone once all are triaged", |s| {
        s.inbox
            .iter()
            .all(|i| !matches!(i.kind, InboxKind::NewTangents { .. }))
    });
    app.quit();

    // The same day again, with a new tangent waiting: no second item (once a day).
    Db::open(&paths, Some(TANGENTS_PROJECT))
        .expect("tangents db")
        .create_task(
            NewTask {
                title: "three".into(),
                p_level: None,
                bucket: None,
                notes: String::new(),
                follow_up_of: None,
                pr: None,
                related: vec![],
                effort: Default::default(),
                criterion: None,
            },
            false,
        )
        .wait()
        .expect("tangent");
    let db = Db::open(&paths, None).expect("registry");
    let again = projects::open(&db, &project.id).expect("open");
    drop(db);
    let mut app = App::start(&fake, &paths, again);
    app.drain(Duration::from_millis(200));
    assert!(
        app.snap
            .inbox
            .iter()
            .all(|i| !matches!(i.kind, InboxKind::NewTangents { .. })),
        "{:?}",
        app.snap.inbox
    );
    app.quit();
}

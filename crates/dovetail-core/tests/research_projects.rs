//! Research-only projects (decided 2026-11-11): the registry kind, the coordinator running in
//! the project's state folder, a restart restoring it, and the kind checks (no slices, no
//! worktrees, no git), through the real core and `FakeDriver`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::{StartSlice, Tool};
use dovetail_api::{
    AgentId, Command, CoreLink, Event, NewTask, ProjectId, ProjectKind, ServerMsg, Snapshot,
    TaskId, apply,
};
use dovetail_core::config::Config;
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::projects::{self, Project, Resolution};
use dovetail_core::{CoreHandle, CoreOptions};
use dovetail_driver::FakeDriver;
use tempfile::TempDir;

const COORDINATOR: &str = "coordinator";

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

/// Registers the research project "Market sizing" and returns it as `dovetail --project` opens
/// it.
fn register(paths: &Paths) -> Project {
    let db = Db::open(paths, None).expect("registry");
    let made = db
        .register_research("market-sizing".into(), "Market sizing".into())
        .wait()
        .expect("register")
        .expect("a new name");
    assert_eq!(made.kind, ProjectKind::Research);
    assert_eq!(made.repo_path, None);
    projects::open(&db, &made.id).expect("open by id")
}

/// One app on a research project.
struct App {
    link: CoreLink,
    core: Option<CoreHandle>,
    snap: Snapshot,
    failures: Vec<String>,
    events: Vec<Event>,
}

impl App {
    fn start(driver: &FakeDriver, paths: &Paths, project: Project) -> App {
        let options = CoreOptions {
            driver: Arc::new(driver.clone()),
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
            events: Vec::new(),
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
                if let Event::CommandFailed { message } = &event {
                    self.failures.push(message.clone());
                }
                self.events.push(event.clone());
                apply(&mut self.snap, seq, event);
            }
        }
    }

    fn wait_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(self) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("timed out waiting for {what}: {:#?}", self.failures));
            match self.link.messages.recv_timeout(left) {
                Ok(msg) => self.fold(msg),
                Err(_) => panic!("timed out waiting for {what}: {:#?}", self.failures),
            }
        }
    }

    fn wait_for(&mut self, what: &str, done: impl Fn(&Snapshot) -> bool) {
        self.wait_until(what, |app| done(&app.snap));
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

    fn socket(&self, paths: &Paths, id: &str) -> PathBuf {
        paths.socket(id)
    }

    fn quit(mut self) {
        self.send(Command::Quit { end_sessions: true });
        let core = self.core.take().expect("core handle");
        assert!(core.wait(Duration::from_secs(10)), "core stopped");
    }
}

fn said(snap: &Snapshot, agent: &str, text: &str) -> bool {
    snap.conversations
        .get(&AgentId(agent.into()))
        .is_some_and(|t| t.entries.iter().any(|e| e.text == text))
}

/// One NDJSON round trip on the app socket, as the bridge does it.
fn socket_call(path: &Path, body: SocketRequestBody) -> (String, bool) {
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
    match response.body {
        SocketResponseBody::ToolResult { text, is_error } => (text, is_error),
        other => panic!("not a tool result: {other:?}"),
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
fn the_coordinator_runs_in_the_state_folder_with_its_role_prompt_and_resumes_after_a_restart() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let role = paths.role_prompt("coordinator");
    std::fs::create_dir_all(role.parent().unwrap()).unwrap();
    std::fs::write(&role, "You are the coordinator.\n").unwrap();
    let project = register(&paths);
    let folder = paths.project_dir(&project.id);

    let fake = fixture("plain_turn");
    let mut app = App::start(&fake, &paths, project.clone());
    let open = app.snap.project.clone().expect("the open project");
    assert_eq!(open.kind, ProjectKind::Research);
    assert_eq!(open.repo_path, folder);
    assert!(
        app.snap
            .projects
            .iter()
            .any(|p| p.id == ProjectId("market-sizing".into()) && p.kind == ProjectKind::Research)
    );
    // Registered: it has a journal, so `:eod` and the morning coordinator work as anywhere.
    assert!(app.snap.journal.is_some());
    // No recipe is expected of a research project.
    assert!(
        app.snap.warnings.iter().all(|w| !w.text.contains("recipe")),
        "{:?}",
        app.snap.warnings
    );
    app.send(Command::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "hello".into(),
        attachments: Vec::new(),
    });
    app.wait_for("the reply", |s| said(s, COORDINATOR, "alpha"));
    let spec = fake.started_specs()[0].clone();
    assert_eq!(spec.cwd, folder);
    assert!(folder.is_dir());
    assert_eq!(
        spec.append_system_prompt_file.as_deref(),
        Some(role.as_path())
    );
    assert!(spec.worktree.is_none());
    app.quit();

    // Reopened the way `dovetail --project market-sizing` does it.
    let db = Db::open(&paths, None).expect("registry");
    let again = projects::open(&db, "market-sizing").expect("still registered");
    drop(db);
    assert_eq!(again.kind, ProjectKind::Research);
    let fake = fixture("plain_turn");
    let mut app = App::start(&fake, &paths, again);
    assert!(said(&app.snap, COORDINATOR, "alpha"), "the chat is kept");
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "the coordinator did not resume");
        app.drain(Duration::from_millis(20));
    }
    let spec = fake.started_specs()[0].clone();
    assert!(spec.resume);
    assert_eq!(spec.cwd, folder);
    app.quit();
}

#[test]
fn a_research_project_runs_no_slices_and_has_no_worktrees_or_git() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let project = register(&paths);
    let folder = paths.project_dir(&project.id);
    // Were git ever asked, it would list this file: the state folder is a repo here on purpose.
    std::fs::create_dir_all(&folder).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&folder)
        .status();
    let have_git = init.is_ok_and(|s| s.success());
    std::fs::write(folder.join("notes.md"), "x\n").unwrap();

    let fake = fixture("plain_turn");
    let mut app = App::start(&fake, &paths, project.clone());
    app.send(Command::CreateTask(NewTask {
        title: "Size the market".into(),
        p_level: None,
        bucket: None,
        notes: String::new(),
        follow_up_of: None,
        pr: None,
        related: vec![],
    }));
    app.wait_for("the task", |s| !s.tasks.is_empty());

    // `:spawn` from the UI.
    app.send(Command::NewSlice {
        task: None,
        name: "later".into(),
        brief: "go".into(),
    });
    app.wait_until("the refusal", |a| !a.failures.is_empty());
    assert!(
        app.failures[0].contains("research project"),
        "{:?}",
        app.failures
    );
    // `project_start_slice` from the coordinator.
    let (text, is_error) = socket_call(
        &app.socket(&paths, &project.id),
        SocketRequestBody::ToolCall {
            agent: AgentId(COORDINATOR.into()),
            tool_use_id: None,
            called_as: None,
            call: Tool::StartSlice(StartSlice {
                task: TaskId(1),
                name: "later".into(),
                brief: "go".into(),
            }),
        },
    );
    assert!(is_error && text.contains("research project"), "{text}");
    assert!(!app.snap.agents.iter().any(|a| a.name == "later"));

    // No worktree refresh, no `@` listing from git, no `:upgrade`.
    app.send(Command::RefreshWorktrees);
    app.send(Command::ListPaths {
        agent: AgentId(COORDINATOR.into()),
        dir: None,
    });
    app.wait_until("the path listing", |a| {
        a.events
            .iter()
            .any(|e| matches!(e, Event::PathsListed { .. }))
    });
    let listed = app.events.iter().find_map(|e| match e {
        Event::PathsListed { paths, .. } => Some(paths.clone()),
        _ => None,
    });
    if have_git {
        assert_eq!(listed, Some(Vec::new()), "git listed the state folder");
    }
    app.send(Command::Upgrade { local: false });
    app.wait_until("the upgrade refusal", |a| a.failures.len() >= 2);
    assert!(
        app.failures[1].contains("research project"),
        "{:?}",
        app.failures
    );
    app.drain(Duration::from_millis(200));
    assert!(
        !app.events
            .iter()
            .any(|e| matches!(e, Event::WorktreeUpserted(_))),
        "a worktree was gathered"
    );
    assert!(app.snap.worktrees.is_empty());
    app.quit();
}

#[test]
fn new_research_project_registers_it_and_opening_it_quits_for_the_switch() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    let project = register(&paths);
    let fake = fixture("plain_turn");
    let mut app = App::start(&fake, &paths, project);

    app.send(Command::NewResearchProject {
        name: "Pricing study".into(),
    });
    app.wait_for("the new project in the list", |s| {
        s.projects.iter().any(|p| p.name == "Pricing study")
    });
    let made = app
        .snap
        .projects
        .iter()
        .find(|p| p.name == "Pricing study")
        .cloned()
        .unwrap();
    assert_eq!(made.id, ProjectId("pricing-study".into()));
    assert_eq!(made.kind, ProjectKind::Research);
    assert_eq!(made.repo_path, paths.project_dir("pricing-study"));
    assert!(made.repo_path.is_dir(), "its state folder is made");
    // The open project stays open.
    assert_eq!(
        app.snap.project.as_ref().map(|p| p.id.0.as_str()),
        Some("market-sizing")
    );

    // The same name again is refused.
    app.send(Command::NewResearchProject {
        name: "Pricing study".into(),
    });
    app.wait_until("the refusal", |a| !a.failures.is_empty());
    assert!(app.failures[0].contains("already"), "{:?}", app.failures);

    // Opening it: the core says so and quits, leaving the id for the binary.
    app.send(Command::OpenProject {
        id: ProjectId("pricing-study".into()),
    });
    app.wait_until("the switch", |a| {
        a.events
            .iter()
            .any(|e| matches!(e, Event::OpeningProject { .. }))
    });
    let core = app.core.take().expect("core handle");
    assert_eq!(core.switch_requested().as_deref(), Some("pricing-study"));
    assert!(
        core.wait(Duration::from_secs(10)),
        "the core quit on its own"
    );
}

#[test]
fn a_task_is_captured_into_a_research_project_by_id_without_opening_it() {
    let (dir, run) = dirs();
    let paths = paths(dir.path(), run.path());
    register(&paths);
    let registry = Db::open(&paths, None).expect("registry");
    let before = projects::find(&registry, "market-sizing").expect("registered");
    assert_eq!(before.kind, ProjectKind::Research);
    assert!(matches!(
        projects::find(&registry, "nope"),
        Err(projects::ProjectError::UnknownId(id)) if id == "nope"
    ));
    // `dovetail add --project market-sizing` with no app running writes to the project's db.
    let db = Db::open(&paths, Some(&before.id)).expect("project db");
    let created = db
        .create_task(
            dovetail_api::NewTask {
                title: "size the market".into(),
                p_level: None,
                bucket: None,
                notes: String::new(),
                follow_up_of: None,
                pr: None,
                related: Vec::new(),
            },
            false,
        )
        .wait()
        .expect("created");
    assert_eq!(created[0].title, "size the market");
    // Finding it did not count as opening it.
    let after = projects::find(&registry, "market-sizing").expect("registered");
    assert_eq!(after, before);
}

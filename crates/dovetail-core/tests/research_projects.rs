//! Research-only projects (decided 2026-11-11): the registry kind, the coordinator running in
//! the project's state folder, a restart restoring it, and the kind checks (no slices, no
//! worktrees, no git), through the real core and `FakeDriver`.

mod common;

use std::time::{Duration, Instant};

use common::{COORDINATOR, Harness, fixture, test_paths};
use dovetail_api::{
    Command, Event, Snapshot,
    agent::{AgentCommand, AgentId},
    app::AppCommand,
    conversation::{ConversationCommand, ConversationEvent},
    library::{LibraryCommand, LibraryEvent},
    project::{ProjectCommand, ProjectEvent, ProjectId, ProjectKind},
    task::{NewTask, TaskCommand, TaskId},
    worktree::{WorktreeCommand, WorktreeEvent},
};
use dovetail_core::db::Db;
use dovetail_core::paths::Paths;
use dovetail_core::projects::{self, Project};
use dovetail_tools::{
    Tool,
    project::{ProjectTool, StartSlice},
};

/// Registers the research project "Market sizing" and returns it as `dovetail --project` opens
/// it.
fn register(paths: &Paths) -> Project {
    let db = Db::open(paths, None).expect("registry");
    let made = db
        .register_research("market-sizing".into(), "Market sizing".into())
        .wait()
        .expect("register")
        .expect("a new name");
    assert_eq!(made.kind(), ProjectKind::Research);
    assert!(made.repos.is_empty());
    projects::open(&db, &made.id).expect("open by id")
}

fn said(snap: &Snapshot, agent: &str, text: &str) -> bool {
    snap.conversations
        .get(&AgentId(agent.into()))
        .is_some_and(|t| t.entries.iter().any(|e| e.text == text))
}

#[test]
fn the_coordinator_runs_in_its_home_folder_with_its_role_prompt_and_resumes_after_a_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = test_paths(dir.path());
    let role = paths.role_prompt("coordinator");
    std::fs::create_dir_all(role.parent().unwrap()).unwrap();
    std::fs::write(&role, "You are the coordinator.\n").unwrap();
    let project = register(&paths);
    let folder = paths.project_dir(&project.id);

    let fake = fixture("plain_turn");
    let mut app = Harness::start_project(fake.clone(), paths.clone(), project.clone());
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
    app.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "hello".into(),
        attachments: Vec::new(),
        effort: None,
    }));
    app.wait_for("the reply", |s| said(s, COORDINATOR, "alpha"));
    // ADR 0021 point 3: every coordinator runs in its project's `home/`, with no repos to attach.
    let spec = fake.started_specs()[0].clone();
    let home = paths.project_home(&project.id);
    assert_eq!(spec.cwd, home);
    assert!(home.is_dir());
    assert!(spec.add_dirs.is_empty());
    assert_eq!(
        spec.append_system_prompt_file.as_deref(),
        Some(role.as_path())
    );
    assert!(spec.claude.worktree.is_none());
    app.quit();

    // Reopened the way `dovetail --project market-sizing` does it.
    let db = Db::open(&paths, None).expect("registry");
    let again = projects::open(&db, "market-sizing").expect("still registered");
    drop(db);
    assert_eq!(again.kind(), ProjectKind::Research);
    let fake = fixture("plain_turn");
    let mut app = Harness::start_project(fake.clone(), paths.clone(), again);
    assert!(said(&app.snap, COORDINATOR, "alpha"), "the chat is kept");
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.started_specs().is_empty() {
        assert!(Instant::now() < deadline, "the coordinator did not resume");
        app.drain(Duration::from_millis(20));
    }
    let spec = fake.started_specs()[0].clone();
    assert!(spec.resume);
    assert_eq!(spec.cwd, home);
    app.quit();
}

#[test]
fn a_research_project_runs_no_slices_and_has_no_worktrees_or_git() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = test_paths(dir.path());
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
    let mut app = Harness::start_project(fake.clone(), paths.clone(), project.clone());
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

    // `:spawn` from the UI.
    app.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "later".into(),
        brief: "go".into(),
    }));
    app.wait_until("the refusal", |a| !a.failures.is_empty());
    assert!(
        app.failures[0].contains("research project"),
        "{:?}",
        app.failures
    );
    // `project_start_slice` from the coordinator.
    let (text, is_error) = app.call(Tool::Project(ProjectTool::StartSlice(StartSlice {
        repo: None,
        model: None,
        effort: None,
        driver: None,
        task: TaskId(1),
        name: "later".into(),
        brief: "go".into(),
    })));
    assert!(is_error && text.contains("research project"), "{text}");
    assert!(!app.snap.agents.iter().any(|a| a.name == "later"));

    // No worktree refresh, no `@` listing from git, no `:upgrade`.
    app.send(Command::Worktree(WorktreeCommand::Refresh));
    app.send(Command::Conversation(ConversationCommand::ListPaths {
        agent: AgentId(COORDINATOR.into()),
        dir: None,
    }));
    app.wait_until("the path listing", |a| {
        a.events.iter().any(|e| {
            matches!(
                e,
                Event::Conversation(ConversationEvent::PathsListed { .. })
            )
        })
    });
    let listed = app.events.iter().find_map(|e| match e {
        Event::Conversation(ConversationEvent::PathsListed { paths, .. }) => Some(paths.clone()),
        _ => None,
    });
    if have_git {
        assert_eq!(listed, Some(Vec::new()), "git listed the state folder");
    }
    app.send(Command::App(AppCommand::StartUpgrade { local: false }));
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
            .any(|e| matches!(e, Event::Worktree(WorktreeEvent::Upserted(_)))),
        "a worktree was gathered"
    );
    assert!(app.snap.worktrees.is_empty());
    app.quit();
}

#[test]
fn new_research_project_registers_it_and_opening_it_quits_for_the_switch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = test_paths(dir.path());
    let project = register(&paths);
    let fake = fixture("plain_turn");
    let mut app = Harness::start_project(fake.clone(), paths.clone(), project);

    app.send(Command::Project(ProjectCommand::NewResearch {
        name: "Pricing study".into(),
    }));
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
    app.send(Command::Project(ProjectCommand::NewResearch {
        name: "Pricing study".into(),
    }));
    app.wait_until("the refusal", |a| !a.failures.is_empty());
    assert!(app.failures[0].contains("already"), "{:?}", app.failures);

    // Opening it: the core says so and quits, leaving the id for the binary.
    app.send(Command::Project(ProjectCommand::Open {
        id: ProjectId("pricing-study".into()),
    }));
    app.wait_until("the switch", |a| {
        a.events
            .iter()
            .any(|e| matches!(e, Event::Project(ProjectEvent::Opening { .. })))
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
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = test_paths(dir.path());
    register(&paths);
    let registry = Db::open(&paths, None).expect("registry");
    let before = projects::find(&registry, "market-sizing").expect("registered");
    assert_eq!(before.kind(), ProjectKind::Research);
    assert!(matches!(
        projects::find(&registry, "nope"),
        Err(projects::ProjectError::UnknownId(id)) if id == "nope"
    ));
    // `dovetail add --project market-sizing` with no app running writes to the project's db.
    let db = Db::open(&paths, Some(&before.id)).expect("project db");
    let created = db
        .create_task(
            dovetail_api::task::NewTask {
                title: "size the market".into(),
                p_level: None,
                bucket: None,
                notes: String::new(),
                follow_up_of: None,
                pr: None,
                related: Vec::new(),
                effort: Default::default(),
                criterion: None,
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

/// `<leader>fr` (ADR 0018, decided 2026-11-11 later): another project's library is read from
/// its `state.db` opened read-only, while a different project is open, and nothing of it is
/// written.
#[test]
fn another_projects_library_is_loaded_read_only() {
    use dovetail_api::{library::BrowseWhat, research::ResearchWhat};
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = test_paths(dir.path());
    // The browsed project, with a task, a round, a report and a decision, written as its own
    // Dovetail would.
    let browsed = register(&paths);
    {
        let db = Db::open(&paths, Some(&browsed.id)).expect("project db");
        let task = db
            .create_task(
                NewTask {
                    title: "size the market".into(),
                    p_level: None,
                    bucket: None,
                    notes: String::new(),
                    follow_up_of: None,
                    pr: None,
                    related: Vec::new(),
                    effort: Default::default(),
                    criterion: None,
                },
                false,
            )
            .wait()
            .expect("created")[0]
            .id;
        db.start_round(task, "Count the teams.".into())
            .wait()
            .expect("round");
        db.insert_report(
            task,
            None,
            "# EU market\n\nAbout 40k teams.\n".into(),
            (0, 0),
        )
        .wait()
        .expect("report");
        db.log_decision(
            dovetail_api::decision::NewDecision {
                task,
                text: "Size by teams".into(),
                why: None,
                alternatives: Vec::new(),
                supersedes: None,
            },
            dovetail_api::decision::DecisionAuthor::User,
        )
        .wait()
        .expect("decision");
    }
    let state_db = paths.state_db(&browsed.id);
    // The DB thread closes its connection (checkpointing the WAL into the file) after the
    // handle is dropped: wait for that, so the file is final.
    let wal = state_db.with_extension("db-wal");
    let deadline = Instant::now() + Duration::from_secs(10);
    while wal.exists() {
        assert!(Instant::now() < deadline, "the writer never closed");
        std::thread::sleep(Duration::from_millis(20));
    }
    let before = std::fs::read(&state_db).expect("its state.db");

    // The open project: another research project.
    let registry = Db::open(&paths, None).expect("registry");
    let made = registry
        .register_research("pricing-study".into(), "Pricing study".into())
        .wait()
        .expect("register")
        .expect("new");
    let open = projects::open(&registry, &made.id).expect("open");
    drop(registry);
    let fake = fixture("plain_turn");
    let mut app = Harness::start_project(fake.clone(), paths.clone(), open);

    app.send(Command::Library(LibraryCommand::BrowseProject {
        id: ProjectId("market-sizing".into()),
    }));
    app.wait_until("the browsed library", |a| {
        !a.failures.is_empty()
            || a.events
                .iter()
                .any(|e| matches!(e, Event::Library(LibraryEvent::ProjectBrowsed(_))))
    });
    assert!(app.failures.is_empty(), "{:?}", app.failures);
    let library = app
        .events
        .iter()
        .find_map(|e| match e {
            Event::Library(LibraryEvent::ProjectBrowsed(b)) => Some(b.clone()),
            _ => None,
        })
        .expect("browsed");
    assert_eq!(library.project.id, ProjectId("market-sizing".into()));
    assert_eq!(library.tasks[0].title, "size the market");
    assert_eq!(library.decisions[0].text, "Size by teams");
    assert_eq!(library.research.len(), 1);
    assert_eq!(library.research[0].rounds[0].brief, "Count the teams.");
    assert!(library.research[0].report_ms.is_some());
    assert_eq!(library.listings.len(), 1);
    assert!(
        library.docs.is_empty(),
        "a research project has no repo docs"
    );
    // The open project is still the open one, and its own snapshot is untouched.
    assert_eq!(
        app.snap.project.as_ref().map(|p| p.id.0.as_str()),
        Some("pricing-study")
    );
    assert!(app.snap.research.is_empty());
    assert!(app.snap.decisions.is_empty());

    let task = library.tasks[0].id;
    app.send(Command::Library(LibraryCommand::LoadBrowsed {
        project: ProjectId("market-sizing".into()),
        what: BrowseWhat::Research {
            task,
            what: ResearchWhat::Report,
        },
    }));
    app.wait_until("the browsed report", |a| {
        !a.failures.is_empty()
            || a.events
                .iter()
                .any(|e| matches!(e, Event::Library(LibraryEvent::BrowsedLoaded { .. })))
    });
    assert!(app.failures.is_empty(), "{:?}", app.failures);
    let report = app
        .events
        .iter()
        .find_map(|e| match e {
            Event::Library(LibraryEvent::BrowsedLoaded { document, .. }) => {
                Some(document.markdown.clone())
            }
            _ => None,
        })
        .expect("loaded");
    assert!(report.contains("About 40k teams."), "{report}");

    // An unknown project is refused.
    app.send(Command::Library(LibraryCommand::BrowseProject {
        id: ProjectId("nope".into()),
    }));
    app.wait_until("the refusal", |a| !a.failures.is_empty());
    assert!(app.failures[0].contains("nope"), "{:?}", app.failures);

    app.quit();
    // Nothing of the browsed project was written.
    assert!(
        std::fs::read(&state_db).expect("its state.db") == before,
        "the browsed state.db changed"
    );
    // SQLite may leave an empty `-wal` beside the file after a reader on a WAL database: no
    // frames, so nothing was written.
    let wal_len = std::fs::metadata(&wal).map_or(0, |m| m.len());
    assert_eq!(wal_len, 0, "browsing wrote to the WAL");
}

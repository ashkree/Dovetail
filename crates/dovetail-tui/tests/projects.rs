//! Projects: the project switcher (`<leader>pp`) marks research projects and opens one after
//! asking; `:project new <name> --research` creates one; a research project has no worktree
//! view (research-only projects, decided 2026-11-11).

mod support;

use dovetail_api::{
    Command, Event, ServerMsg, Snapshot,
    app::AppEvent,
    tracking::project::{ProjectCommand, ProjectEvent, ProjectId, ProjectKind, ProjectSummary},
};
use support::*;

fn leader(app: &mut dovetail_tui::App, keys: &str) {
    app.on_key(key(' '), 0);
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 1 + i as u64);
    }
}

fn code(id: &str, path: &str) -> ProjectSummary {
    ProjectSummary {
        id: ProjectId(id.into()),
        name: id.into(),
        repo_path: path.into(),
        kind: ProjectKind::Code,
        ..ProjectSummary::default()
    }
}

fn research(id: &str, name: &str) -> ProjectSummary {
    ProjectSummary {
        id: ProjectId(id.into()),
        name: name.into(),
        repo_path: format!("/home/you/.local/share/dovetail/projects/{id}").into(),
        kind: ProjectKind::Research,
        ..ProjectSummary::default()
    }
}

/// The test world with two code projects and one research project registered.
fn with_projects(open_research: bool) -> Snapshot {
    let mut snap = world();
    let business = snap.project.clone().expect("the test world has a project");
    let market = research("market-sizing", "Market sizing");
    snap.projects = vec![
        business.clone(),
        code("notes-site", "/src/notes-site"),
        market.clone(),
    ];
    if open_research {
        snap.project = Some(market);
    }
    snap
}

/// Renders a narrow-enough screen to read whole picker rows.
fn screen(app: &mut dovetail_tui::App) -> String {
    render(app, 140, 40)
}

#[test]
fn leader_p_p_lists_every_project_and_marks_research_ones() {
    let mut app = app_with(with_projects(false));
    leader(&mut app, "pp");
    let screen = screen(&mut app);
    assert!(screen.contains("Projects"), "{screen}");
    let market = screen
        .lines()
        .find(|l| l.contains("Market sizing"))
        .expect("the research project is listed");
    assert!(
        market.contains(dovetail_tui::overlay::switcher::RESEARCH_MARKER)
            && market.contains("research"),
        "{market}"
    );
    let notes = screen
        .lines()
        .find(|l| l.contains("notes-site") && l.contains("/src/notes-site"))
        .expect("a code project shows its folder");
    assert!(
        !notes.contains(dovetail_tui::overlay::switcher::RESEARCH_MARKER),
        "{notes}"
    );
    assert!(
        screen
            .lines()
            .any(|l| l.contains("business") && l.contains("open ·")),
        "{screen}"
    );
}

#[test]
fn picking_a_project_asks_then_sends_open_project() {
    let mut app = app_with(with_projects(false));
    leader(&mut app, "pp");
    type_keys(&mut app, "market");
    app.on_key(enter(), 90);
    assert!(app.take_commands().is_empty(), "nothing before the answer");
    assert!(screen(&mut app).contains("Open Market sizing?"));
    app.on_key(key('y'), 100);
    assert_eq!(
        app.take_commands(),
        vec![Command::Project(ProjectCommand::Open {
            id: ProjectId("market-sizing".into())
        })]
    );
    // The core answers by quitting; the UI quits with it, without asking.
    assert!(!app.should_quit());
    app.on_server(ServerMsg::Event {
        seq: 1,
        event: Event::Project(ProjectEvent::Opening {
            id: ProjectId("market-sizing".into()),
        }),
    });
    assert!(app.should_quit());
}

#[test]
fn picking_the_open_project_does_nothing() {
    let mut app = app_with(with_projects(false));
    leader(&mut app, "pp");
    type_keys(&mut app, "business");
    app.on_key(enter(), 90);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.note(), Some("business is already open"));
}

#[test]
fn project_new_research_sends_the_name() {
    let mut app = app_with(with_projects(false));
    type_keys(&mut app, ":project new Market sizing 2 --research");
    app.on_key(enter(), 500);
    assert_eq!(
        app.take_commands(),
        vec![Command::Project(ProjectCommand::NewResearch {
            name: "Market sizing 2".into()
        })]
    );
}

/// Types `:project new <name> --research`; the core's answer is a snapshot that lists the project.
fn create_project(app: &mut dovetail_tui::App, name: &str) {
    type_keys(app, &format!(":project new {name} --research"));
    app.on_key(enter(), 500);
    assert_eq!(
        app.take_commands(),
        vec![Command::Project(ProjectCommand::NewResearch {
            name: name.into()
        })]
    );
    let mut snap = with_projects(false);
    snap.projects.push(research("market-sizing-2", name));
    app.on_server(ServerMsg::Snapshot(snap));
}

#[test]
fn a_new_research_project_asks_to_open_it_now_and_y_switches() {
    let mut app = app_with(with_projects(false));
    create_project(&mut app, "Market sizing 2");
    let screen = render(&mut app, 120, 30);
    assert!(
        screen.contains("Open Market sizing 2 now? (y/n)"),
        "{screen}"
    );
    app.on_key(key('y'), 600);
    assert_eq!(
        app.take_commands(),
        vec![Command::Project(ProjectCommand::Open {
            id: ProjectId("market-sizing-2".into())
        })]
    );
}

#[test]
fn n_on_the_open_now_question_stays_and_it_is_not_asked_twice() {
    let mut app = app_with(with_projects(false));
    create_project(&mut app, "Market sizing 2");
    app.on_key(key('n'), 600);
    assert!(app.take_commands().is_empty());
    assert!(!app.should_quit());
    let mut snap = with_projects(false);
    snap.projects
        .push(research("market-sizing-2", "Market sizing 2"));
    app.on_server(ServerMsg::Snapshot(snap));
    let screen = render(&mut app, 120, 30);
    assert!(!screen.contains("now?"), "{screen}");
}

#[test]
fn a_refused_new_project_asks_nothing() {
    let mut app = app_with(with_projects(false));
    type_keys(&mut app, ":project new Market sizing --research");
    app.on_key(enter(), 500);
    app.take_commands();
    // The name is taken: the core says so, and a later snapshot still lists the old project.
    app.on_server(ServerMsg::Event {
        seq: 1,
        event: Event::App(AppEvent::CommandFailed {
            message: "there is already a research project named `Market sizing`".into(),
        }),
    });
    app.on_server(ServerMsg::Snapshot(with_projects(false)));
    let screen = render(&mut app, 120, 30);
    assert!(!screen.contains("now?"), "{screen}");
}

#[test]
fn project_new_without_research_says_how() {
    let mut app = app_with(with_projects(false));
    type_keys(&mut app, ":project new thing");
    app.on_key(enter(), 500);
    assert!(app.take_commands().is_empty());
    assert!(app.note().is_some_and(|n| n.contains("--research")));
}

#[test]
fn a_research_project_has_no_worktree_view() {
    let mut app = app_with(with_projects(true));
    leader(&mut app, "g");
    assert!(app.take_commands().is_empty(), "no RefreshWorktrees");
    assert_eq!(
        app.note(),
        Some("a research project has no repo, so no worktrees")
    );
}

fn recipe_started(profiles: &[&str]) -> ServerMsg {
    ServerMsg::Event {
        seq: 1,
        event: Event::Project(ProjectEvent::RecipeStarted {
            project: ProjectId("business".into()),
            name: "business".into(),
            path: "/home/you/.config/dovetail/projects/business.toml".into(),
            profiles: profiles.iter().map(|p| p.to_string()).collect(),
        }),
    }
}

#[test]
fn a_starter_recipe_offers_the_profiles_default_first() {
    let mut app = app_with(with_projects(false));
    app.on_server(recipe_started(&["maveron", "work"]));
    let screen = screen(&mut app);
    assert!(screen.contains("Profile for business"), "{screen}");
    let at = |needle: &str| screen.find(needle).expect(needle);
    assert!(
        at(dovetail_tui::overlay::switcher::NO_PROFILE) < at("maveron"),
        "{screen}"
    );
    // Enter on the first row keeps the default: nothing to send.
    app.on_key(enter(), 90);
    assert!(app.take_commands().is_empty());
    assert!(app.note().is_some_and(|n| n.contains("business.toml")));
}

#[test]
fn picking_a_profile_sends_it_for_the_recipe() {
    let mut app = app_with(with_projects(false));
    app.on_server(recipe_started(&["maveron", "work"]));
    type_keys(&mut app, "work");
    app.on_key(enter(), 90);
    assert_eq!(
        app.take_commands(),
        vec![Command::Project(ProjectCommand::SetRecipeProfile {
            project: ProjectId("business".into()),
            profile: "work".into(),
        })]
    );
}

#[test]
fn no_profiles_means_no_picker() {
    let mut app = app_with(with_projects(false));
    app.on_server(recipe_started(&[]));
    assert!(!screen(&mut app).contains("Profile for"));
    assert!(app.note().is_some_and(|n| n.contains("business.toml")));
}

//! Tangents (ADR 0017, amended 2026-11-11; M22), driven by `MockCore`'s two seeded tangents:
//! the switcher pins Tangents last with its marker, the side panel shows tangents next to side
//! chats and asks or attaches one, the morning's "new tangents" item opens the triage list, and
//! the list files, starts or drops a tangent.

mod support;

use dovetail_api::{
    Command,
    activity::inbox::InboxKind,
    agents::tangent::{TangentCommand, tangent_agent},
    tracking::project::ProjectId,
    tracking::task::TaskId,
};
use dovetail_mock::{MockConfig, MockCore};
use dovetail_tui::App;
use support::*;

fn start() -> (MockCore, App) {
    let core = MockCore::new(MockConfig::default());
    let app = app_with(core.state().clone());
    (core, app)
}

/// Hands the core the commands and the app every answer that is due now.
fn deliver(core: &mut MockCore, app: &mut App, commands: Vec<Command>) {
    for command in commands {
        core.handle(command);
    }
    while core.peek_delay() == 0 {
        let Some(scheduled) = core.next_message() else {
            break;
        };
        app.on_server(scheduled.msg);
    }
}

/// Runs the mock until `done`, handing over commands as they come.
fn run_until(core: &mut MockCore, app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..3_000 {
        if done(app) {
            return;
        }
        for command in app.take_commands() {
            core.handle(command);
        }
        if let Some(scheduled) = core.next_message() {
            app.on_server(scheduled.msg);
        }
    }
    panic!("never happened: {what}");
}

fn screen(app: &mut App) -> String {
    render(app, 160, 44)
}

fn tangents_item(core: &MockCore) -> Option<u32> {
    core.state().inbox.iter().find_map(|i| match i.kind {
        InboxKind::NewTangents { count } => Some(count),
        _ => None,
    })
}

#[test]
fn the_switcher_pins_tangents_last_with_its_own_marker() {
    let (_, mut app) = start();
    let items = dovetail_tui::overlay::switcher::project_items(
        &start().0.state().clone(),
        &Default::default(),
    );
    let last = items.last().expect("projects");
    assert_eq!(last.label, "Tangents");
    assert_eq!(last.glyph, dovetail_tui::overlay::switcher::TANGENT_MARKER);
    assert!(
        items[..items.len() - 1]
            .iter()
            .all(|i| i.glyph != dovetail_tui::overlay::switcher::TANGENT_MARKER)
    );
    // As drawn: Tangents is the last project row.
    press(&mut app, " pp");
    let screen = screen(&mut app);
    let line = |needle: &str| {
        screen
            .lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} missing:\n{screen}"))
    };
    let tangents = line("↗ Tangents");
    for other in ["· Dovetail", "· notes-site", "◇ Market sizing"] {
        assert!(line(other) < tangents, "{other} after Tangents:\n{screen}");
    }
}

#[test]
fn the_side_panel_shows_tangents_beside_side_chats_and_asks_one_with_t() {
    let (mut core, mut app) = start();
    press(&mut app, " ?");
    let first = screen(&mut app);
    assert!(first.contains("side-1"), "{first}");
    assert!(first.contains("1/2 ⇥"), "{first}");
    app.on_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        ),
        1,
    );
    assert_eq!(app.panel_agent(), Some(&tangent_agent(TaskId(1))));
    let shown = screen(&mut app);
    assert!(shown.contains("↗ tangent · a attach"), "{shown}");
    assert!(shown.contains("kitty sends images"), "{shown}");
    // The tree lists no tangent: it belongs to Tangents, not this project.
    press(&mut app, " ?");
    assert!(!screen(&mut app).contains("tangent-1"));
    press(&mut app, " ?");

    // `T` asks a new one; it opens in the panel and its answer is filed in Tangents.
    assert!(press(&mut app, "T").is_empty());
    assert!(screen(&mut app).contains("tangent ›"));
    type_keys(&mut app, "Which rye flour?");
    app.on_key(enter(), 900);
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::Tangent(TangentCommand::New {
            question: "Which rye flour?".into()
        })]
    );
    deliver(&mut core, &mut app, commands);
    assert_eq!(app.panel_agent(), Some(&tangent_agent(TaskId(3))));
    for _ in 0..3_000 {
        if core
            .state()
            .tangents
            .iter()
            .any(|t| t.task.id == TaskId(3) && t.research.is_some())
        {
            break;
        }
        for command in app.take_commands() {
            core.handle(command);
        }
        if let Some(scheduled) = core.next_message() {
            app.on_server(scheduled.msg);
        }
    }
    assert!(
        core.state()
            .tangents
            .iter()
            .any(|t| t.task.id == TaskId(3) && t.research.is_some()),
        "the mock files the answer when the turn ends"
    );
    // `:tangent <question>` asks one straight away.
    type_keys(&mut app, ":tangent And spelt?");
    app.on_key(enter(), 2_000);
    assert_eq!(
        app.take_commands(),
        vec![Command::Tangent(TangentCommand::New {
            question: "And spelt?".into()
        })]
    );
}

#[test]
fn a_attaches_the_tangent_shown_to_the_task_in_focus_after_asking() {
    let (mut core, mut app) = start();
    select(&mut app, "Fuzzy picker");
    let task = core
        .state()
        .tasks
        .iter()
        .find(|t| t.title.contains("Fuzzy picker"))
        .expect("the seeded task")
        .id;
    press(&mut app, " ?");
    app.on_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        ),
        1,
    );
    assert!(press(&mut app, "a").is_empty(), "nothing before the answer");
    let asked = screen(&mut app);
    assert!(
        asked.contains("Attach the tangent \"How does kitty"),
        "{asked}"
    );
    let commands = press(&mut app, "y");
    assert_eq!(
        commands,
        vec![Command::Tangent(TangentCommand::Attach {
            tangent: TaskId(1),
            task,
        })]
    );
    deliver(&mut core, &mut app, commands);
    assert!(core.state().tangents.iter().all(|t| t.task.id != TaskId(1)));
    assert_eq!(tangents_item(&core), Some(1), "one new tangent left");
    // Its session went with it; the panel moved on.
    assert_ne!(app.panel_agent(), Some(&tangent_agent(TaskId(1))));
    // `a` on a side chat explains itself.
    press(&mut app, "a");
    assert!(app.note().is_some_and(|n| n.contains("side chat")));
}

#[test]
fn the_morning_item_opens_the_list_to_file_start_or_drop_each_tangent() {
    let (mut core, mut app) = start();
    let inbox = screen(&mut app);
    assert!(inbox.contains("2 new tangents"), "{inbox}");

    // Enter on the item shows the tangents list in the main region.
    press(&mut app, " n");
    for _ in 0..5 {
        if screen(&mut app).contains("▸ ↗ coordinator") {
            break;
        }
        press(&mut app, "j");
    }
    app.on_key(enter(), 10);
    let list = screen(&mut app);
    assert!(list.contains("↗ tangents · 2 new of 2"), "{list}");
    assert!(list.contains("How does kitty"), "{list}");
    assert!(list.contains("Asked from Dovetail"), "{list}");

    // `f`: pick a project (Tangents is not offered), then it is filed there.
    press(&mut app, "f");
    let picker = screen(&mut app);
    assert!(picker.contains("File tangent into"), "{picker}");
    assert!(!picker.contains("↗ Tangents"), "{picker}");
    type_keys(&mut app, "market");
    app.on_key(enter(), 900);
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::Tangent(TangentCommand::File {
            tangent: TaskId(1),
            project: ProjectId("market-sizing".into()),
        })]
    );
    deliver(&mut core, &mut app, commands);
    assert_eq!(tangents_item(&core), Some(1));
    assert!(screen(&mut app).contains("↗ tangents · 1 new of 1"));

    // `s`: start a task from it in the open project; it stays, done.
    press(&mut app, "s");
    type_keys(&mut app, "dovetail");
    app.on_key(enter(), 900);
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::Tangent(TangentCommand::StartFrom {
            tangent: TaskId(2),
            project: ProjectId("dovetail".into()),
        })]
    );
    deliver(&mut core, &mut app, commands);
    assert!(
        core.state()
            .tasks
            .iter()
            .any(|t| t.notes.contains("Started from the tangent"))
    );
    assert_eq!(tangents_item(&core), None, "all triaged");
    let list = screen(&mut app);
    assert!(list.contains("started"), "{list}");

    // `d`: drop, after a y/n.
    assert!(press(&mut app, "d").is_empty());
    assert!(screen(&mut app).contains("Drop the tangent"));
    let commands = press(&mut app, "y");
    assert_eq!(
        commands,
        vec![Command::Tangent(TangentCommand::Drop {
            tangent: TaskId(2)
        })]
    );
    deliver(&mut core, &mut app, commands);
    assert!(screen(&mut app).contains("No tangents."));
}

#[test]
fn tangents_lists_them_any_time_and_enter_reads_the_answer() {
    let (mut core, mut app) = start();
    type_keys(&mut app, ":tangents");
    app.on_key(enter(), 900);
    assert!(screen(&mut app).contains("↗ tangents · 2 new of 2"));
    app.on_key(enter(), 1_000);
    assert!(app.reading_mode());
    run_until(
        &mut core,
        &mut app,
        "the answer in the reading column",
        |app| app.note() != Some("loading…"),
    );
    deliver(&mut core, &mut app, Vec::new());
    let read = render(&mut app, 200, 44);
    assert!(read.contains("kitty sends images"), "{read}");
}

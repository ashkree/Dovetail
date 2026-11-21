//! Tasks #39, #46: the model picker and driver-switch confirm modal. `:model [name]` and
//! `/model [name]` open the picker on the focused session, grouped by driver; picking an
//! own-driver row sends `SetModel`; picking another driver's row on the coordinator opens the
//! confirm modal (`Handoff` default, `Clean start`, `Esc` cancels) which sends `SwitchDriver`.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    AgentKind, AgentStatus, Command, DriverKind, Event, ModelOption, ReasoningEffort, ServerMsg,
    Snapshot,
};
use dovetail_tui::App;
use dovetail_tui::action::Action;
use dovetail_tui::command;
use dovetail_tui::keymap::{Key, Keymap, Scope};
use dovetail_tui::mode::Mode;
use support::*;

fn claude_models() -> Vec<ModelOption> {
    let listed = |name: &str, resolved: &str, efforts: &[ReasoningEffort]| ModelOption {
        name: name.into(),
        resolved: Some(resolved.into()),
        efforts: Some(efforts.to_vec()),
        driver: DriverKind::Claude,
    };
    vec![
        listed("opus", "claude-opus-5-5", &ReasoningEffort::ALL),
        listed("sonnet", "claude-sonnet-5-5", &ReasoningEffort::ALL),
        listed("haiku", "claude-haiku-5-5", &ReasoningEffort::ALL),
    ]
}

fn both_driver_models() -> Vec<ModelOption> {
    let mut models = claude_models();
    models.push(ModelOption {
        name: "gemini-3.8-flash".into(),
        resolved: Some("Gemini 3.8 Flash".into()),
        efforts: Some(vec![ReasoningEffort::Low, ReasoningEffort::High]),
        driver: DriverKind::Agy,
    });
    models
}

fn world_with(models: Vec<ModelOption>) -> Snapshot {
    let mut snap = world();
    snap.models = models;
    for a in &mut snap.agents {
        if a.kind == AgentKind::Coordinator {
            a.model = Some("claude-opus-5-5".into());
            a.effort = Some(ReasoningEffort::High);
        }
    }
    snap
}

fn world_with_models() -> Snapshot {
    world_with(claude_models())
}

fn coordinator(app: &App) -> dovetail_api::AgentId {
    app.snapshot()
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .map(|a| a.id.clone())
        .expect("coordinator")
}

fn down() -> KeyEvent {
    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
}

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn left() -> KeyEvent {
    KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
}

fn right() -> KeyEvent {
    KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)
}

#[test]
fn only_colon_model_names_the_selector() {
    let map = Keymap::default_map();
    let label = |keys: &str| {
        let keys = Key::parse_sequence(keys).expect("keys");
        dovetail_tui::keymap::label_sequence(&keys)
    };
    assert_eq!(
        map.bindings_of(Action::PickModel),
        Vec::<(Mode, Scope, String)>::new(),
        "no key: :model and /model only"
    );
    // `<leader>m` stays the message timeline.
    assert!(map.bindings_of(Action::ShowMessages).contains(&(
        Mode::Normal,
        Scope::Global,
        label("<leader>m")
    )));
    let parsed = command::parse("model sonnet").expect("parses");
    assert_eq!(parsed.action, Action::PickModel);
    assert_eq!(parsed.arg, "sonnet");
}

#[test]
fn rows_group_by_driver_in_driver_kind_all_order() {
    let mut app = app_with(world_with(both_driver_models()));
    type_keys(&mut app, ":model");
    app.on_key(enter(), 100);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("model · coordinator"), "{screen}");
    let claude_pos = screen.find("Claude Code").expect("Claude Code heading");
    let agy_pos = screen.find("agy").expect("agy heading");
    assert!(claude_pos < agy_pos, "Claude Code comes before agy");
}

#[test]
fn empty_agy_catalog_notes_in_coordinator_preview() {
    let mut app = app_with(world_with_models());
    type_keys(&mut app, ":model");
    app.on_key(enter(), 100);
    let screen = render(&mut app, 120, 40);
    assert!(
        screen.contains("agy models appear once an agy session has run"),
        "{screen}"
    );
}

#[test]
fn slices_hide_other_driver_rows_and_note_coordinator_only() {
    let mut app = app_with(world_with(both_driver_models()));
    // Focus the tree and move onto slice-4.
    press(&mut app, "l");
    select(&mut app, "slice-4");
    type_keys(&mut app, ":model");
    app.on_key(enter(), 100);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("model · slice-4"), "{screen}");
    assert!(screen.contains("Claude Code"), "{screen}");
    assert!(!screen.contains("gemini-3.8-flash"), "{screen}");
    assert!(
        screen.contains("Only the coordinator switches driver."),
        "{screen}"
    );
}

#[test]
fn enter_on_own_driver_row_emits_set_model() {
    let mut app = app_with(world_with_models());
    let id = coordinator(&app);
    type_keys(&mut app, ":model");
    app.on_key(enter(), 100);
    assert!(render(&mut app, 120, 40).contains("model · coordinator"));
    // Move to sonnet · xhigh (opus has 5 rows, sonnet has low/medium/high/xhigh = index 8).
    for i in 0..8 {
        app.on_key(down(), 110 + i);
    }
    assert!(app.take_commands().is_empty(), "nothing sent before Enter");
    app.on_key(enter(), 200);
    assert_eq!(
        app.take_commands(),
        vec![Command::SetModel {
            agent: id,
            model: "sonnet".into(),
            effort: Some(ReasoningEffort::Xhigh),
        }]
    );
    assert_eq!(app.mode(), Mode::Normal);
    assert!(!render(&mut app, 120, 40).contains("model · coordinator"));
}

#[test]
fn esc_closes_the_picker_without_sending() {
    let mut app = app_with(world_with_models());
    type_keys(&mut app, ":model");
    app.on_key(enter(), 50);
    assert!(render(&mut app, 120, 40).contains("model · coordinator"));
    app.on_key(down(), 60);
    app.on_key(esc(), 100);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.mode(), Mode::Normal);
    assert!(!render(&mut app, 120, 40).contains("model · coordinator"));
}

#[test]
fn slash_model_and_colon_model_prefill_the_picker_query() {
    let mut app = app_with(world_with_models());
    let id = coordinator(&app);
    type_keys(&mut app, " c");
    press(&mut app, "i");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "/model haiku high");
    app.on_key(enter(), 500);
    assert!(
        app.take_commands().is_empty(),
        "/model never reaches Claude Code"
    );
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("model · coordinator"), "{screen}");
    assert!(screen.contains("› haiku high"), "{screen}");
    app.on_key(enter(), 600);
    assert_eq!(
        app.take_commands(),
        vec![Command::SetModel {
            agent: id.clone(),
            model: "haiku".into(),
            effort: Some(ReasoningEffort::High),
        }]
    );

    // `:model sonnet` also prefills the picker query.
    type_keys(&mut app, ":model sonnet xhigh");
    app.on_key(enter(), 700);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("› sonnet xhigh"), "{screen}");
    app.on_key(enter(), 800);
    assert_eq!(
        app.take_commands(),
        vec![Command::SetModel {
            agent: id,
            model: "sonnet".into(),
            effort: Some(ReasoningEffort::Xhigh),
        }]
    );
}

#[test]
fn picking_another_driver_opens_confirm_and_sends_switch_driver() {
    let open_agy_confirm = |app: &mut App| {
        type_keys(app, ":model gemini high");
        app.on_key(enter(), 100);
        let picker_screen = render(app, 120, 40);
        assert!(
            picker_screen.contains("Starts a new coordinator session on agy."),
            "{picker_screen}"
        );
        app.on_key(enter(), 200);
        assert!(
            app.take_commands().is_empty(),
            "picking agy opens confirm, does not send yet"
        );
        let confirm_screen = render(app, 120, 40);
        assert!(
            confirm_screen.contains("This starts a new session on agy."),
            "{confirm_screen}"
        );
        assert!(confirm_screen.contains("Handoff"), "{confirm_screen}");
        assert!(confirm_screen.contains("Clean start"), "{confirm_screen}");
    };

    // Default Enter sends SwitchDriver { handoff: true }.
    let mut app = app_with(world_with(both_driver_models()));
    let id = coordinator(&app);
    open_agy_confirm(&mut app);
    app.on_key(enter(), 300);
    assert_eq!(
        app.take_commands(),
        vec![Command::SwitchDriver {
            agent: id.clone(),
            driver: DriverKind::Agy,
            model: "gemini-3.8-flash".into(),
            effort: Some(ReasoningEffort::High),
            handoff: true,
        }]
    );
    assert_eq!(app.mode(), Mode::Normal);
    assert!(!render(&mut app, 120, 40).contains("This starts a new session"));

    // Tab toggles to Clean start; Enter sends SwitchDriver { handoff: false }.
    let mut app = app_with(world_with(both_driver_models()));
    open_agy_confirm(&mut app);
    app.on_key(tab(), 300);
    app.on_key(enter(), 310);
    assert_eq!(
        app.take_commands(),
        vec![Command::SwitchDriver {
            agent: id.clone(),
            driver: DriverKind::Agy,
            model: "gemini-3.8-flash".into(),
            effort: Some(ReasoningEffort::High),
            handoff: false,
        }]
    );

    // l / Right select Clean start; h / Left return to Handoff.
    let mut app = app_with(world_with(both_driver_models()));
    open_agy_confirm(&mut app);
    app.on_key(right(), 300);
    app.on_key(left(), 310);
    press(&mut app, "l");
    app.on_key(enter(), 320);
    assert_eq!(
        app.take_commands(),
        vec![Command::SwitchDriver {
            agent: id,
            driver: DriverKind::Agy,
            model: "gemini-3.8-flash".into(),
            effort: Some(ReasoningEffort::High),
            handoff: false,
        }]
    );

    // Esc cancels without sending SwitchDriver.
    let mut app = app_with(world_with(both_driver_models()));
    open_agy_confirm(&mut app);
    app.on_key(esc(), 300);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.mode(), Mode::Normal);
    assert!(!render(&mut app, 120, 40).contains("This starts a new session"));
}

#[test]
fn the_usage_row_shows_model_and_effort_once_the_core_confirms() {
    let mut app = app_with(world_with_models());
    type_keys(&mut app, " c");
    assert!(render(&mut app, 160, 40).contains("opus-5-5 high"));
    let mut agent = app
        .snapshot()
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .cloned()
        .expect("coordinator");
    agent.model = Some("claude-sonnet-5-5".into());
    agent.effort = Some(ReasoningEffort::Max);
    agent.status = AgentStatus::Idle;
    app.on_server(ServerMsg::Event {
        seq: app.snapshot().seq + 1,
        event: Event::AgentUpserted(agent),
    });
    assert!(render(&mut app, 160, 40).contains("sonnet-5-5 max"));
}

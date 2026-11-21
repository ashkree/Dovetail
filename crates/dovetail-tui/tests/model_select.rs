//! Task #39: the model selector. `:model` and `/model` open it on the focused
//! session; `j`/`k`/`h`/`l` pick, Enter sends one `SetModel` and Esc sends nothing.

mod support;

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

fn models() -> Vec<ModelOption> {
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

fn world_with_models() -> Snapshot {
    let mut snap = world();
    snap.models = models();
    for a in &mut snap.agents {
        if a.kind == AgentKind::Coordinator {
            a.model = Some("claude-opus-5-5".into());
            a.effort = Some(ReasoningEffort::High);
        }
    }
    snap
}

fn coordinator(app: &App) -> dovetail_api::AgentId {
    app.snapshot()
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Coordinator)
        .map(|a| a.id.clone())
        .expect("coordinator")
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
fn enter_applies_the_pick_to_the_focused_session() {
    let mut app = app_with(world_with_models());
    let id = coordinator(&app);
    type_keys(&mut app, ":model");
    app.on_key(enter(), 100);
    assert!(render(&mut app, 120, 40).contains("model · coordinator"));
    // Down to sonnet, effort up from high to xhigh.
    let sent = press(&mut app, "jl");
    assert!(sent.is_empty(), "nothing is sent before Enter: {sent:?}");
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
fn esc_closes_the_selector_without_sending() {
    let mut app = app_with(world_with_models());
    type_keys(&mut app, ":model");
    app.on_key(enter(), 50);
    assert!(render(&mut app, 120, 40).contains("model · coordinator"));
    press(&mut app, "jjhh");
    app.on_key(esc(), 100);
    assert!(app.take_commands().is_empty());
    assert!(!render(&mut app, 120, 40).contains("model · coordinator"));
}

#[test]
fn slash_model_in_the_chat_bar_opens_the_selector_and_is_never_sent() {
    let mut app = app_with(world_with_models());
    let id = coordinator(&app);
    type_keys(&mut app, " c");
    press(&mut app, "i");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "/model haiku");
    app.on_key(enter(), 500);
    assert!(
        app.take_commands().is_empty(),
        "/model never reaches Claude Code"
    );
    assert!(render(&mut app, 120, 40).contains("model · coordinator"));
    // Opened on the name typed after it.
    app.on_key(enter(), 600);
    assert_eq!(
        app.take_commands(),
        vec![Command::SetModel {
            agent: id,
            model: "haiku".into(),
            effort: Some(ReasoningEffort::High),
        }]
    );
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

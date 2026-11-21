//! M15 (e): the map shelf (`<leader>M`) and the outline, driven by `MockCore`'s two seeded maps:
//! the structure map "Dovetail at a glance" and the swimlane map "Answering a permission
//! request" (drawn later, so first on the shelf). `Enter` fetches a map with `LoadMap`; `o`
//! folds a group, `f` switches flow, `]]`/`[[` walk a flow's steps, `Esc` goes back a step.

mod support;

use dovetail_api::map::{MapCommand, MapEvent, MapId};
use dovetail_api::{Command, Event, ServerMsg};
use dovetail_mock::{MockConfig, MockCore};
use dovetail_tui::App;
use dovetail_tui::region::RegionId;
use support::*;

fn start() -> (MockCore, App) {
    let core = MockCore::new(MockConfig::default());
    let app = app_with(core.state().clone());
    (core, app)
}

fn leader(app: &mut App, keys: &str) {
    app.on_key(key(' '), 0);
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 1 + i as u64);
    }
}

/// Hands the core the app's commands and the app the core's answers.
fn deliver(core: &mut MockCore, app: &mut App) {
    for command in app.take_commands() {
        core.handle(command);
    }
    while core.peek_delay() == 0 {
        let Some(scheduled) = core.next_message() else {
            break;
        };
        app.on_server(scheduled.msg);
    }
}

fn shelf() -> (MockCore, App) {
    let (core, mut app) = start();
    leader(&mut app, "M");
    (core, app)
}

/// The shelf, with the map on row `row` opened. Opening goes to MAP mode (M15 f); `Esc` leaves
/// it for the outline in Normal mode, which these tests drive.
fn outline(row: usize) -> (MockCore, App) {
    let (mut core, mut app) = shelf();
    for _ in 0..row {
        app.on_key(key('j'), 0);
    }
    app.on_key(enter(), 0);
    deliver(&mut core, &mut app);
    assert_eq!(app.mode(), dovetail_tui::Mode::Map);
    app.on_key(esc(), 0);
    (core, app)
}

fn press_seq(app: &mut App, keys: &str) {
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), i as u64);
    }
}

#[test]
fn leader_big_m_shows_the_shelf_newest_first_and_again_goes_back() {
    let (_core, mut app) = shelf();
    assert_eq!(app.focus(), RegionId::Main);
    insta::assert_snapshot!(render(&mut app, 120, 24));
    leader(&mut app, "M");
    assert!(!render(&mut app, 120, 24).contains("maps · 2"));
}

#[test]
fn enter_asks_for_the_map_and_shows_the_outline_when_it_arrives() {
    let (mut core, mut app) = shelf();
    app.on_key(enter(), 0);
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::Map(MapCommand::Load {
            id: MapId("permission-round-trip".into()),
            version: None
        })]
    );
    // Still the shelf until the map arrives.
    assert!(render(&mut app, 120, 24).contains("maps · 2"));
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    assert!(render(&mut app, 120, 24).contains("Answering a permission request"));
}

#[test]
fn the_swimlane_outline_lists_nodes_in_flow_order_with_lanes_and_the_flow() {
    let (_core, mut app) = outline(0);
    insta::assert_snapshot!(render(&mut app, 120, 44));
}

#[test]
fn the_structure_outline_nests_groups_and_shows_the_selected_node_in_full() {
    let (_core, mut app) = outline(1);
    // user, tui, keymap (inside tui): move onto keymap.
    press_seq(&mut app, "jj");
    insta::assert_snapshot!(render(&mut app, 120, 44));
}

#[test]
fn o_folds_a_group_and_the_cursor_stays_on_it() {
    let (_core, mut app) = outline(1);
    press_seq(&mut app, "j");
    let open = render(&mut app, 120, 44);
    assert!(open.contains("keymap.rs"));
    press_seq(&mut app, "o");
    let folded = render(&mut app, 120, 44);
    assert!(folded.contains("1 inside"), "{folded}");
    assert!(
        !folded.lines().any(|l| l.contains("▤ keymap.rs")),
        "{folded}"
    );
    press_seq(&mut app, "o");
    assert!(render(&mut app, 120, 44).contains("▤ keymap.rs"));
}

#[test]
fn square_brackets_walk_the_flow_and_unfold_what_hides_the_step() {
    let (_core, mut app) = outline(1);
    // Fold tui, then walk: user -> tui -> core (tui has no hidden step), cursor follows.
    press_seq(&mut app, "jo");
    press_seq(&mut app, "gg]]");
    let at_tui = render(&mut app, 120, 45);
    assert!(at_tui.contains("▸ 2 dovetail-tui"), "{at_tui}");
    press_seq(&mut app, "]]");
    let at_core = render(&mut app, 120, 45);
    assert!(at_core.contains("▸ 3 dovetail-core"), "{at_core}");
    press_seq(&mut app, "[[[[");
    assert!(render(&mut app, 120, 45).contains("▸ 1 You"));
    // Before the first step: a note, not a wrap.
    press_seq(&mut app, "[[");
    assert!(app.note().is_some_and(|n| n.contains("first step")));
}

#[test]
fn esc_goes_outline_then_shelf_then_conversation() {
    let (_core, mut app) = outline(0);
    app.on_key(esc(), 0);
    assert!(render(&mut app, 120, 24).contains("maps · 2"));
    app.on_key(esc(), 0);
    assert!(!render(&mut app, 120, 24).contains("maps · 2"));
}

#[test]
fn a_newer_version_of_the_open_map_is_fetched_and_keeps_the_cursor() {
    let (_core, mut app) = outline(0);
    press_seq(&mut app, "jj");
    let before = render(&mut app, 120, 44);
    let mut summary = app.snapshot().maps[0].clone();
    summary.version += 1;
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::Map(MapEvent::Upserted(summary)),
    });
    let commands = app.take_commands();
    assert!(matches!(
        commands.as_slice(),
        [Command::Map(MapCommand::Load { version: None, .. })]
    ));
    // Nothing else changes while the new version is on its way.
    assert_eq!(render(&mut app, 120, 44), before);
}

#[test]
fn it_reads_in_a_narrow_pane_too() {
    let (_core, mut app) = outline(1);
    insta::assert_snapshot!(render(&mut app, 70, 24));
}

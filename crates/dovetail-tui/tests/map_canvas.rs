//! M15 (f): the kitty canvas and MAP mode, with a hand-made map and picture standing in for the
//! painter: no merman here, only where the picture puts each node.
//!
//! The map: `a` and `b` side by side on top (a → b), group `g` below holding `c`. The terminal
//! is 120×40 cells of 10×20 px.

mod support;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use dovetail_api::{Command, Event, ServerMsg};
use dovetail_api::{
    conversation::ConversationCommand,
    map::{
        Connection, Group, Map, MapCommand, MapContent, MapEvent, MapId, MapKind, MapSummary, Node,
        NodeId, NodeKind,
    },
};
use dovetail_tui::canvas::kitty::Scene;
use dovetail_tui::canvas::view::CellPx;
use dovetail_tui::canvas::{Painted, Picture, Placed, PxRect};
use dovetail_tui::event_loop::MouseCapture;
use dovetail_tui::{App, Mode};
use support::*;

const CELL: CellPx = CellPx { w: 10, h: 20 };
const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

fn id(s: &str) -> NodeId {
    NodeId(s.into())
}

fn node(name: &str, kind: NodeKind) -> Node {
    Node {
        id: id(name),
        kind,
        label: format!("node {name}"),
        description: String::new(),
        file: None,
        pseudocode: None,
    }
}

fn map() -> Map {
    Map {
        id: MapId("sample".into()),
        version: 1,
        content: MapContent {
            title: "Sample".into(),
            kind: MapKind::Structure,
            nodes: vec![
                node("a", NodeKind::Endpoint),
                node("b", NodeKind::Data),
                node("g", NodeKind::Part),
                node("c", NodeKind::File),
            ],
            groups: vec![Group {
                parent: id("g"),
                children: vec![id("c")],
            }],
            connections: vec![Connection {
                from: id("a"),
                to: id("b"),
                label: "reads".into(),
            }],
            flows: vec![],
            lanes: vec![],
        },
    }
}

fn placed(name: &str, x: f64, y: f64, w: f64, h: f64, is_group: bool) -> Placed {
    Placed {
        id: id(name),
        rect: PxRect { x, y, w, h },
        is_group,
        step: None,
    }
}

/// An 800×400 picture: `a` top left, `b` top right, group `g` filling the bottom left with `c`.
fn picture() -> Arc<Picture> {
    Arc::new(Picture {
        png: vec![0x89, b'P', b'N', b'G'],
        width: 800,
        height: 400,
        nodes: vec![
            placed("a", 50.0, 50.0, 100.0, 60.0, false),
            placed("b", 600.0, 50.0, 100.0, 60.0, false),
            placed("g", 0.0, 200.0, 400.0, 200.0, true),
            placed("c", 100.0, 260.0, 100.0, 60.0, false),
        ],
    })
}

/// The app with the map open from the shelf, in MAP mode; `cell` turns graphics on.
fn open(cell: Option<CellPx>) -> App {
    let mut snap = world();
    snap.maps = vec![MapSummary {
        id: MapId("sample".into()),
        title: "Sample".into(),
        kind: MapKind::Structure,
        task: None,
        drawn_by: None,
        version: 1,
        created_ms: 0,
        updated_ms: 0,
    }];
    let mut app = app_with(snap);
    app.set_graphics(cell);
    type_keys(&mut app, " M");
    app.on_key(enter(), 100);
    assert!(matches!(
        app.take_commands().as_slice(),
        [Command::Map(MapCommand::Load { .. })]
    ));
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::Map(MapEvent::Loaded(map())),
    });
    app
}

/// Draws a frame, as the loop does, and returns the scene kitty would be given.
fn frame(app: &mut App) -> (String, Option<Scene>) {
    let screen = render(app, WIDTH, HEIGHT);
    (screen, app.graphics_scene())
}

/// Opens the map with graphics and hands back the painting it asks for.
fn painted() -> App {
    let mut app = open(Some(CELL));
    frame(&mut app);
    let job = app
        .take_paint_job()
        .expect("the open map asks to be painted");
    assert_eq!(job.key.map, MapId("sample".into()));
    app.on_painted(Painted {
        key: job.key,
        result: Ok(picture()),
    });
    app
}

/// The screen cell showing picture point (x, y).
fn cell_at(scene: &Scene, x: f64, y: f64) -> (u16, u16) {
    let p = scene.placement;
    let sx = f64::from(p.cols) * f64::from(CELL.w) / f64::from(p.src_w);
    let sy = f64::from(p.rows) * f64::from(CELL.h) / f64::from(p.src_h);
    (
        p.col + ((x - f64::from(p.src_x)) * sx / f64::from(CELL.w)) as u16,
        p.row + ((y - f64::from(p.src_y)) * sy / f64::from(CELL.h)) as u16,
    )
}

fn click(app: &mut App, (column, row): (u16, u16)) {
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.on_mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }
}

fn special(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE), 0);
}

#[test]
fn opening_a_map_enters_map_mode_full_screen() {
    let mut app = painted();
    assert_eq!(app.mode(), Mode::Map);
    let (screen, scene) = frame(&mut app);
    assert!(screen.contains(" MAP "), "{screen}");
    // The side column is gone: the map pane runs the whole width.
    assert!(!screen.contains("Needs you"), "{screen}");
    let scene = scene.expect("the picture is on screen");
    assert!(scene.placement.cols > 0 && scene.placement.rows > 0);
    // The outline sits beside the canvas.
    assert!(screen.contains("node a"), "{screen}");
}

/// Decision 23f: mouse capture only in MAP mode. The loop calls `MouseCapture::sync` with
/// `App::wants_mouse` after every message; this walks the same calls through each mode.
#[test]
fn mouse_capture_is_on_in_map_mode_only() {
    let mut app = open(None);
    let mut capture = MouseCapture::default();
    let mut out = Vec::new();
    let mut sync = |app: &App| {
        out.clear();
        capture
            .sync(app.wants_mouse(), &mut out)
            .expect("write to a Vec");
        (
            capture.is_on(),
            String::from_utf8(out.clone()).expect("ascii"),
        )
    };
    let (on, written) = sync(&app);
    assert!(
        on && written.contains("\x1b[?1000h"),
        "MAP mode: {written:?}"
    );
    // Esc: Normal mode, capture off.
    app.on_key(esc(), 0);
    assert_eq!(app.mode(), Mode::Normal);
    let (on, written) = sync(&app);
    assert!(
        !on && written.contains("\x1b[?1000l"),
        "Normal: {written:?}"
    );
    // <leader>v: back in MAP mode.
    type_keys(&mut app, " v");
    assert_eq!(app.mode(), Mode::Map);
    assert!(sync(&app).0);
    // `:` from MAP mode: Command mode, capture off.
    app.on_key(key(':'), 0);
    assert_eq!(app.mode(), Mode::Command);
    assert!(!sync(&app).0);
    app.on_key(esc(), 0);
    // Insert mode (the chat bar): off.
    app.on_key(key('i'), 0);
    assert_eq!(app.mode(), Mode::Insert);
    assert!(!sync(&app).0);
    app.on_key(esc(), 0);
    // A fresh app, never in MAP mode, never asks.
    let fresh = app_with(world());
    assert!(!fresh.wants_mouse());
    // Mouse events outside MAP mode do nothing.
    let mut normal = open(Some(CELL));
    normal.on_key(esc(), 0);
    click(&mut normal, (10, 10));
    assert_eq!(normal.selected_map_node(), None);
}

#[test]
fn a_click_selects_the_node_drawn_under_the_cell() {
    let mut app = painted();
    let (_, scene) = frame(&mut app);
    let scene = scene.expect("on screen");
    click(&mut app, cell_at(&scene, 650.0, 80.0));
    assert_eq!(app.selected_map_node(), Some(&id("b")));
    // Inside the group, the inner node wins.
    click(&mut app, cell_at(&scene, 150.0, 290.0));
    assert_eq!(app.selected_map_node(), Some(&id("c")));
    // The group's own space selects the group.
    click(&mut app, cell_at(&scene, 300.0, 380.0));
    assert_eq!(app.selected_map_node(), Some(&id("g")));
    // The ring follows the selection.
    let (_, scene) = frame(&mut app);
    assert!(scene.and_then(|s| s.ring).is_some());
}

#[test]
fn hjkl_move_to_the_nearest_node_that_way() {
    let mut app = painted();
    frame(&mut app);
    // Nothing selected: the first key picks the node nearest the middle.
    app.on_key(key('l'), 0);
    assert!(app.selected_map_node().is_some());
    let scene = frame(&mut app).1.expect("on screen");
    click(&mut app, cell_at(&scene, 100.0, 80.0));
    assert_eq!(app.selected_map_node(), Some(&id("a")));
    app.on_key(key('l'), 0);
    assert_eq!(app.selected_map_node(), Some(&id("b")));
    app.on_key(key('h'), 0);
    assert_eq!(app.selected_map_node(), Some(&id("a")));
    app.on_key(key('j'), 0);
    assert_eq!(app.selected_map_node(), Some(&id("c")));
    // Tab follows a → b.
    app.on_key(key('k'), 0);
    special(&mut app, KeyCode::Tab);
    assert_eq!(app.selected_map_node(), Some(&id("b")));
    special(&mut app, KeyCode::BackTab);
    assert_eq!(app.selected_map_node(), Some(&id("a")));
}

#[test]
fn zoom_and_pan_crop_the_cached_picture_without_repainting() {
    let mut app = painted();
    let (_, before) = frame(&mut app);
    let before = before.expect("on screen");
    assert_eq!((before.placement.src_w, before.placement.src_h), (800, 400));
    app.on_key(key('+'), 0);
    app.on_key(key('+'), 0);
    let (screen, zoomed) = frame(&mut app);
    let zoomed = zoomed.expect("on screen");
    assert!(
        Arc::ptr_eq(&before.picture, &zoomed.picture),
        "same picture"
    );
    assert!(zoomed.placement.src_w < 800, "{:?}", zoomed.placement);
    assert!(screen.contains("1.6×"), "{screen}");
    assert!(app.take_paint_job().is_none(), "zoom never repaints");
    let x = zoomed.placement.src_x;
    app.on_key(ctrl('l'), 0);
    let panned = frame(&mut app).1.expect("on screen");
    assert!(panned.placement.src_x > x, "{:?}", panned.placement);
    app.on_key(key('0'), 0);
    let fit = frame(&mut app).1.expect("on screen");
    assert_eq!(fit.placement, before.placement);
    // Selecting never repaints either: the ring is its own image.
    app.on_key(key('l'), 0);
    frame(&mut app);
    assert!(app.take_paint_job().is_none());
}

#[test]
fn a_scroll_zooms_around_the_pointer() {
    let mut app = painted();
    let scene = frame(&mut app).1.expect("on screen");
    let at = cell_at(&scene, 650.0, 80.0);
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: at.0,
        row: at.1,
        modifiers: KeyModifiers::NONE,
    });
    let zoomed = frame(&mut app).1.expect("on screen");
    assert!(zoomed.placement.src_w < 800);
    // b is still under the pointer.
    click(&mut app, at);
    assert_eq!(app.selected_map_node(), Some(&id("b")));
}

#[test]
fn enter_drills_into_a_group_and_backspace_comes_back() {
    let mut app = painted();
    let scene = frame(&mut app).1.expect("on screen");
    click(&mut app, cell_at(&scene, 300.0, 380.0));
    assert_eq!(app.selected_map_node(), Some(&id("g")));
    app.on_key(enter(), 0);
    let inside = frame(&mut app).1.expect("on screen");
    assert!(inside.placement.src_w < 800, "{:?}", inside.placement);
    special(&mut app, KeyCode::Backspace);
    let back = frame(&mut app).1.expect("on screen");
    assert_eq!(back.placement, scene.placement);
    // On a plain node, Enter only says there is nothing inside.
    click(&mut app, cell_at(&scene, 100.0, 80.0));
    app.on_key(enter(), 0);
    assert!(app.note().is_some_and(|n| n.contains("nothing inside")));
}

#[test]
fn a_popup_hides_the_picture_and_leaving_the_map_takes_it_away() {
    let mut app = painted();
    assert!(frame(&mut app).1.is_some());
    // `u` asks first; the question row does not cover the canvas.
    app.on_key(key('u'), 0);
    assert!(frame(&mut app).0.contains("Ask the coordinator to redraw"));
    app.on_key(key('n'), 0);
    assert!(app.take_commands().is_empty());
    app.on_key(esc(), 0);
    assert_eq!(app.mode(), Mode::Normal);
    // Esc again: the shelf, so no picture.
    app.on_key(esc(), 0);
    assert!(frame(&mut app).1.is_none());
}

#[test]
fn u_sends_the_coordinator_a_redraw_request_after_a_yes() {
    let mut app = painted();
    app.on_key(key('u'), 0);
    app.on_key(key('y'), 0);
    let commands = app.take_commands();
    let [Command::Conversation(ConversationCommand::Prompt { text, .. })] = commands.as_slice()
    else {
        panic!("one prompt: {commands:?}");
    };
    assert!(
        text.contains("`sample`") && text.contains("maps_draw"),
        "{text}"
    );
}

#[test]
fn without_graphics_the_outline_is_the_whole_view() {
    let mut app = open(None);
    assert_eq!(app.mode(), Mode::Map);
    let (screen, scene) = frame(&mut app);
    assert!(scene.is_none());
    assert!(app.take_paint_job().is_none(), "nothing to paint for");
    assert!(screen.contains("node a"), "{screen}");
    // j and k walk the rows; zoom says why it cannot.
    app.on_key(key('j'), 0);
    assert!(app.selected_map_node().is_some());
    app.on_key(key('+'), 0);
    assert!(app.note().is_some_and(|n| n.contains("cannot show")));
}

#[test]
fn a_failed_painting_says_why_and_is_not_retried() {
    let mut app = open(Some(CELL));
    frame(&mut app);
    let job = app.take_paint_job().expect("asked");
    app.on_painted(Painted {
        key: job.key,
        result: Err(dovetail_tui::canvas::PaintError("merman said no".into())),
    });
    let (screen, scene) = frame(&mut app);
    assert!(screen.contains("merman said no"), "{screen}");
    assert!(scene.is_none());
    assert!(app.take_paint_job().is_none());
}

/// A paste in MAP mode is not typed as keys: `hjkl` in it would move the selection.
#[test]
fn a_paste_in_map_mode_does_nothing() {
    let mut app = painted();
    frame(&mut app);
    app.on_paste("lllj+", 0);
    assert_eq!(app.selected_map_node(), None);
    assert_eq!(app.mode(), Mode::Map);
}

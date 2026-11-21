//! The working indicators: the line under a working agent's conversation, the status bar's
//! long-command spinner, and the rule that nothing animates unless one is on screen.

mod support;

use dovetail_api::*;
use support::*;

/// The b.png world with the coordinator working, its conversation in view. The slices are idle,
/// so the only spinner is the chat's.
fn working_app() -> dovetail_tui::App {
    let mut snap = world();
    for a in snap
        .agents
        .iter_mut()
        .filter(|a| a.status == AgentStatus::Working)
    {
        a.status = AgentStatus::Idle;
    }
    snap.conversations
        .insert(agent_id("coordinator"), rich_conversation(NOW_MS));
    snap.agents[0].status = AgentStatus::Working;
    app_with(snap)
}

/// Everything idle: nothing may animate.
fn idle_app() -> dovetail_tui::App {
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    app_with(snap)
}

fn draw(app: &mut dovetail_tui::App) -> String {
    render(app, 100, 30)
}

fn line_with<'a>(screen: &'a str, needle: &str) -> Option<&'a str> {
    screen.lines().find(|l| l.contains(needle))
}

#[test]
fn a_working_agent_with_no_tool_shows_a_woodworking_word_and_elapsed_time() {
    let mut app = working_app();
    app.set_now(NOW_MS + 12_000);
    let screen = draw(&mut app);
    let line = line_with(&screen, "12s").expect("the working line");
    assert!(
        dovetail_tui::activity::WORDS
            .iter()
            .any(|w| line.contains(&format!("{w}…"))),
        "{line}"
    );
}

#[test]
fn a_running_tool_is_named_instead_of_a_word() {
    let mut app = working_app();
    app.on_server(ServerMsg::Event {
        seq: app_seq(&app),
        event: Event::AgentUpserted({
            let mut a = app_agent(&app, "coordinator");
            a.tool = Some(RunningTool {
                name: "Read".into(),
                arg: "/repo/docs/plan.md".into(),
            });
            a
        }),
    });
    app.set_now(NOW_MS + 65_000);
    let screen = draw(&mut app);
    let line = line_with(&screen, "Reading plan.md…").expect("the working line");
    assert!(line.contains("1m 05s"), "{line}");
    insta::assert_snapshot!(screen);
}

#[test]
fn the_word_is_kept_for_the_whole_turn() {
    let mut app = working_app();
    let word_at = |app: &mut dovetail_tui::App, ms: i64| {
        app.set_now(NOW_MS + ms);
        let screen = draw(app);
        dovetail_tui::activity::WORDS
            .iter()
            .find(|w| screen.contains(&format!("{w}…")))
            .copied()
    };
    let first = word_at(&mut app, 1_000);
    assert!(first.is_some());
    assert_eq!(word_at(&mut app, 9_000), first);
    assert_eq!(word_at(&mut app, 70_000), first);
}

#[test]
fn an_idle_agent_has_no_working_line() {
    let mut app = idle_app();
    app.set_now(NOW_MS + 5_000);
    let screen = draw(&mut app);
    assert!(
        !dovetail_tui::activity::WORDS
            .iter()
            .any(|w| screen.contains(&format!("{w}…")))
    );
}

#[test]
fn the_working_line_does_not_move_the_transcript() {
    let mut calm = working_app();
    let mut snap = calm.snapshot().clone();
    snap.agents[0].status = AgentStatus::Idle;
    let mut idle = app_with(snap);
    let (a, b) = (draw(&mut idle), draw(&mut calm));
    // The row under the text is reserved either way, so the text is on the same rows.
    let row_of = |screen: &str| {
        screen
            .lines()
            .position(|l| l.contains("let key = std::env::var"))
    };
    assert!(row_of(&a).is_some());
    assert_eq!(row_of(&a), row_of(&b));
}

// --- animation is scheduled only while something is on screen ---

#[test]
fn an_idle_app_schedules_no_wake() {
    let mut app = idle_app();
    draw(&mut app);
    assert_eq!(app.next_animation_in(), None);
    assert!(!app.animating());
}

#[test]
fn a_working_app_wakes_within_a_frame() {
    let mut app = working_app();
    draw(&mut app);
    let wake = app.next_animation_in().expect("a spinner is on screen");
    assert!(wake <= std::time::Duration::from_millis(125), "{wake:?}");
    assert!(wake >= std::time::Duration::from_millis(1));
}

#[test]
fn animation_stops_when_the_agent_stops_working() {
    let mut app = working_app();
    draw(&mut app);
    assert!(app.animating());
    let mut done = app_agent(&app, "coordinator");
    done.status = AgentStatus::Idle;
    app.on_server(ServerMsg::Event {
        seq: app_seq(&app),
        event: Event::AgentUpserted(done),
    });
    // The redraw is what takes the spinner off the screen.
    draw(&mut app);
    assert_eq!(app.next_animation_in(), None);
}

#[test]
fn only_a_new_frame_dirties_the_screen() {
    let mut app = working_app();
    draw(&mut app);
    app.take_dirty();
    // Same 125 ms frame: nothing visible changed, so no redraw.
    let frame_start = (NOW_MS / 125) * 125;
    app.set_now(frame_start + 1);
    app.take_dirty();
    app.set_now(frame_start + 60);
    assert!(!app.is_dirty());
    app.set_now(frame_start + 125);
    assert!(app.is_dirty());
}

// --- long commands ---

fn closing_app(fresh: bool) -> dovetail_tui::App {
    let mut app = idle_app();
    app.on_server(ServerMsg::Event {
        seq: app_seq(&app),
        event: Event::JournalChanged(JournalState {
            latest: None,
            closing: true,
            fresh,
        }),
    });
    app
}

#[test]
fn eod_shows_the_closing_indicator_in_the_status_bar() {
    let mut app = closing_app(false);
    app.set_now(NOW_MS + 12_000);
    let screen = draw(&mut app);
    let bar = screen.lines().last().expect("a status bar");
    assert!(bar.contains("Closing the day… 12s"), "{bar}");
    assert!(app.next_animation_in().is_some());
    insta::assert_snapshot!(bar);
}

#[test]
fn fresh_and_upgrade_say_their_own_words() {
    let mut app = closing_app(true);
    assert!(draw(&mut app).contains("Starting fresh…"));
    let mut app = idle_app();
    app.on_server(ServerMsg::Event {
        seq: app_seq(&app),
        event: Event::UpgradeBuilding(true),
    });
    assert!(draw(&mut app).contains("Building the upgrade…"));
}

#[test]
fn when_the_day_is_closed_the_indicator_clears_and_a_notice_shows() {
    let mut app = closing_app(false);
    draw(&mut app);
    app.on_server(ServerMsg::Event {
        seq: app_seq(&app),
        event: Event::JournalChanged(JournalState {
            latest: Some("2026-11-01".into()),
            closing: false,
            fresh: false,
        }),
    });
    let screen = draw(&mut app);
    assert!(!screen.contains("Closing the day…"));
    assert_eq!(app.note(), Some("day closed"));
    assert_eq!(app.next_animation_in(), None);
}

// --- helpers on the app's copy of the state ---

fn app_seq(app: &dovetail_tui::App) -> u64 {
    app.snapshot().seq + 1
}

fn app_agent(app: &dovetail_tui::App, id: &str) -> Agent {
    app.snapshot()
        .agents
        .iter()
        .find(|a| a.id.0 == id)
        .cloned()
        .expect("agent")
}

#[test]
fn a_working_slice_row_in_the_tree_keeps_the_ui_animating() {
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    let slice = snap
        .agents
        .iter_mut()
        .find(|a| a.kind == AgentKind::Slice)
        .expect("a slice");
    slice.status = AgentStatus::Working;
    let mut app = app_with(snap);
    render(&mut app, 200, 40);
    assert!(app.animating());
    assert!(app.next_animation_in().is_some());
}

#[test]
fn a_subagent_showing_progress_does_not_keep_the_ui_animating() {
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    let sub = snap
        .agents
        .iter_mut()
        .find(|a| matches!(a.kind, AgentKind::Subagent { .. }) && a.progress.is_some())
        .expect("a subagent with progress");
    sub.status = AgentStatus::Working;
    let mut app = app_with(snap);
    draw(&mut app);
    assert!(!app.animating());
}

#[test]
fn a_pane_too_narrow_for_any_spinner_does_not_animate() {
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    snap.agents
        .iter_mut()
        .find(|a| a.kind == AgentKind::Slice)
        .expect("a slice")
        .status = AgentStatus::Working;
    let mut app = app_with(snap);
    render(&mut app, 40, 30);
    assert!(!app.animating());
}

//! The usage indicator in the header row (task #16).

mod support;

use dovetail_api::{Event, ServerMsg, UsageLimits, UsageWindow};
use support::*;

fn usage(five: f64, week: f64) -> UsageLimits {
    let now_s = NOW_MS / 1_000;
    UsageLimits {
        five_hour: Some(UsageWindow {
            utilization: five,
            resets_at: Some(now_s + 2 * 3_600 + 20 * 60),
            status: "allowed".into(),
        }),
        seven_day: Some(UsageWindow {
            utilization: week,
            resets_at: Some(now_s + 3 * 86_400),
            status: "allowed".into(),
        }),
    }
}

fn app_with_usage() -> dovetail_tui::App {
    let mut snap = world();
    snap.usage = usage(0.34, 0.18);
    for a in &mut snap.agents {
        if a.id == agent_id("coordinator") {
            a.model = Some("claude-opus-5-5".into());
        }
    }
    app_with(snap)
}

fn header(app: &mut dovetail_tui::App, width: u16) -> String {
    render(app, width, 12)
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

#[test]
fn the_header_drops_resets_then_model_then_week() {
    let mut app = app_with_usage();
    let rows: Vec<String> = [120, 80, 60, 44, 34, 24]
        .into_iter()
        .map(|w| format!("{w:>3} |{}", header(&mut app, w)))
        .collect();
    insta::assert_snapshot!(rows.join("\n"));
}

#[test]
fn with_no_turn_yet_only_the_model_shows() {
    let mut snap = world();
    for a in &mut snap.agents {
        a.model = Some("claude-opus-5-5".into());
    }
    let mut app = app_with(snap);
    let row = header(&mut app, 120);
    assert!(row.ends_with("opus-5-5"), "{row}");
    assert!(!row.contains("5h"), "{row}");
}

#[test]
fn a_reset_passing_redraws_once_and_shows_a_dash() {
    let mut app = app_with_usage();
    app.take_dirty();
    let reset_ms = NOW_MS + (2 * 3_600 + 20 * 60) * 1_000;
    assert_eq!(app.next_clock_change_ms(), Some(reset_ms));
    app.set_now(reset_ms - 1);
    assert!(!app.is_dirty(), "nothing changes before the reset");
    app.set_now(reset_ms);
    assert!(app.is_dirty(), "the reset passing redraws the header");
    let row = header(&mut app, 120);
    assert!(row.contains("5h —"), "{row}");
    assert!(!row.contains("5h 34%"), "{row}");

    // The next report, for the new window, replaces the dash.
    let mut next = usage(0.02, 0.18);
    if let Some(w) = next.five_hour.as_mut() {
        w.resets_at = Some(reset_ms / 1_000 + 5 * 3_600);
    }
    app.on_server(ServerMsg::Event {
        seq: 1,
        event: Event::UsageChanged(next),
    });
    let row = header(&mut app, 120);
    assert!(row.contains("5h 2%"), "{row}");
}

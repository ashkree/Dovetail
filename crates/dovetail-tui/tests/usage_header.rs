//! The usage line at the bottom right of the status bar (task #16, moved in task #40).

mod support;

use dovetail_api::{Event, ServerMsg, UsageLimits, UsageStatus, UsageWindow};
use support::*;

fn usage(five: f64, week: f64) -> UsageLimits {
    UsageLimits {
        five_hour: Some(UsageWindow {
            utilization: five,
            resets_at_ms: Some(NOW_MS + (2 * 3_600 + 20 * 60) * 1_000),
            status: UsageStatus::Allowed,
        }),
        seven_day: Some(UsageWindow {
            utilization: week,
            resets_at_ms: Some(NOW_MS + 3 * 86_400_000),
            status: UsageStatus::Allowed,
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

/// The status row, the last on screen.
fn header(app: &mut dovetail_tui::App, width: u16) -> String {
    render(app, width, 12)
        .lines()
        .last()
        .unwrap_or("")
        .trim_end()
        .to_string()
}

#[test]
fn the_usage_line_drops_resets_then_model_then_week() {
    let mut app = app_with_usage();
    let rows: Vec<String> = [120, 80, 60, 44, 34, 24]
        .into_iter()
        .map(|w| format!("{w:>3} |{}", header(&mut app, w)))
        .collect();
    insta::assert_snapshot!(rows.join("\n"));
}

#[test]
fn with_no_turn_yet_only_the_model_and_context_show() {
    let mut snap = world();
    for a in &mut snap.agents {
        a.model = Some("claude-opus-5-5".into());
    }
    let mut app = app_with(snap);
    let row = header(&mut app, 120);
    assert!(row.ends_with("opus-5-5 · ctx 62%"), "{row}");
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
    assert!(app.is_dirty(), "the reset passing redraws the usage line");
    let row = header(&mut app, 120);
    assert!(row.contains("5h —"), "{row}");
    assert!(!row.contains("5h 34%"), "{row}");

    // The next report, for the new window, replaces the dash.
    let mut next = usage(0.02, 0.18);
    if let Some(w) = next.five_hour.as_mut() {
        w.resets_at_ms = Some(reset_ms + 5 * 3_600_000);
    }
    app.on_server(ServerMsg::Event {
        seq: 1,
        event: Event::UsageChanged(next),
    });
    let row = header(&mut app, 120);
    assert!(row.contains("5h 2%"), "{row}");
}

/// Task #26: Claude Code 2.1.319 can report only the weekly window. The usage the core makes of
/// that line (dovetail-core `a_rate_limit_event_with_only_the_weekly_window_reaches_the_ui_as_such`)
/// shows the 5-hour window as a dash, never as nothing, and the dash is the last part to drop.
#[test]
fn a_report_without_the_5h_window_shows_a_dash() {
    let mut snap = world();
    snap.usage = UsageLimits {
        five_hour: None,
        seven_day: Some(UsageWindow {
            utilization: 0.06,
            resets_at_ms: Some(NOW_MS + 3 * 86_400_000),
            status: UsageStatus::Allowed,
        }),
    };
    let mut app = app_with(snap);
    let row = header(&mut app, 120);
    assert!(row.contains("5h — · week 6% (resets Wed)"), "{row}");
    let narrow = header(&mut app, 44);
    assert!(narrow.contains("5h —"), "{narrow}");
}

/// Task #40: the not-a-project warning sits in the status bar, only while it applies, and falls
/// back to its short text before the usage line gives way.
#[test]
fn the_not_a_project_warning_shows_in_the_status_bar_only_while_it_applies() {
    let mut snap = world();
    snap.usage = usage(0.34, 0.18);
    let mut app = app_with(snap.clone());
    assert!(
        !header(&mut app, 160).contains("not a"),
        "no warning, none shown"
    );

    snap.warnings.push(dovetail_api::Warning {
        id: dovetail_api::WarningId(dovetail_api::UNREGISTERED_WARNING.into()),
        text: "not a Dovetail project".into(),
    });
    let mut app = app_with(snap);
    let wide = header(&mut app, 200);
    assert!(
        wide.contains("⚠ not a Dovetail project yet: :project add keeps it"),
        "{wide}"
    );
    assert!(wide.contains("5h 34%"), "{wide}");
    let narrow = header(&mut app, 60);
    assert!(narrow.contains("⚠ not a project yet"), "{narrow}");
    assert!(!narrow.contains("Dovetail project yet"), "{narrow}");
}

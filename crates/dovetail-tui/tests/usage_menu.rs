//! Task #46 (C5): the usage limits menu. `/usage` in the chat bar and `:usage` open it, send
//! `Command::LoadUsage`, show loading until `Event::UsageLoaded` arrives, render Claude Code and
//! agy groups with bars, percentages, reset times, models, plan and note, show failed reports'
//! reasons, refresh on `Tab`, and close on `Esc`.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{Command, DriverKind, DriverUsage, Event, LimitWindow, ServerMsg, UsageGroup};
use dovetail_tui::{App, Mode};
use support::*;

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn down() -> KeyEvent {
    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
}

fn deliver_usage(app: &mut App, reports: Vec<DriverUsage>) {
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::UsageLoaded(reports),
    });
}

fn sample_reports() -> Vec<DriverUsage> {
    vec![
        DriverUsage {
            driver: DriverKind::Claude,
            plan: Some("pro".into()),
            groups: vec![UsageGroup {
                name: "Claude Code".into(),
                models: String::new(),
                windows: vec![
                    LimitWindow {
                        name: "5-hour".into(),
                        used_fraction: 0.43,
                        resets_at_ms: Some(NOW_MS + 2 * 3_600_000),
                    },
                    LimitWindow {
                        name: "Weekly".into(),
                        used_fraction: 0.91,
                        resets_at_ms: Some(NOW_MS + 3 * 86_400_000),
                    },
                ],
            }],
            note: Some("extra usage off (out of credits)".into()),
            failed: false,
        },
        DriverUsage {
            driver: DriverKind::Agy,
            plan: None,
            groups: vec![UsageGroup {
                name: "Gemini Models".into(),
                models: "Gemini 4 Neon, Gemini 3.8 Flash".into(),
                windows: vec![LimitWindow {
                    name: "5-hour".into(),
                    used_fraction: 0.72,
                    resets_at_ms: Some(NOW_MS + 3_600_000),
                }],
            }],
            note: None,
            failed: false,
        },
    ]
}

#[test]
fn slash_usage_opens_menu_sends_load_usage_and_shows_loading() {
    let mut app = app_with(world());
    press(&mut app, "i");
    type_keys(&mut app, "/");
    app.on_key(esc(), 0);
    type_keys(&mut app, "usage");
    app.on_key(enter(), 100);
    assert_eq!(app.take_commands(), vec![Command::LoadUsage]);
    let screen = render(&mut app, 130, 32);
    assert!(screen.contains("Usage"), "{screen}");
    assert!(screen.contains("loading usage limits…"), "{screen}");
}

#[test]
fn slash_usage_with_trailing_text_is_refused() {
    let mut app = app_with(world());
    press(&mut app, "i");
    type_keys(&mut app, "/");
    app.on_key(esc(), 0);
    type_keys(&mut app, "usage extra");
    app.on_key(enter(), 100);
    assert!(app.take_commands().is_empty());
    assert!(!render(&mut app, 130, 32).contains("loading usage limits…"));
    assert_eq!(
        app.note(),
        Some("/usage takes nothing after it; nothing was sent")
    );
}

#[test]
fn colon_usage_populates_claude_and_agy_groups_and_refreshes_on_tab() {
    let mut app = app_with(world());
    type_keys(&mut app, ":usage");
    app.on_key(enter(), 100);
    assert_eq!(app.take_commands(), vec![Command::LoadUsage]);
    assert!(render(&mut app, 130, 32).contains("loading usage limits…"));

    deliver_usage(&mut app, sample_reports());
    let screen = render(&mut app, 130, 32);
    assert!(screen.contains("Claude Code"), "{screen}");
    assert!(screen.contains("agy"), "{screen}");
    assert!(screen.contains("Gemini Models"), "{screen}");
    assert!(screen.contains("91%"), "{screen}");
    assert!(screen.contains("72%"), "{screen}");
    assert!(screen.contains("plan: pro"), "{screen}");
    assert!(screen.contains("5-hour"), "{screen}");
    assert!(screen.contains("43%"), "{screen}");
    assert!(screen.contains("resets 12:15"), "{screen}");
    assert!(screen.contains("resets Wed"), "{screen}");
    assert!(
        screen.contains("extra usage off (out of credits)"),
        "{screen}"
    );

    // Move to agy's group and inspect models in preview.
    app.on_key(down(), 150);
    let agy_screen = render(&mut app, 130, 32);
    assert!(
        agy_screen.contains("Gemini 4 Neon, Gemini 3.8 Flash"),
        "{agy_screen}"
    );
    assert!(agy_screen.contains("resets 11:15"), "{agy_screen}");

    // Tab triggers a reload.
    app.on_key(tab(), 200);
    assert_eq!(app.take_commands(), vec![Command::LoadUsage]);

    // Esc closes cleanly to Normal mode.
    app.on_key(esc(), 210);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(!render(&mut app, 130, 32).contains("tab refresh · esc close"));
}

#[test]
fn failed_report_displays_single_row_and_failure_reason_in_preview() {
    let mut app = app_with(world());
    type_keys(&mut app, ":usage");
    app.on_key(enter(), 100);
    app.take_commands();

    deliver_usage(
        &mut app,
        vec![DriverUsage::failed(
            DriverKind::Agy,
            "agy binary not found in PATH",
        )],
    );
    let screen = render(&mut app, 130, 32);
    assert!(screen.contains("unavailable"), "{screen}");
    assert!(screen.contains("failed"), "{screen}");
    assert!(screen.contains("agy binary not found in PATH"), "{screen}");
}

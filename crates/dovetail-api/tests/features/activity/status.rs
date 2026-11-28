//! Tests for the `status` feature.

use crate::common::{Case, roundtrip, run, totals, warning};
use dovetail_api::model::DriverKind;
use dovetail_api::status::{
    DriverUsage, LimitWindow, StatusCommand, StatusEvent, Totals, UsageGroup, Warning, WarningId,
};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "usage_loaded",
            Snapshot::default(),
            vec![(1, Event::Status(StatusEvent::UsageLoaded(driver_usage())))],
        ),
        Case::new(
            "status",
            Snapshot::default(),
            vec![
                (1, Event::Status(StatusEvent::TotalsChanged(totals()))),
                (2, Event::Status(StatusEvent::WarningSet(warning()))),
                (
                    3,
                    Event::Status(StatusEvent::WarningCleared(WarningId("mode".into()))),
                ),
            ],
        ),
    ]
}

#[test]
fn status_events_roundtrip() {
    for (n, event) in [
        Event::Status(StatusEvent::TotalsChanged(totals())),
        Event::Status(StatusEvent::WarningSet(warning())),
        Event::Status(StatusEvent::WarningCleared(WarningId("mode".into()))),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }
}

#[test]
fn totals_changed_replaces_totals() {
    let totals = Totals {
        memory_mb: 640,
        needs_you: 3,
        cost_usd: 1.5,
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![Event::Status(StatusEvent::TotalsChanged(totals.clone()))],
    );
    assert_eq!(state.totals, totals);
}

#[test]
fn warnings_are_set_by_id_and_cleared() {
    let warning = |id: &str, text: &str| Warning {
        id: WarningId(id.into()),
        text: text.into(),
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Status(StatusEvent::WarningSet(warning(
                "mode",
                "coordinator is not in auto mode",
            ))),
            Event::Status(StatusEvent::WarningSet(warning(
                "cgroup",
                "memory caps unavailable",
            ))),
            Event::Status(StatusEvent::WarningSet(warning(
                "mode",
                "slice-a is not in auto mode",
            ))),
            Event::Status(StatusEvent::WarningCleared(WarningId("cgroup".into()))),
        ],
    );
    assert_eq!(
        state.warnings,
        vec![warning("mode", "slice-a is not in auto mode")]
    );
}

fn driver_usage() -> Vec<DriverUsage> {
    vec![
        DriverUsage {
            driver: DriverKind::Claude,
            plan: Some("max".into()),
            groups: vec![UsageGroup {
                name: "Claude Code".into(),
                models: String::new(),
                windows: vec![LimitWindow {
                    name: "5-hour".into(),
                    used_fraction: 0.34,
                    resets_at_ms: Some(1_700_008_400_000),
                }],
            }],
            note: Some("extra usage off".into()),
            failed: false,
        },
        DriverUsage::failed(DriverKind::Agy, "agy binary not found"),
    ]
}

#[test]
fn usage_loaded_and_load_usage_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Status(StatusEvent::UsageLoaded(driver_usage())),
    });
    roundtrip(&Command::Status(StatusCommand::LoadUsage));
}

#[test]
fn usage_loaded_updates_agy_usage_and_ignores_claude_only() {
    let mut state = Snapshot {
        seq: 4,
        ..Snapshot::default()
    };
    let expected = Snapshot {
        seq: 5,
        ..state.clone()
    };
    run(
        &mut state,
        vec![Event::Status(StatusEvent::UsageLoaded(vec![
            driver_usage().remove(0),
        ]))],
    );
    assert_eq!(state, expected);

    let agy_report = DriverUsage {
        driver: DriverKind::Agy,
        plan: None,
        groups: vec![UsageGroup {
            name: "Gemini Models".into(),
            models: "Gemini 4 Neon".into(),
            windows: vec![LimitWindow {
                name: "5-hour".into(),
                used_fraction: 0.1,
                resets_at_ms: Some(1_700_000_000_000),
            }],
        }],
        note: None,
        failed: false,
    };
    let mut expected_agy = state.clone();
    expected_agy.seq = 6;
    expected_agy.agy_usage = Some(agy_report.clone());
    run(
        &mut state,
        vec![Event::Status(StatusEvent::UsageLoaded(vec![agy_report]))],
    );
    assert_eq!(state, expected_agy);
}

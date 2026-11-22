//! Tests for the `status` feature.

use crate::common::{Case, roundtrip, run, totals, warning};
use dovetail_api::status::{StatusEvent, Totals, Warning, WarningId};
use dovetail_api::{Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![Case::new(
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
    )]
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

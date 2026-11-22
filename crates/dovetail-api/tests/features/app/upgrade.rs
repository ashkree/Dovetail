//! Tests for the `upgrade` feature.

use crate::common::{Case, roundtrip, run};
use dovetail_api::app::{AppCommand, AppEvent};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "upgrade",
            Snapshot::default(),
            vec![(1, Event::App(AppEvent::UpgradeBuilding(true)))],
        ),
        Case::no_op(
            "upgrade_restarting",
            Snapshot::default(),
            vec![(1, Event::App(AppEvent::UpgradeRestarting))],
        ),
    ]
}

#[test]
fn upgrade_commands_and_events_roundtrip() {
    for (n, event) in [
        Event::App(AppEvent::UpgradeBuilding(true)),
        Event::App(AppEvent::UpgradeRestarting),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    roundtrip(&Command::App(AppCommand::StartUpgrade { local: true }));
}

#[test]
fn upgrade_restarting_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(&mut state, vec![Event::App(AppEvent::UpgradeRestarting)]);
    assert_eq!(state, expected);
}

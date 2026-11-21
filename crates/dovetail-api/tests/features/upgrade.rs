//! Tests for the `upgrade` feature.

use crate::common::{Case, roundtrip, run};
use dovetail_api::upgrade::{UpgradeCommand, UpgradeEvent};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "upgrade",
            Snapshot::default(),
            vec![(1, Event::Upgrade(UpgradeEvent::Building(true)))],
        ),
        Case::no_op(
            "upgrade_restarting",
            Snapshot::default(),
            vec![(1, Event::Upgrade(UpgradeEvent::Restarting))],
        ),
    ]
}

#[test]
fn upgrade_commands_and_events_roundtrip() {
    for (n, event) in [
        Event::Upgrade(UpgradeEvent::Building(true)),
        Event::Upgrade(UpgradeEvent::Restarting),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    roundtrip(&Command::Upgrade(UpgradeCommand::Start { local: true }));
}

#[test]
fn upgrade_restarting_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(&mut state, vec![Event::Upgrade(UpgradeEvent::Restarting)]);
    assert_eq!(state, expected);
}

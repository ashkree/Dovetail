//! Tests for the `journal` feature.

use crate::common::{Case, roundtrip};
use dovetail_api::journal::{JournalCommand, JournalEvent, JournalState};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![Case::new(
        "journal",
        Snapshot::default(),
        vec![(
            1,
            Event::Journal(JournalEvent::Changed(JournalState {
                latest: Some("2026-11-16".into()),
                closing: false,
                fresh: true,
            })),
        )],
    )]
}

#[test]
fn journal_commands_and_events_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Journal(JournalEvent::Changed(JournalState {
            latest: None,
            closing: true,
            fresh: true,
        })),
    });

    for command in [
        Command::Journal(JournalCommand::EndOfDay),
        Command::Journal(JournalCommand::Handoff),
        Command::Journal(JournalCommand::FreshSession),
    ] {
        roundtrip(&command);
    }
}

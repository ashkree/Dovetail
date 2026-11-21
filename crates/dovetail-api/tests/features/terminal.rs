//! Tests for the `terminal` feature.

use crate::common::{Case, id, roundtrip, run};
use dovetail_api::terminal::{TerminalCommand, TerminalEvent};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn terminal_events() -> Vec<Event> {
    vec![
        Event::Terminal(TerminalEvent::Lend),
        Event::Terminal(TerminalEvent::Returned { error: None }),
        Event::Terminal(TerminalEvent::Returned {
            error: Some("nvim exited with status 1".into()),
        }),
    ]
}

pub fn cases() -> Vec<Case> {
    vec![Case::no_op(
        "terminal",
        Snapshot::default(),
        terminal_events()
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    )]
}

#[test]
fn terminal_commands_and_events_roundtrip() {
    for (n, event) in terminal_events().into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    for command in [
        Command::Terminal(TerminalCommand::OpenEditor {
            path: "/tmp/wt".into(),
        }),
        Command::Terminal(TerminalCommand::Released),
        Command::Terminal(TerminalCommand::TakeOver {
            agent: id("slice-a"),
        }),
    ] {
        roundtrip(&command);
    }
}

#[test]
fn terminal_events_only_advance_seq() {
    let mut state = Snapshot::default();
    let events = terminal_events();
    let mut expected = state.clone();
    expected.seq = events.len() as u64;
    run(&mut state, events);
    assert_eq!(state, expected);
}

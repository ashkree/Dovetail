//! Tests for the `app` feature, including its terminal, clipboard and upgrade parts.

use crate::common::{Case, roundtrip, run};
use dovetail_api::app::{AppCommand, AppEvent};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

mod clipboard;
mod terminal;
mod upgrade;

pub fn cases() -> Vec<Case> {
    let mut cases = own_cases();
    cases.extend(clipboard::cases());
    cases.extend(terminal::cases());
    cases.extend(upgrade::cases());
    cases
}

fn own_cases() -> Vec<Case> {
    vec![Case::no_op(
        "app",
        Snapshot::default(),
        vec![(
            1,
            Event::App(AppEvent::CommandFailed {
                message: "no such agent".into(),
            }),
        )],
    )]
}

#[test]
fn app_commands_and_events_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::App(AppEvent::CommandFailed {
            message: "no such agent".into(),
        }),
    });
    for command in [
        Command::App(AppCommand::OpenExternal {
            url: "https://docs.rs/nucleo".into(),
        }),
        Command::App(AppCommand::Resync),
        Command::App(AppCommand::Quit {
            end_sessions: false,
        }),
    ] {
        roundtrip(&command);
    }
}

#[test]
fn command_failed_only_advances_seq() {
    let mut state = Snapshot {
        seq: 4,
        ..Snapshot::default()
    };
    let mut expected = state.clone();
    expected.seq = 5;
    run(
        &mut state,
        vec![Event::App(AppEvent::CommandFailed {
            message: "no such agent".into(),
        })],
    );
    assert_eq!(state, expected);
}

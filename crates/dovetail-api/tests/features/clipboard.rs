//! Tests for the `clipboard` feature.

use crate::common::{Case, id, roundtrip, run};
use dovetail_api::clipboard::{ClipboardCommand, ClipboardEvent, Paste};
use dovetail_api::conversation::{Attachment, ImageType};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![Case::no_op(
        "clipboard",
        Snapshot::default(),
        vec![(
            1,
            Event::Clipboard(ClipboardEvent::Pasted {
                agent: id("coordinator"),
                paste: Paste::Text("hello".into()),
            }),
        )],
    )]
}

#[test]
fn clipboard_commands_and_events_roundtrip() {
    for paste in [
        Paste::Image(Attachment {
            path: "/data/dovetail/attachments/1.png".into(),
            media_type: ImageType::Png,
            size: 1024,
        }),
        Paste::Text("pasted".into()),
        Paste::Refused("too large".into()),
    ] {
        roundtrip(&ServerMsg::Event {
            seq: 0,
            event: Event::Clipboard(ClipboardEvent::Pasted {
                agent: id("coordinator"),
                paste,
            }),
        });
    }

    for command in [
        Command::Clipboard(ClipboardCommand::Paste {
            agent: id("coordinator"),
        }),
        Command::Clipboard(ClipboardCommand::PasteText {
            agent: id("coordinator"),
            text: "hi".into(),
        }),
        Command::Clipboard(ClipboardCommand::Copy {
            text: "/tmp/wt".into(),
        }),
    ] {
        roundtrip(&command);
    }
}

#[test]
fn clipboard_pasted_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(
        &mut state,
        vec![Event::Clipboard(ClipboardEvent::Pasted {
            agent: id("coordinator"),
            paste: Paste::Text("hello".into()),
        })],
    );
    assert_eq!(state, expected);
}

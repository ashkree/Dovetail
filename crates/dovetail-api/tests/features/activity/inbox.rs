//! Tests for the `inbox` feature.

use crate::common::{Case, inbox_items, question, roundtrip, run};
use dovetail_api::inbox::{InboxCommand, InboxEvent, InboxId, InboxKind, PermissionAnswer};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};
use serde_json::json;

pub fn cases() -> Vec<Case> {
    vec![Case::new(
        "inbox",
        Snapshot::default(),
        vec![
            (1, Event::Inbox(InboxEvent::Upserted(question(1, "s1")))),
            (2, Event::Inbox(InboxEvent::Resolved { id: InboxId(1) })),
        ],
    )]
}

#[test]
fn inbox_commands_and_events_roundtrip() {
    for (n, item) in inbox_items().into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event: Event::Inbox(InboxEvent::Upserted(item)),
        });
    }
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Inbox(InboxEvent::Resolved { id: InboxId(1) }),
    });

    let commands = vec![
        Command::Inbox(InboxCommand::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::Allow,
        }),
        Command::Inbox(InboxCommand::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::AllowAlways,
        }),
        Command::Inbox(InboxCommand::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::Deny {
                message: Some("use the test target".into()),
            },
        }),
        Command::Inbox(InboxCommand::AnswerQuestion {
            item: InboxId(2),
            text: "8080".into(),
        }),
        Command::Inbox(InboxCommand::Dismiss { item: InboxId(3) }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn an_old_blocking_question_payload_still_parses() {
    let kind: InboxKind =
        serde_json::from_value(json!({"question": {"text": "Which port?", "blocking": true}}))
            .expect("old payload parses");
    assert_eq!(
        kind,
        InboxKind::Question {
            text: "Which port?".into()
        }
    );
}

#[test]
fn inbox_upserted_inserts_then_replaces_by_id() {
    let mut changed = question(1, "s1");
    changed.kind = InboxKind::Error {
        text: "crashed".into(),
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Inbox(InboxEvent::Upserted(question(1, "s1"))),
            Event::Inbox(InboxEvent::Upserted(question(2, "s2"))),
            Event::Inbox(InboxEvent::Upserted(changed.clone())),
        ],
    );
    assert_eq!(state.inbox, vec![changed, question(2, "s2")]);
}

#[test]
fn inbox_resolved_removes_the_item() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Inbox(InboxEvent::Upserted(question(1, "s1"))),
            Event::Inbox(InboxEvent::Upserted(question(2, "s1"))),
            Event::Inbox(InboxEvent::Resolved { id: InboxId(1) }),
        ],
    );
    assert_eq!(state.inbox, vec![question(2, "s1")]);
}

//! Tests for the `conversation` feature.

use crate::common::{Case, entry, id, rich_entry, roundtrip, run};
use dovetail_api::agent::AgentId;
use dovetail_api::conversation::{
    Attachment, ConversationCommand, ConversationEntry, ConversationEvent, ConversationTail,
    EntryId, EntryKind, ImageType, agent_message_text,
};
use dovetail_api::effort::EffortId;
use dovetail_api::snapshot::{CONVERSATION_KEEP, CONVERSATION_TAIL};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};
use serde_json::json;

fn coordinator() -> AgentId {
    AgentId("coordinator".into())
}

fn tagged_entry(id: u64, kind: EntryKind, text: &str, effort: Option<i64>) -> ConversationEntry {
    ConversationEntry {
        id: EntryId(id),
        at_ms: 1_000,
        kind,
        text: text.into(),
        attachments: vec![],
        effort: effort.map(EffortId),
    }
}

pub fn cases() -> Vec<Case> {
    let mut with_tail = Snapshot {
        seq: 10,
        now_ms: 5_000,
        ..Snapshot::default()
    };
    with_tail.conversations.insert(
        coordinator(),
        ConversationTail {
            entries: vec![tagged_entry(5, EntryKind::User, "hello", Some(2))],
            has_more: true,
        },
    );

    vec![
        Case::new(
            "conversation",
            with_tail,
            vec![
                (
                    11,
                    Event::Conversation(ConversationEvent::Appended {
                        agent: coordinator(),
                        entry: tagged_entry(6, EntryKind::Assistant, "", Some(2)),
                    }),
                ),
                (
                    12,
                    Event::Conversation(ConversationEvent::AssistantDelta {
                        agent: coordinator(),
                        entry: EntryId(6),
                        text: "Will ".into(),
                    }),
                ),
                (
                    13,
                    Event::Conversation(ConversationEvent::AssistantDelta {
                        agent: coordinator(),
                        entry: EntryId(6),
                        text: "do.".into(),
                    }),
                ),
                // A client that joined mid-stream starts the entry at snapshot time.
                (
                    14,
                    Event::Conversation(ConversationEvent::AssistantDelta {
                        agent: coordinator(),
                        entry: EntryId(9),
                        text: "late".into(),
                    }),
                ),
                (
                    15,
                    Event::Conversation(ConversationEvent::Page {
                        agent: coordinator(),
                        entries: vec![
                            tagged_entry(2, EntryKind::User, "older", None),
                            tagged_entry(5, EntryKind::User, "hello", Some(2)),
                        ],
                        has_more: false,
                    }),
                ),
                (
                    16,
                    Event::Conversation(ConversationEvent::Appended {
                        agent: coordinator(),
                        entry: tagged_entry(6, EntryKind::Assistant, "Will do, now.", Some(2)),
                    }),
                ),
            ],
        ),
        Case::no_op(
            "conversation_paths_listed",
            Snapshot::default(),
            vec![(
                1,
                Event::Conversation(ConversationEvent::PathsListed {
                    agent: coordinator(),
                    dir: None,
                    paths: vec!["src/lib.rs".into()],
                }),
            )],
        ),
    ]
}

#[test]
fn conversation_commands_and_events_roundtrip() {
    let events = vec![
        Event::Conversation(ConversationEvent::Appended {
            agent: id("slice-a"),
            entry: rich_entry(),
        }),
        Event::Conversation(ConversationEvent::AssistantDelta {
            agent: id("slice-a"),
            entry: EntryId(9),
            text: "more *text*\n".into(),
        }),
        Event::Conversation(ConversationEvent::Page {
            agent: id("slice-a"),
            entries: vec![rich_entry()],
            has_more: false,
        }),
        Event::Conversation(ConversationEvent::PathsListed {
            agent: id("slice-a"),
            dir: Some("src/".into()),
            paths: vec!["lib.rs".into()],
        }),
    ];
    for (n, event) in events.into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Conversation(ConversationCommand::Prompt {
            agent: id("coordinator"),
            text: "plan M2".into(),
            attachments: Vec::new(),
            effort: None,
        }),
        Command::Conversation(ConversationCommand::Prompt {
            agent: id("coordinator"),
            text: String::new(),
            attachments: vec![Attachment {
                path: "/data/dovetail/attachments/2.webp".into(),
                media_type: ImageType::Webp,
                size: 10,
            }],
            effort: Some(EffortId(7)),
        }),
        Command::Conversation(ConversationCommand::Load {
            agent: id("slice-a"),
            before: Some(EntryId(9)),
        }),
        Command::Conversation(ConversationCommand::Load {
            agent: id("slice-a"),
            before: None,
        }),
        Command::Conversation(ConversationCommand::ListPaths {
            agent: id("slice-a"),
            dir: None,
        }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn an_agent_message_entry_roundtrips_and_old_kinds_still_read() {
    let entry = ConversationEntry {
        id: EntryId(4),
        at_ms: 9,
        kind: EntryKind::AgentMessage,
        text: agent_message_text("scout", true, "report\nmore"),
        attachments: Vec::new(),
        effort: None,
    };
    roundtrip(&entry);
    assert_eq!(
        serde_json::to_value(&entry).unwrap()["kind"],
        "agent_message"
    );
    let parts = entry.agent_message().unwrap();
    assert_eq!(
        (parts.from, parts.subagent, parts.body),
        ("scout", true, "report\nmore")
    );
    // An entry stored before the kind existed.
    let old: ConversationEntry = serde_json::from_value(
        json!({"id": 1, "at_ms": 0, "kind": "notice", "text": "Message from x: y"}),
    )
    .unwrap();
    assert_eq!(old.kind, EntryKind::Notice);
    assert_eq!(old.agent_message(), None);
}

#[test]
fn conversation_appended_creates_the_tail_then_appends() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(1, EntryKind::User, "hi"),
            }),
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(2, EntryKind::Assistant, "hello"),
            }),
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")],
        ConversationTail {
            entries: vec![
                entry(1, EntryKind::User, "hi"),
                entry(2, EntryKind::Assistant, "hello")
            ],
            has_more: false,
        }
    );
}

#[test]
fn conversation_appended_with_a_known_id_replaces_the_entry() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(1, EntryKind::Assistant, "draft"),
            }),
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(1, EntryKind::Assistant, "final text"),
            }),
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")].entries,
        vec![entry(1, EntryKind::Assistant, "final text")]
    );
}

#[test]
fn conversation_keeps_the_newest_2000_and_marks_more() {
    let mut state = Snapshot::default();
    let n = CONVERSATION_KEEP as u64 + 1;
    let events = (1..=n)
        .map(|n| {
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(n, EntryKind::User, "x"),
            })
        })
        .collect();
    run(&mut state, events);
    let tail = &state.conversations[&id("s1")];
    assert_eq!(tail.entries.len(), CONVERSATION_KEEP);
    assert_eq!(tail.entries.first().map(|e| e.id), Some(EntryId(2)));
    assert!(tail.has_more);
}

#[test]
fn paged_history_survives_the_next_message() {
    // Plan section 7, decided 2026-11-03: the 200-entry tail is for snapshots; what a client
    // pages in stays (up to CONVERSATION_KEEP) when new entries arrive.
    let mut state = Snapshot::default();
    let tail: Vec<Event> = (300..500)
        .map(|n| {
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(n, EntryKind::User, "x"),
            })
        })
        .collect();
    run(&mut state, tail);
    let older = (100..300)
        .map(|n| entry(n, EntryKind::User, "old"))
        .collect();
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Page {
                agent: id("s1"),
                entries: older,
                has_more: false,
            }),
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(500, EntryKind::User, "new"),
            }),
        ],
    );
    let tail = &state.conversations[&id("s1")];
    assert_eq!(tail.entries.len(), 401);
    assert_eq!(tail.entries.first().map(|e| e.id), Some(EntryId(100)));
}

#[test]
fn a_trimmed_snapshot_carries_only_the_tail() {
    let mut state = Snapshot::default();
    let events = (1..=250)
        .map(|n| {
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(n, EntryKind::User, "x"),
            })
        })
        .collect();
    run(&mut state, events);
    let sent = state.trimmed();
    let tail = &sent.conversations[&id("s1")];
    assert_eq!(tail.entries.len(), CONVERSATION_TAIL);
    assert_eq!(tail.entries.first().map(|e| e.id), Some(EntryId(51)));
    assert!(tail.has_more);
    assert_eq!(
        state.conversations[&id("s1")].entries.len(),
        250,
        "the original is untouched"
    );
}

#[test]
fn assistant_delta_extends_its_entry() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(1, EntryKind::Assistant, ""),
            }),
            Event::Conversation(ConversationEvent::AssistantDelta {
                agent: id("s1"),
                entry: EntryId(1),
                text: "Hel".into(),
            }),
            Event::Conversation(ConversationEvent::AssistantDelta {
                agent: id("s1"),
                entry: EntryId(1),
                text: "lo **there**".into(),
            }),
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")].entries,
        vec![entry(1, EntryKind::Assistant, "Hello **there**")]
    );
}

#[test]
fn assistant_delta_for_an_unknown_entry_starts_one_at_snapshot_time() {
    let mut state = Snapshot {
        now_ms: 5_000,
        ..Snapshot::default()
    };
    run(
        &mut state,
        vec![Event::Conversation(ConversationEvent::AssistantDelta {
            agent: id("s1"),
            entry: EntryId(4),
            text: "Hi".into(),
        })],
    );
    assert_eq!(
        state.conversations[&id("s1")].entries,
        vec![ConversationEntry {
            id: EntryId(4),
            at_ms: 5_000,
            kind: EntryKind::Assistant,
            text: "Hi".into(),
            attachments: Vec::new(),
            effort: None,
        }]
    );
}

#[test]
fn conversation_page_puts_older_entries_before_the_tail() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(3, EntryKind::User, "new"),
            }),
            Event::Conversation(ConversationEvent::Page {
                agent: id("s1"),
                entries: vec![
                    entry(1, EntryKind::User, "old"),
                    entry(2, EntryKind::Assistant, "older reply"),
                ],
                has_more: false,
            }),
        ],
    );
    let ids: Vec<_> = state.conversations[&id("s1")]
        .entries
        .iter()
        .map(|e| e.id.0)
        .collect();
    assert_eq!(ids, [1, 2, 3]);
    assert!(!state.conversations[&id("s1")].has_more);
}

#[test]
fn conversation_page_skips_entries_it_already_has() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(2, EntryKind::User, "kept"),
            }),
            Event::Conversation(ConversationEvent::Page {
                agent: id("s1"),
                entries: vec![
                    entry(1, EntryKind::User, "old"),
                    entry(2, EntryKind::User, "duplicate"),
                ],
                has_more: true,
            }),
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")],
        ConversationTail {
            entries: vec![
                entry(1, EntryKind::User, "old"),
                entry(2, EntryKind::User, "kept")
            ],
            has_more: true,
        }
    );
}

#[test]
fn a_tagged_entry_keeps_its_tag_through_append_and_streaming() {
    let mut state = Snapshot::default();
    let mut you = entry(1, EntryKind::User, "Add Paddle");
    you.effort = Some(EffortId(2));
    let mut reply = entry(2, EntryKind::Assistant, "");
    reply.effort = Some(EffortId(2));
    run(
        &mut state,
        vec![
            Event::Conversation(ConversationEvent::Appended {
                agent: id("coordinator"),
                entry: you,
            }),
            Event::Conversation(ConversationEvent::Appended {
                agent: id("coordinator"),
                entry: reply,
            }),
            Event::Conversation(ConversationEvent::AssistantDelta {
                agent: id("coordinator"),
                entry: EntryId(2),
                text: "Added.".into(),
            }),
        ],
    );
    let entries = &state.conversations[&id("coordinator")].entries;
    assert_eq!(entries[0].effort, Some(EffortId(2)));
    assert_eq!(entries[1].effort, Some(EffortId(2)));
    assert_eq!(entries[1].text, "Added.");
}

#[test]
fn paths_listed_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(
        &mut state,
        vec![Event::Conversation(ConversationEvent::PathsListed {
            agent: coordinator(),
            dir: None,
            paths: vec!["src/lib.rs".into()],
        })],
    );
    assert_eq!(state, expected);
}

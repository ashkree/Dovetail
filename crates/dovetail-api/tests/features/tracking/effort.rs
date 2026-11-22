//! Tests for the `effort` feature.

use crate::common::{Case, effort, rich_effort, rich_task, roundtrip, run};
use dovetail_api::conversation::{ConversationCommand, ConversationEntry};
use dovetail_api::effort::{
    Criterion, CriterionId, CriterionState, EffortChange, EffortCommand, EffortEvent, EffortId,
    EffortKind, EffortStatus, NewEffort, UNSORTED_EFFORT,
};
use dovetail_api::task::Task;
use dovetail_api::{Command, Event, ServerMsg, Snapshot};
use serde_json::json;

pub fn cases() -> Vec<Case> {
    vec![Case::new(
        "effort",
        Snapshot::default(),
        vec![(1, Event::Effort(EffortEvent::Upserted(rich_effort())))],
    )]
}

#[test]
fn effort_commands_and_events_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Effort(EffortEvent::Upserted(rich_effort())),
    });

    let commands = vec![
        Command::Effort(EffortCommand::Create(NewEffort {
            name: "Guest Checkout".into(),
            kind: EffortKind::Feature,
            question_or_goal: "A buyer can pay without an account".into(),
        })),
        Command::Effort(EffortCommand::Rename {
            id: EffortId(7),
            name: "providers".into(),
        }),
        Command::Effort(EffortCommand::Update {
            id: EffortId(7),
            patch: vec![
                EffortChange::Kind(EffortKind::Experiment),
                EffortChange::QuestionOrGoal("Which provider?".into()),
                EffortChange::Status(EffortStatus::Closed),
            ],
        }),
        Command::Effort(EffortCommand::SetWhereWasI {
            id: EffortId(7),
            where_was_i: rich_effort().where_was_i,
        }),
        Command::Effort(EffortCommand::AddCriterion {
            effort: EffortId(7),
            text: "Fees compared".into(),
        }),
        Command::Effort(EffortCommand::ClaimCriterion {
            id: CriterionId(3),
            evidence: "finding 41".into(),
        }),
        Command::Effort(EffortCommand::ApproveCriterion { id: CriterionId(3) }),
        Command::Effort(EffortCommand::ReturnCriterion {
            id: CriterionId(3),
            reason: Some("Adyen is missing".into()),
        }),
        Command::Effort(EffortCommand::ReturnCriterion {
            id: CriterionId(3),
            reason: None,
        }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn effort_upsert_inserts_then_replaces_by_id() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Effort(EffortEvent::Upserted(effort(1, "unsorted"))),
            Event::Effort(EffortEvent::Upserted(effort(2, "guest-checkout"))),
        ],
    );
    assert_eq!(state.efforts.len(), 2);

    // A rename, a new card and a claimed criterion all arrive as the whole effort.
    let mut changed = effort(2, "guest-pay");
    changed.where_was_i.next_step = "Order email".into();
    changed.criteria.push(Criterion {
        id: CriterionId(5),
        effort: EffortId(2),
        text: "Pay as guest end to end".into(),
        state: CriterionState::Claimed,
        evidence: Some("PR #41".into()),
    });
    run(
        &mut state,
        vec![Event::Effort(EffortEvent::Upserted(changed.clone()))],
    );
    assert_eq!(state.efforts.len(), 2);
    assert_eq!(state.efforts[1], changed);
    assert_eq!(state.efforts[0].name, "unsorted");
}

/// Values written before efforts existed still read: a task lands in Unsorted, a chat line and a
/// prompt carry no tag, and a snapshot has no efforts.
#[test]
fn values_from_before_efforts_still_parse() {
    let mut old_task = serde_json::to_value(rich_task()).unwrap();
    let fields = old_task.as_object_mut().unwrap();
    fields.remove("effort");
    fields.remove("criterion");
    let parsed: Task = serde_json::from_value(old_task).unwrap();
    assert_eq!(parsed.effort, UNSORTED_EFFORT);
    assert_eq!(parsed.criterion, None);

    let line: ConversationEntry =
        serde_json::from_value(json!({"id": 1, "at_ms": 2, "kind": "user", "text": "hi"})).unwrap();
    assert_eq!(line.effort, None);

    let prompt: Command = serde_json::from_value(
        json!({"conversation": {"prompt": {"agent": "coordinator", "text": "hi"}}}),
    )
    .unwrap();
    assert!(matches!(
        prompt,
        Command::Conversation(ConversationCommand::Prompt { effort: None, .. })
    ));

    let mut old_snapshot = serde_json::to_value(Snapshot::default()).unwrap();
    old_snapshot.as_object_mut().unwrap().remove("efforts");
    let parsed: Snapshot = serde_json::from_value(old_snapshot).unwrap();
    assert!(parsed.efforts.is_empty());
}

/// Kinds and states are lower snake case, ids are bare numbers, and an unclaimed criterion has no
/// `evidence` key.
#[test]
fn an_effort_has_the_json_shape_the_page_reads() {
    let value = serde_json::to_value(rich_effort()).unwrap();
    assert_eq!(value["id"], json!(7));
    assert_eq!(value["kind"], json!("research"));
    assert_eq!(value["status"], json!("open"));
    assert_eq!(
        value["where_was_i"]["waits_on_you"],
        json!("Approve criterion 3")
    );
    assert_eq!(value["criteria"][0]["state"], json!("claimed"));
    assert_eq!(value["criteria"][0]["evidence"], json!("finding 41"));
    assert!(value["criteria"][1].get("evidence").is_none());
    roundtrip(&rich_effort());
}

//! Tests for the `decision` feature.

use crate::common::{Case, decision, roundtrip, run};
use dovetail_api::decision::{
    Decision, DecisionAuthor, DecisionCommand, DecisionEvent, DecisionId, NewDecision,
};
use dovetail_api::task::TaskId;
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![Case::new(
        "decision",
        Snapshot::default(),
        vec![(1, Event::Decision(DecisionEvent::Added(decision())))],
    )]
}

#[test]
fn decision_commands_and_events_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Decision(DecisionEvent::Added(decision())),
    });
    roundtrip(&Command::Decision(DecisionCommand::Log(NewDecision {
        task: TaskId(1),
        text: "Use the mock".into(),
        why: None,
        alternatives: vec![],
        supersedes: None,
    })));
}

#[test]
fn a_decision_is_added_once() {
    let mut state = Snapshot::default();
    let d = Decision {
        id: DecisionId(1),
        task: TaskId(1),
        at_ms: 0,
        by: DecisionAuthor::User,
        text: "x".into(),
        why: None,
        alternatives: vec![],
        supersedes: None,
        evidence: Vec::new(),
    };
    run(
        &mut state,
        vec![
            Event::Decision(DecisionEvent::Added(d.clone())),
            Event::Decision(DecisionEvent::Added(d)),
        ],
    );
    assert_eq!(state.decisions.len(), 1);
}

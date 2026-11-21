//! Tests for the `activity` feature.

use crate::common::{Case, activity_item, agent_message, id, roundtrip, run};
use dovetail_api::activity::{ActivityEvent, ActivityItem, ActivityKind, AgentMessage};
use dovetail_api::{Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    Case::new(
        "activity",
        Snapshot::default(),
        vec![
            (1, Event::Activity(ActivityEvent::Added(activity_item()))),
            (
                2,
                Event::Activity(ActivityEvent::MessageAdded(agent_message())),
            ),
        ],
    )
    .into()
}

impl From<Case> for Vec<Case> {
    fn from(case: Case) -> Self {
        vec![case]
    }
}

#[test]
fn activity_events_roundtrip() {
    for (n, event) in [
        Event::Activity(ActivityEvent::Added(activity_item())),
        Event::Activity(ActivityEvent::MessageAdded(agent_message())),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }
}

#[test]
fn activity_keeps_the_newest_500() {
    let mut state = Snapshot::default();
    let events = (1..=501)
        .map(|n| {
            Event::Activity(ActivityEvent::Added(ActivityItem {
                id: n,
                at_ms: 0,
                agent: None,
                kind: ActivityKind::Ci,
                text: format!("ci {n}"),
            }))
        })
        .collect();
    run(&mut state, events);
    assert_eq!(state.activity.len(), 500);
    assert_eq!(state.activity.first().map(|a| a.id), Some(2));
    assert_eq!(state.activity.last().map(|a| a.id), Some(501));
}

#[test]
fn messages_keep_the_newest_500() {
    let mut state = Snapshot::default();
    let events = (1..=501)
        .map(|n| {
            Event::Activity(ActivityEvent::MessageAdded(AgentMessage {
                id: n,
                at_ms: 0,
                from: id("coordinator"),
                to: id("s1"),
                text: format!("msg {n}"),
            }))
        })
        .collect();
    run(&mut state, events);
    assert_eq!(state.messages.len(), 500);
    assert_eq!(state.messages.first().map(|m| m.id), Some(2));
    assert_eq!(state.messages.last().map(|m| m.id), Some(501));
}

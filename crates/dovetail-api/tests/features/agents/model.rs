//! Tests for the `model` feature.

use crate::common::{Case, id, roundtrip};
use dovetail_api::model::{DriverKind, ModelCommand, ModelEvent, ModelOption, ReasoningEffort};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

pub fn cases() -> Vec<Case> {
    vec![Case::new(
        "model",
        Snapshot::default(),
        vec![(
            1,
            Event::Model(ModelEvent::Changed(vec![ModelOption {
                name: "opus".into(),
                resolved: None,
                efforts: None,
            }])),
        )],
    )]
}

#[test]
fn model_commands_and_events_roundtrip() {
    roundtrip(&DriverKind::Claude);
    roundtrip(&DriverKind::Agy);
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Model(ModelEvent::Changed(vec![ModelOption::named("fable")])),
    });
    roundtrip(&Command::Model(ModelCommand::Set {
        agent: id("slice-a"),
        model: "sonnet".into(),
        effort: Some(ReasoningEffort::Max),
    }));
}

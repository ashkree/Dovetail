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
                driver: dovetail_api::model::DriverKind::Claude,
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
    for command in [
        Command::Model(ModelCommand::SwitchDriver {
            agent: id("coordinator"),
            driver: DriverKind::Agy,
            model: "claude-opus-5-5".into(),
            effort: Some(ReasoningEffort::High),
            handoff: true,
        }),
        Command::Model(ModelCommand::SwitchDriver {
            agent: id("coordinator"),
            driver: DriverKind::Claude,
            model: "opus".into(),
            effort: None,
            handoff: false,
        }),
    ] {
        roundtrip(&command);
    }
    roundtrip(&ModelOption::named_on(DriverKind::Agy, "gemini-3.8-flash"));
}

#[test]
fn driver_defaults_to_claude_when_missing() {
    let mut old_option = serde_json::to_value(ModelOption::named("sonnet")).unwrap();
    old_option.as_object_mut().unwrap().remove("driver");
    let parsed: ModelOption = serde_json::from_value(old_option).unwrap();
    assert_eq!(parsed.driver, DriverKind::Claude);
}

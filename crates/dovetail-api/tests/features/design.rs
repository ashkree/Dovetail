//! Tests for the `design` feature.

use crate::common::{Case, id, roundtrip};
use dovetail_api::agent::{AgentEvent, AgentId};
use dovetail_api::design::{
    DesignCommand, DesignEvent, Preview, PreviewState, VariantLevel, VariantSet, VariantSlot,
};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn sample_slots() -> Vec<VariantSlot> {
    let slot = |name: &str, level, options: &[&str]| VariantSlot {
        name: name.into(),
        level,
        options: options.iter().map(|o| (*o).into()).collect(),
        kept: None,
    };
    vec![
        slot("theme", VariantLevel::Global, &["light", "dark"]),
        slot("header", VariantLevel::Component, &["plain", "bold"]),
    ]
}

/// ADR 0026: a preview goes through its states and goes with its slice; declared variants are
/// replaced whole, a pick changes the selection, and a set with no slots removes the entry.
pub fn cases() -> Vec<Case> {
    let web = AgentId("web".into());
    let declared = VariantSet::declared(web.clone(), sample_slots(), &Default::default());
    let mut picked = declared.clone();
    picked.selection.insert("theme".into(), "dark".into());
    let mut kept = picked.clone();
    kept.slots[0].kept = Some("dark".into());
    let preview = |state| Preview {
        agent: web.clone(),
        state,
    };
    let events = vec![
        Event::Design(DesignEvent::PreviewChanged(preview(PreviewState::Starting))),
        Event::Design(DesignEvent::PreviewChanged(preview(
            PreviewState::Running {
                url: "http://127.0.0.1:41000/".into(),
            },
        ))),
        Event::Design(DesignEvent::VariantsUpserted(declared.clone())),
        Event::Design(DesignEvent::VariantsUpserted(picked)),
        Event::Design(DesignEvent::VariantsUpserted(kept)),
        Event::Design(DesignEvent::VariantsUpserted(VariantSet {
            agent: web.clone(),
            slots: Vec::new(),
            selection: Default::default(),
        })),
        Event::Design(DesignEvent::VariantsUpserted(declared)),
        Event::Design(DesignEvent::PreviewChanged(preview(PreviewState::Failed {
            output: "the preview exited".into(),
        }))),
        Event::Agent(AgentEvent::Removed(web)),
    ];
    vec![Case::new(
        "design",
        Snapshot::default(),
        events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    )]
}

#[test]
fn design_commands_and_events_roundtrip() {
    let web = id("web");
    let variants = VariantSet::declared(web.clone(), sample_slots(), &Default::default());
    for (n, event) in [
        Event::Design(DesignEvent::PreviewChanged(Preview {
            agent: web.clone(),
            state: PreviewState::Running {
                url: "http://127.0.0.1:41000/".into(),
            },
        })),
        Event::Design(DesignEvent::VariantsUpserted(variants)),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    for command in [
        Command::Design(DesignCommand::StartPreview { agent: web.clone() }),
        Command::Design(DesignCommand::RestartPreview { agent: web.clone() }),
        Command::Design(DesignCommand::SelectVariant {
            agent: web.clone(),
            slot: "theme".into(),
            option: "dark".into(),
        }),
        Command::Design(DesignCommand::KeepVariants { agent: web }),
    ] {
        roundtrip(&command);
    }
}

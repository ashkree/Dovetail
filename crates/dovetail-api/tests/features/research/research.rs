//! Tests for the `research` feature.

use crate::common::{Case, research_listing, research_summary, roundtrip, run};
use dovetail_api::agent::{AgentId, AgentKind};
use dovetail_api::inbox::{InboxId, InboxKind};
use dovetail_api::library::DocImage;
use dovetail_api::research::{
    Evidence, FindingId, ProposalKind, ResearchCommand, ResearchContent, ResearchDocument,
    ResearchEvent, ResearchKind, ResearchWhat, SourceId,
};
use dovetail_api::tangent::{tangent_agent, tangent_task};
use dovetail_api::task::TaskId;
use dovetail_api::{Command, Event, ServerMsg, Snapshot};
use serde_json::json;

fn loaded_events() -> Vec<Event> {
    vec![
        Event::Research(ResearchEvent::Loaded {
            task: TaskId(4),
            what: ResearchWhat::Listing,
            content: ResearchContent::Listing(research_listing()),
        }),
        Event::Research(ResearchEvent::Loaded {
            task: TaskId(4),
            what: ResearchWhat::Source(SourceId(1)),
            content: ResearchContent::Document(ResearchDocument {
                path: None,
                markdown: "# S1".into(),
                images: vec![DocImage {
                    src: "fig.png".into(),
                    path: "/r/fig.png".into(),
                    width: 640,
                    height: 480,
                }],
            }),
        }),
    ]
}

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "research",
            Snapshot::default(),
            vec![(
                1,
                Event::Research(ResearchEvent::Upserted(research_summary())),
            )],
        ),
        Case::no_op(
            "research_loaded",
            Snapshot::default(),
            loaded_events()
                .into_iter()
                .enumerate()
                .map(|(i, e)| (i as u64 + 1, e))
                .collect(),
        ),
    ]
}

#[test]
fn research_commands_and_events_roundtrip() {
    let mut events = vec![Event::Research(ResearchEvent::Upserted(research_summary()))];
    events.extend(loaded_events());
    for (n, event) in events.into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Research(ResearchCommand::StartRound {
            task: TaskId(4),
            brief: "per seat or flat?".into(),
        }),
        Command::Research(ResearchCommand::CloseRound { task: TaskId(4) }),
        Command::Research(ResearchCommand::DropRound {
            task: TaskId(3),
            round: Some(2),
        }),
        Command::Research(ResearchCommand::SetKind {
            task: TaskId(3),
            kind: ResearchKind::Question,
        }),
        Command::Research(ResearchCommand::FileUnfiled {
            item: InboxId(4),
            task: TaskId(3),
        }),
        Command::Research(ResearchCommand::Load {
            task: TaskId(4),
            what: ResearchWhat::Brief { round: 1 },
        }),
        Command::Research(ResearchCommand::Load {
            task: TaskId(4),
            what: ResearchWhat::Finding(FindingId(3)),
        }),
        Command::Research(ResearchCommand::Load {
            task: TaskId(4),
            what: ResearchWhat::Report,
        }),
        Command::Research(ResearchCommand::StartAsk { task: TaskId(4) }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn a_research_proposal_and_its_evidence_survive_json() {
    let kind = InboxKind::Proposal {
        task: TaskId(4),
        kind: ProposalKind::Decision,
        text: "Flat pricing".into(),
        evidence: vec![
            Evidence::Finding(FindingId(3)),
            Evidence::Source(SourceId(12)),
        ],
        why: Some("simpler".into()),
        alternatives: vec![],
    };
    roundtrip(&kind);
    roundtrip(&InboxKind::UnfiledFinding {
        finding: 3,
        label: "pricing-pages".into(),
        sources: 2,
    });
    roundtrip(&InboxKind::NewTangents { count: 2 });
    roundtrip(&AgentKind::Tangent);
    // A tangent's session is named after its task in Tangents, and back.
    assert_eq!(tangent_agent(TaskId(12)), AgentId("tangent-12".into()));
    assert_eq!(
        tangent_task(&AgentId("tangent-12".into())),
        Some(TaskId(12))
    );
    assert_eq!(tangent_task(&AgentId("side-1".into())), None);
    assert_eq!(tangent_task(&AgentId("tangent-x".into())), None);
    // Evidence is written as the id the report cites.
    assert_eq!(
        serde_json::to_value(Evidence::Finding(FindingId(3))).unwrap(),
        json!("F3")
    );
    assert_eq!(
        Evidence::parse("[S12]"),
        Some(Evidence::Source(SourceId(12)))
    );
    for bad in ["", "F", "F0", "f3", "X3", "F-1", "F3x", "3"] {
        assert_eq!(Evidence::parse(bad), None, "{bad:?}");
    }
    assert!(serde_json::from_value::<Evidence>(json!("nope")).is_err());
}

#[test]
fn research_loaded_only_advances_seq() {
    let mut state = Snapshot::default();
    let events = loaded_events();
    let mut expected = state.clone();
    expected.seq = events.len() as u64;
    run(&mut state, events);
    assert_eq!(state, expected);
}

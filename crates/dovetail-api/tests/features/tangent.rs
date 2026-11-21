//! Tests for the `tangent` feature.

use crate::common::{Case, research_listing, roundtrip, run, tangent_summary, task};
use dovetail_api::project::ProjectId;
use dovetail_api::research::{FindingId, ResearchContent, ResearchWhat};
use dovetail_api::tangent::{TangentCommand, TangentEvent, TangentSummary};
use dovetail_api::task::TaskId;
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn loaded_event() -> Event {
    Event::Tangent(TangentEvent::Loaded {
        tangent: TaskId(2),
        what: ResearchWhat::Listing,
        content: ResearchContent::Listing(research_listing()),
    })
}

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "tangent",
            Snapshot::default(),
            vec![
                (
                    1,
                    Event::Tangent(TangentEvent::Upserted(TangentSummary {
                        task: task(7, "idea"),
                        research: None,
                    })),
                ),
                (
                    2,
                    Event::Tangent(TangentEvent::Upserted(TangentSummary {
                        task: task(8, "other"),
                        research: None,
                    })),
                ),
                (3, Event::Tangent(TangentEvent::Removed(TaskId(7)))),
            ],
        ),
        Case::no_op(
            "tangent_loaded",
            Snapshot::default(),
            vec![(1, loaded_event())],
        ),
    ]
}

#[test]
fn tangent_commands_and_events_roundtrip() {
    for (n, event) in [
        Event::Tangent(TangentEvent::Upserted(tangent_summary())),
        Event::Tangent(TangentEvent::Removed(TaskId(2))),
        loaded_event(),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Tangent(TangentCommand::New {
            question: "sourdough hydration?".into(),
        }),
        Command::Tangent(TangentCommand::Attach {
            tangent: TaskId(2),
            task: TaskId(4),
        }),
        Command::Tangent(TangentCommand::File {
            tangent: TaskId(2),
            project: ProjectId("market-sizing".into()),
        }),
        Command::Tangent(TangentCommand::StartFrom {
            tangent: TaskId(2),
            project: ProjectId("dovetail".into()),
        }),
        Command::Tangent(TangentCommand::Drop { tangent: TaskId(2) }),
        Command::Tangent(TangentCommand::Load {
            tangent: TaskId(2),
            what: ResearchWhat::Finding(FindingId(1)),
        }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn tangent_loaded_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(&mut state, vec![loaded_event()]);
    assert_eq!(state, expected);
}

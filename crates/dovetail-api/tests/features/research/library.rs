//! Tests for the `library` feature.

use crate::common::{Case, decision, research_listing, research_summary, roundtrip, run};
use dovetail_api::library::{
    BrowseWhat, BrowsedDoc, BrowsedListing, BrowsedProject, DocEntry, LibraryCommand, LibraryEvent,
};
use dovetail_api::project::{ProjectId, ProjectKind, ProjectSummary};
use dovetail_api::research::{ResearchDocument, ResearchWhat};
use dovetail_api::task::TaskId;
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn library_events() -> Vec<Event> {
    vec![
        Event::Library(LibraryEvent::DocsListed {
            docs: vec![DocEntry {
                path: "adr/0018-reading-mode-layout.md".into(),
                title: "Reading mode".into(),
            }],
        }),
        Event::Library(LibraryEvent::DocLoaded {
            path: "research.md".into(),
            document: ResearchDocument::text(None, "# Research".into()),
        }),
        Event::Library(LibraryEvent::ProjectBrowsed(Box::new(BrowsedProject {
            project: ProjectSummary {
                id: ProjectId("market-sizing".into()),
                name: "Market sizing".into(),
                repo_path: "/d/projects/market-sizing".into(),
                kind: ProjectKind::Research,
                ..ProjectSummary::default()
            },
            tasks: Vec::new(),
            decisions: vec![decision()],
            research: vec![research_summary()],
            listings: vec![BrowsedListing {
                task: TaskId(4),
                listing: research_listing(),
            }],
            docs: Vec::new(),
        }))),
        Event::Library(LibraryEvent::BrowsedLoaded {
            doc: BrowsedDoc {
                project: ProjectId("notes-site".into()),
                what: BrowseWhat::Doc("intro.md".into()),
            },
            document: ResearchDocument::text(None, "# Notes".into()),
        }),
    ]
}

pub fn cases() -> Vec<Case> {
    vec![Case::no_op(
        "library",
        Snapshot::default(),
        library_events()
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    )]
}

#[test]
fn library_commands_and_events_roundtrip() {
    for (n, event) in library_events().into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    for command in [
        Command::Library(LibraryCommand::ListDocs),
        Command::Library(LibraryCommand::BrowseProject {
            id: ProjectId("market-sizing".into()),
        }),
        Command::Library(LibraryCommand::LoadBrowsed {
            project: ProjectId("market-sizing".into()),
            what: BrowseWhat::Research {
                task: TaskId(4),
                what: ResearchWhat::Report,
            },
        }),
        Command::Library(LibraryCommand::LoadDoc {
            path: "adr/0018-reading-mode-layout.md".into(),
        }),
    ] {
        roundtrip(&command);
    }
}

#[test]
fn library_events_only_advance_seq() {
    let mut state = Snapshot::default();
    let events = library_events();
    let mut expected = state.clone();
    expected.seq = events.len() as u64;
    run(&mut state, events);
    assert_eq!(state, expected);
}

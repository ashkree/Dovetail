//! Tests for the `project` feature.

use crate::common::{Case, roundtrip, run};
use dovetail_api::project::{
    ProjectCommand, ProjectCounts, ProjectEvent, ProjectId, ProjectKind, ProjectSummary, RepoId,
    RepoSummary,
};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn project(id: &str, repos: &[&str]) -> ProjectSummary {
    ProjectSummary {
        id: ProjectId(id.into()),
        name: id.into(),
        repo_path: format!("/code/{id}").into(),
        kind: if repos.is_empty() {
            ProjectKind::Research
        } else {
            ProjectKind::Code
        },
        repos: repos
            .iter()
            .enumerate()
            .map(|(i, name)| RepoSummary {
                id: RepoId(i as i64 + 1),
                name: (*name).into(),
                path: format!("/code/{name}").into(),
            })
            .collect(),
        account: None,
        last_opened_ms: Some(1),
        counts: ProjectCounts::default(),
    }
}

fn passthrough_events() -> Vec<Event> {
    vec![
        Event::Project(ProjectEvent::Opening {
            id: ProjectId("market-sizing".into()),
        }),
        Event::Project(ProjectEvent::RecipeStarted {
            project: ProjectId("other".into()),
            name: "other".into(),
            path: "/home/user/.config/dovetail/projects/other.toml".into(),
            profiles: vec!["work".into()],
        }),
    ]
}

/// ADR 0021: a changed project replaces its row in `projects`, and `project` too when it is
/// the open one; a new one is appended.
pub fn cases() -> Vec<Case> {
    let snapshot = Snapshot {
        seq: 0,
        project: Some(project("shop", &["shop-api"])),
        projects: vec![project("shop", &["shop-api"]), project("notes", &[])],
        ..Snapshot::default()
    };
    let mut two = project("shop", &["shop-api", "shop-web"]);
    two.account = Some("work".into());
    two.counts = ProjectCounts {
        efforts: 3,
        running: 1,
        needs_you: 2,
    };
    let mut renamed = project("notes", &[]);
    renamed.name = "Market notes".into();
    vec![
        Case::new(
            "projects",
            snapshot,
            vec![
                (1, Event::Project(ProjectEvent::Upserted(two))),
                (2, Event::Project(ProjectEvent::Upserted(renamed))),
                (
                    3,
                    Event::Project(ProjectEvent::Upserted(project("garden", &["garden"]))),
                ),
            ],
        ),
        Case::no_op(
            "project_passthrough",
            Snapshot::default(),
            passthrough_events()
                .into_iter()
                .enumerate()
                .map(|(i, e)| (i as u64 + 1, e))
                .collect(),
        ),
    ]
}

#[test]
fn project_commands_and_events_roundtrip() {
    let mut events = passthrough_events();
    events.push(Event::Project(ProjectEvent::Upserted(project(
        "shop",
        &["shop-api"],
    ))));
    for (n, event) in events.into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Project(ProjectCommand::Open {
            id: ProjectId("dovetail".into()),
        }),
        Command::Project(ProjectCommand::NewResearch {
            name: "Market sizing".into(),
        }),
        Command::Project(ProjectCommand::Create {
            name: "Shop".into(),
            account: Some("work".into()),
            repo: Some("/code/shop".into()),
        }),
        Command::Project(ProjectCommand::Rename {
            id: ProjectId("shop".into()),
            name: "Shopfront".into(),
        }),
        Command::Project(ProjectCommand::SetAccount {
            id: ProjectId("shop".into()),
            account: None,
        }),
        Command::Project(ProjectCommand::AddRepo {
            project: ProjectId("shop".into()),
            path: "/code/shop-web".into(),
            name: Some("web".into()),
        }),
        Command::Project(ProjectCommand::RemoveRepo {
            project: ProjectId("shop".into()),
            repo: RepoId(2),
        }),
        Command::Project(ProjectCommand::Register {
            path: "/home/user/src/other".into(),
        }),
        Command::Project(ProjectCommand::SetRecipeProfile {
            project: ProjectId("other".into()),
            profile: "work".into(),
        }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn opening_and_recipe_started_only_advance_seq() {
    let mut state = Snapshot::default();
    let events = passthrough_events();
    let mut expected = state.clone();
    expected.seq = events.len() as u64;
    run(&mut state, events);
    assert_eq!(state, expected);
}

//! Protocol integration tests, organised by domain and feature.

mod activity;
mod agents;
mod app;
mod code;
mod common;
mod coverage;
mod research;
mod tracking;
mod visual;

use std::collections::BTreeMap;

use common::{
    Case, activity_item, agent_message, canvas_card, decision, id, inbox_items, map_summary,
    research_summary, result_name, rich_effort, rich_entry, rich_task, roundtrip, subagent,
    tangent_summary, task as build_task, todo, totals, warning, worktree_infos,
};
use dovetail_api::agent::{AgentId, SlashCommand};
use dovetail_api::canvas::{Block, BlockId, Card, CardId, CardKind, CardLink, Layout};
use dovetail_api::conversation::ConversationTail;
use dovetail_api::decision::DecisionAuthor;
use dovetail_api::design::{Preview, PreviewState, VariantLevel, VariantSet, VariantSlot};
use dovetail_api::effort::EffortId;
use dovetail_api::journal::JournalState;
use dovetail_api::model::{DriverKind, ModelOption};
use dovetail_api::project::{ProjectId, ProjectKind, ProjectSummary};
use dovetail_api::status::{
    DriverUsage, LimitWindow, UsageGroup, UsageLimits, UsageStatus, UsageWindow,
};
use dovetail_api::task::TaskEvent;
use dovetail_api::{ApplyResult, Event, PROTOCOL_VERSION, ServerMsg, Snapshot, apply};

#[test]
fn hello_and_a_full_snapshot_roundtrip() {
    let project = ProjectSummary {
        id: ProjectId("dovetail".into()),
        name: "Dovetail".into(),
        repo_path: "/home/user/src/dovetail".into(),
        kind: ProjectKind::Code,
        ..ProjectSummary::default()
    };
    let research = ProjectSummary {
        id: ProjectId("market-sizing".into()),
        name: "Market sizing".into(),
        repo_path: "/home/user/.local/share/dovetail/projects/market-sizing".into(),
        kind: ProjectKind::Research,
        ..ProjectSummary::default()
    };
    let snapshot = Snapshot {
        seq: 42,
        now_ms: 1_700_000_000_000,
        project: Some(project.clone()),
        projects: vec![project, research],
        efforts: vec![rich_effort()],
        blocks: vec![Block {
            id: BlockId(4),
            effort: EffortId(7),
            title: "payment-providers".into(),
            layout: Layout::QuestionTree,
            x: 0,
            y: 0,
        }],
        cards: vec![
            canvas_card(),
            Card {
                id: CardId(92),
                block: None,
                kind: CardKind::Idea,
                x: 0,
                y: 0,
                reference: None,
                title: "Ask Mollie".into(),
                text: "Ask Mollie\nabout EU-only pricing".into(),
                seen: false,
                ..canvas_card()
            },
        ],
        card_links: vec![CardLink {
            from: CardId(92),
            to: CardId(90),
            label: "answers".into(),
        }],
        previews: vec![
            Preview {
                agent: AgentId("web-slice".into()),
                state: PreviewState::Running {
                    url: "http://127.0.0.1:41000/".into(),
                },
            },
            Preview {
                agent: AgentId("api-slice".into()),
                state: PreviewState::Failed {
                    output: "nothing listened".into(),
                },
            },
        ],
        variants: vec![VariantSet::declared(
            AgentId("web-slice".into()),
            vec![VariantSlot {
                name: "theme".into(),
                level: VariantLevel::Global,
                options: vec!["light".into(), "dark".into()],
                kept: Some("dark".into()),
            }],
            &Default::default(),
        )],
        effort_limit: 4,
        tasks: vec![rich_task()],
        agents: vec![subagent()],
        todos: vec![todo()],
        inbox: inbox_items(),
        activity: vec![activity_item()],
        messages: vec![agent_message()],
        usage: UsageLimits {
            five_hour: Some(UsageWindow {
                utilization: 0.34,
                resets_at_ms: Some(1_700_008_400_000),
                status: UsageStatus::Allowed,
            }),
            seven_day: None,
        },
        decisions: vec![
            decision(),
            dovetail_api::decision::Decision {
                by: DecisionAuthor::User,
                ..decision()
            },
        ],
        conversations: BTreeMap::from([(
            id("slice-a"),
            ConversationTail {
                entries: vec![rich_entry()],
                has_more: true,
            },
        )]),
        totals: totals(),
        warnings: vec![warning()],
        worktrees: worktree_infos(),
        commands: BTreeMap::from([(
            AgentId("coordinator".into()),
            vec![SlashCommand {
                name: "compact".into(),
                description: "Summarise".into(),
                argument_hint: "<instructions>".into(),
            }],
        )]),
        profile: Some("maveron".into()),
        agy_usage: Some(DriverUsage {
            driver: DriverKind::Agy,
            plan: None,
            groups: vec![UsageGroup {
                name: "Gemini Models".into(),
                models: "Gemini 4 Neon".into(),
                windows: vec![LimitWindow {
                    name: "5-hour".into(),
                    used_fraction: 0.1,
                    resets_at_ms: Some(1_700_000_000_000),
                }],
            }],
            note: None,
            failed: false,
        }),
        journal: Some(JournalState {
            latest: Some("2026-11-10".into()),
            closing: false,
            fresh: false,
        }),
        upgrade_building: true,
        maps: vec![map_summary()],
        research: vec![research_summary()],
        tangents: vec![tangent_summary()],
        models: vec![
            ModelOption::named("opus"),
            ModelOption {
                name: "claude-haiku-4-5-20251001".into(),
                resolved: Some("claude-haiku-4-5-20251001".into()),
                efforts: Some(vec![]),
                driver: dovetail_api::model::DriverKind::Claude,
            },
        ],
    };
    roundtrip(&ServerMsg::Hello {
        protocol: PROTOCOL_VERSION,
    });
    roundtrip(&ServerMsg::Snapshot(snapshot));
}

// Sequencing

#[test]
fn next_seq_applies_and_advances_seq() {
    let mut state = Snapshot {
        seq: 7,
        ..Snapshot::default()
    };
    assert_eq!(
        apply(
            &mut state,
            8,
            Event::Task(TaskEvent::Upserted(build_task(1, "a")))
        ),
        ApplyResult::Applied
    );
    assert_eq!(state.seq, 8);
    assert_eq!(state.tasks.len(), 1);
}

#[test]
fn skipped_seq_is_a_gap_and_changes_nothing() {
    let mut state = Snapshot {
        seq: 7,
        ..Snapshot::default()
    };
    let before = state.clone();
    assert_eq!(
        apply(
            &mut state,
            9,
            Event::Task(TaskEvent::Upserted(build_task(1, "a")))
        ),
        ApplyResult::Gap {
            expected: 8,
            got: 9
        }
    );
    assert_eq!(state, before);
}

#[test]
fn old_or_repeated_seq_is_stale_and_changes_nothing() {
    let mut state = Snapshot {
        seq: 7,
        ..Snapshot::default()
    };
    let before = state.clone();
    assert_eq!(
        apply(
            &mut state,
            7,
            Event::Task(TaskEvent::Upserted(build_task(1, "a")))
        ),
        ApplyResult::Stale
    );
    assert_eq!(
        apply(
            &mut state,
            3,
            Event::Task(TaskEvent::Upserted(build_task(1, "a")))
        ),
        ApplyResult::Stale
    );
    assert_eq!(state, before);
}

pub(crate) fn all_cases() -> Vec<Case> {
    [
        activity::cases(),
        agents::cases(),
        app::cases(),
        code::cases(),
        research::cases(),
        tracking::cases(),
        visual::cases(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Replays each feature's cases through `apply` and checks every event's result: all applied,
/// except the deliberate gap and repeat in `agents_inbox_and_order`. Each case must also change
/// the snapshot (and no-op cases must leave everything except `seq` unchanged).
#[test]
fn every_case_replays_with_the_expected_results() {
    for case in all_cases() {
        let mut state = case.snapshot.clone();
        let results: Vec<_> = case
            .events
            .iter()
            .map(|(seq, event)| result_name(apply(&mut state, *seq, event.clone())))
            .collect();
        let expected: Vec<&str> = if case.name == "agents_inbox_and_order" {
            vec![
                "applied", "gap", "stale", "applied", "applied", "applied", "applied",
            ]
        } else {
            vec!["applied"; case.events.len()]
        };
        assert_eq!(results, expected, "case {}", case.name);
        assert_ne!(state, case.snapshot, "case {} changed nothing", case.name);
        if case.only_advances_seq {
            let mut expected_state = case.snapshot.clone();
            expected_state.seq = state.seq;
            assert_eq!(
                state, expected_state,
                "case {} changed more than seq",
                case.name
            );
        }
    }
}

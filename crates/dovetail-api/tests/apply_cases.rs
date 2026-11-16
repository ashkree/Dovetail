//! Shared cases for the two copies of `apply()` (ADR 0023, point 5): this crate's and the GUI's
//! `gui/src/core/apply.ts`. Each case is a snapshot, numbered events and the snapshot Rust's
//! `apply` makes of them, written to `tests/fixtures/apply/<name>.json`; the GUI's test replays
//! the same files through its own reducer.
//!
//! This test fails when a file differs from what Rust computes now. After a deliberate change,
//! rewrite them with `DOVETAIL_BLESS=1 cargo test -p dovetail-api --test apply_cases`, and the
//! GUI's test then says whether its reducer agrees.

use std::path::Path;

use dovetail_api::map::{MapId, MapKind, MapSummary};
use dovetail_api::research::ResearchSummary;
use dovetail_api::*;
use serde_json::{Value, json};

fn task(id: i64, title: &str) -> Task {
    Task {
        id: TaskId(id),
        title: title.into(),
        status: TaskStatus::Todo,
        p_level: None,
        bucket: None,
        order: 0,
        deps: vec![],
        notes: String::new(),
        pr: None,
        follow_up_of: None,
        coordinator_created: false,
        related: vec![],
        effort: UNSORTED_EFFORT,
        criterion: None,
    }
}

fn agent(id: &str) -> Agent {
    Agent {
        id: AgentId(id.into()),
        name: id.into(),
        kind: AgentKind::Slice,
        parent: None,
        task: Some(TaskId(1)),
        worktree: None,
        status: AgentStatus::Working,
        status_line: String::new(),
        memory_mb: None,
        cost_usd: 0.0,
        permission_mode: PermissionMode::Auto,
        context_pct: None,
        progress: None,
        last_activity_ms: 0,
        lock: SessionLock::Headless,
        model: None,
        effort: None,
        subagent_tokens: 0,
        tool: None,
    }
}

fn entry(id: u64, kind: EntryKind, text: &str, effort: Option<i64>) -> ConversationEntry {
    ConversationEntry {
        id: EntryId(id),
        at_ms: 1_000,
        kind,
        text: text.into(),
        attachments: vec![],
        effort: effort.map(EffortId),
    }
}

fn effort(id: i64, name: &str) -> Effort {
    Effort {
        id: EffortId(id),
        name: name.into(),
        kind: EffortKind::Feature,
        question_or_goal: String::new(),
        status: EffortStatus::Open,
        where_was_i: WhereWasI::default(),
        criteria: vec![],
    }
}

fn coordinator() -> AgentId {
    AgentId("coordinator".into())
}

fn map(id: &str, updated_ms: i64) -> MapSummary {
    MapSummary {
        id: MapId(id.into()),
        title: id.into(),
        kind: MapKind::Structure,
        task: None,
        drawn_by: None,
        version: 1,
        created_ms: 1,
        updated_ms,
    }
}

fn todo(list: &str, id: &str) -> TodoItem {
    TodoItem {
        list: TodoListId(list.into()),
        id: id.into(),
        subject: format!("do {id}"),
        status: TodoStatus::Pending,
        blocked_by: vec![],
        created_by: None,
    }
}

fn research(task: i64, findings: u32) -> ResearchSummary {
    ResearchSummary {
        task: TaskId(task),
        dir: "/r".into(),
        rounds: vec![],
        findings,
        quick_findings: 0,
        sources: 0,
        report_ms: None,
        kind: None,
    }
}

/// Every other change the reducer stores, one event each, plus the events it passes through.
fn the_rest() -> Case {
    let a = AgentId("slice-a".into());
    let mut snapshot = Snapshot {
        seq: 0,
        agents: vec![agent("slice-a")],
        maps: vec![map("m1", 1), map("m2", 2)],
        todos: vec![todo("l1", "1"), todo("l2", "1")],
        warnings: vec![Warning {
            id: WarningId("w".into()),
            text: "careful".into(),
        }],
        ..Snapshot::default()
    };
    snapshot.commands.insert(a.clone(), vec![]);
    let worktree: WorktreeInfo = serde_json::from_value(json!({
        "agent": "slice-a", "path": "/w", "branch": "dovetail/a", "base": "main",
        "sync": null, "changes": null, "unpushed": null, "pr": "none", "setup": "done",
        "checked_ms": 3,
    }))
    .expect("worktree json");
    let mut renamed = agent("slice-a");
    renamed.status_line = "testing".into();
    let events = vec![
        Event::AgentUpserted(renamed),
        Event::WorktreeUpserted(worktree),
        Event::TodosReplaced {
            list: TodoListId("l1".into()),
            items: vec![todo("l1", "2"), todo("l1", "3")],
        },
        Event::ActivityAdded(ActivityItem {
            id: 1,
            at_ms: 4,
            agent: Some(a.clone()),
            kind: ActivityKind::Lifecycle,
            text: "slice a ready".into(),
        }),
        Event::MessageAdded(AgentMessage {
            id: 1,
            at_ms: 5,
            from: a.clone(),
            to: coordinator(),
            text: "done".into(),
        }),
        Event::DecisionAdded(Decision {
            id: DecisionId(1),
            task: TaskId(1),
            at_ms: 6,
            by: DecisionAuthor::User,
            text: "use sqlite".into(),
            why: None,
            alternatives: vec![],
            supersedes: None,
            evidence: vec![],
        }),
        // A redrawn map moves to the front.
        Event::MapUpserted(map("m1", 9)),
        Event::ResearchUpserted(research(1, 2)),
        Event::ResearchUpserted(research(1, 3)),
        Event::TangentUpserted(TangentSummary {
            task: task(7, "idea"),
            research: None,
        }),
        Event::TangentUpserted(TangentSummary {
            task: task(8, "other"),
            research: None,
        }),
        Event::TangentRemoved(TaskId(7)),
        Event::TotalsChanged(Totals::default()),
        Event::UsageChanged(UsageLimits::default()),
        Event::WarningCleared(WarningId("w".into())),
        Event::CommandsChanged {
            agent: a.clone(),
            commands: vec![SlashCommand {
                name: "review".into(),
                description: "review it".into(),
                argument_hint: String::new(),
            }],
        },
        Event::ModelsChanged(vec![ModelOption {
            name: "opus".into(),
            resolved: None,
            efforts: None,
        }]),
        Event::JournalChanged(JournalState {
            latest: Some("2026-11-16".into()),
            closing: false,
            fresh: true,
        }),
        Event::UpgradeBuilding(true),
        // Removing an agent drops its conversation, worktree and commands.
        Event::AgentRemoved(a),
    ];
    Case {
        name: "the_rest",
        snapshot,
        events: events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    }
}

/// One case: a name, the starting snapshot and the events in order.
struct Case {
    name: &'static str,
    snapshot: Snapshot,
    events: Vec<(u64, Event)>,
}

fn cases() -> Vec<Case> {
    let mut with_tail = Snapshot {
        seq: 10,
        now_ms: 5_000,
        ..Snapshot::default()
    };
    with_tail.conversations.insert(
        coordinator(),
        ConversationTail {
            entries: vec![entry(5, EntryKind::User, "hello", Some(2))],
            has_more: true,
        },
    );
    let mut with_agent = Snapshot {
        seq: 3,
        agents: vec![agent("slice-a"), agent("slice-b")],
        ..Snapshot::default()
    };
    with_agent.conversations.insert(
        AgentId("slice-a".into()),
        ConversationTail {
            entries: vec![entry(1, EntryKind::User, "go", None)],
            has_more: false,
        },
    );
    let mut claimed = effort(2, "guest-checkout");
    claimed.criteria.push(Criterion {
        id: CriterionId(3),
        effort: EffortId(2),
        text: "pay as guest".into(),
        state: CriterionState::Claimed,
        evidence: Some("PR #41".into()),
    });
    let mut moved = task(1, "form");
    moved.effort = EffortId(2);
    moved.criterion = Some(CriterionId(3));
    moved.status = TaskStatus::Review;
    vec![
        Case {
            name: "tasks_and_efforts",
            snapshot: Snapshot {
                seq: 0,
                ..Snapshot::default()
            },
            events: vec![
                (1, Event::EffortUpserted(effort(1, "unsorted"))),
                (2, Event::EffortUpserted(effort(2, "guest-pay"))),
                (3, Event::TaskUpserted(task(1, "form"))),
                (4, Event::TaskUpserted(task(2, "email"))),
                (5, Event::EffortUpserted(claimed)),
                (6, Event::TaskUpserted(moved)),
                (7, Event::TaskRemoved(TaskId(2))),
            ],
        },
        Case {
            name: "conversation",
            snapshot: with_tail,
            events: vec![
                (
                    11,
                    Event::ConversationAppended {
                        agent: coordinator(),
                        entry: entry(6, EntryKind::Assistant, "", Some(2)),
                    },
                ),
                (
                    12,
                    Event::AssistantDelta {
                        agent: coordinator(),
                        entry: EntryId(6),
                        text: "Will ".into(),
                    },
                ),
                (
                    13,
                    Event::AssistantDelta {
                        agent: coordinator(),
                        entry: EntryId(6),
                        text: "do.".into(),
                    },
                ),
                // A client that joined mid-stream starts the entry at snapshot time.
                (
                    14,
                    Event::AssistantDelta {
                        agent: coordinator(),
                        entry: EntryId(9),
                        text: "late".into(),
                    },
                ),
                (
                    15,
                    Event::ConversationPage {
                        agent: coordinator(),
                        entries: vec![
                            entry(2, EntryKind::User, "older", None),
                            entry(5, EntryKind::User, "hello", Some(2)),
                        ],
                        has_more: false,
                    },
                ),
                (
                    16,
                    Event::ConversationAppended {
                        agent: coordinator(),
                        entry: entry(6, EntryKind::Assistant, "Will do, now.", Some(2)),
                    },
                ),
            ],
        },
        Case {
            name: "agents_inbox_and_order",
            snapshot: with_agent,
            events: vec![
                (
                    4,
                    Event::InboxUpserted(InboxItem {
                        id: InboxId(1),
                        agent: AgentId("slice-a".into()),
                        at_ms: 7,
                        kind: InboxKind::Question {
                            text: "which one?".into(),
                        },
                    }),
                ),
                // A gap and a repeat change nothing.
                (9, Event::TaskRemoved(TaskId(1))),
                (4, Event::TaskRemoved(TaskId(1))),
                (5, Event::AgentRemoved(AgentId("slice-a".into()))),
                (6, Event::InboxResolved { id: InboxId(1) }),
                (
                    7,
                    Event::WarningSet(Warning {
                        id: WarningId("w".into()),
                        text: "careful".into(),
                    }),
                ),
                (
                    8,
                    Event::CommandFailed {
                        message: "nothing stored".into(),
                    },
                ),
            ],
        },
        the_rest(),
    ]
}

/// Events `apply()` passes through without storing: the UI acts on them as they arrive.
const PASSED_THROUGH: &[&str] = &[
    "browsed_loaded",
    "clipboard_pasted",
    "command_failed",
    "doc_loaded",
    "docs_listed",
    "lend_terminal",
    "map_loaded",
    "opening_project",
    "paths_listed",
    "project_browsed",
    "recipe_started",
    "research_loaded",
    "tangent_loaded",
    "terminal_returned",
    "upgrade_restarting",
];

/// The serde tag of an event: the string itself, or the one key of its object.
fn tag(event: &Event) -> String {
    match serde_json::to_value(event).expect("event json") {
        Value::String(tag) => tag,
        Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
        other => panic!("unexpected event shape {other}"),
    }
}

/// ADR 0023, point 5: a new `Event` needs a new case. The tags come from the generated
/// TypeScript (`scripts/check.sh` keeps it current), so a variant added in Rust shows up there.
#[test]
fn every_stored_event_has_a_shared_case() {
    let generated =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gui/src/core/generated/Event.ts");
    let text = std::fs::read_to_string(&generated).expect("generated Event.ts");
    let mut tags: Vec<String> = text
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(str::to_owned)
        .collect();
    tags.sort();
    tags.dedup();
    let covered: Vec<String> = cases()
        .iter()
        .flat_map(|c| c.events.iter().map(|(_, e)| tag(e)))
        .collect();
    let missing: Vec<&String> = tags
        .iter()
        .filter(|t| !covered.contains(t) && !PASSED_THROUGH.contains(&t.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "events with no shared apply case: {missing:?}"
    );
}

fn result_name(result: ApplyResult) -> &'static str {
    match result {
        ApplyResult::Applied => "applied",
        ApplyResult::Gap { .. } => "gap",
        ApplyResult::Stale => "stale",
    }
}

#[test]
fn shared_apply_cases_match_the_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/apply");
    let bless = std::env::var("DOVETAIL_BLESS").is_ok_and(|v| v == "1");
    for case in cases() {
        let mut state = case.snapshot.clone();
        let mut results = Vec::new();
        for (seq, event) in &case.events {
            results.push(result_name(apply(&mut state, *seq, event.clone())));
        }
        let computed = json!({
            "snapshot": case.snapshot,
            "events": case.events.iter().map(|(seq, event)| json!({"seq": seq, "event": event})).collect::<Vec<_>>(),
            "results": results,
            "expected": state,
        });
        let path = dir.join(format!("{}.json", case.name));
        if bless {
            std::fs::create_dir_all(&dir).expect("fixture dir");
            let text = serde_json::to_string_pretty(&computed).expect("json") + "\n";
            std::fs::write(&path, text).expect("write fixture");
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "{} is missing: run DOVETAIL_BLESS=1 cargo test -p dovetail-api --test apply_cases",
                path.display()
            )
        });
        let stored: Value = serde_json::from_str(&text).expect("fixture json");
        assert_eq!(
            stored,
            computed,
            "{} differs from what apply() computes now; rewrite it with DOVETAIL_BLESS=1 and run \
             the GUI's tests",
            path.display()
        );
    }
}

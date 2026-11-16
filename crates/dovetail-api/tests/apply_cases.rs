//! Shared cases for the two copies of `apply()` (ADR 0023, point 5): this crate's and the GUI's
//! `gui/src/core/apply.ts`. Each case is a snapshot, numbered events and the snapshot Rust's
//! `apply` makes of them, written to `tests/fixtures/apply/<name>.json`; the GUI's test replays
//! the same files through its own reducer.
//!
//! This test fails when a file differs from what Rust computes now. After a deliberate change,
//! rewrite them with `DOVETAIL_BLESS=1 cargo test -p dovetail-api --test apply_cases`, and the
//! GUI's test then says whether its reducer agrees.

use std::path::Path;

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
    ]
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

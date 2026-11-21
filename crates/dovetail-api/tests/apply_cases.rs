//! Reducer cases for `apply()` beyond `apply.rs`: canvas, design, projects, efforts, maps, research,
//! tangents and the other singletons. Each case is a snapshot and numbered events; the test replays
//! them and checks each event's result.

use dovetail_api::map::{MapId, MapKind, MapSummary};
use dovetail_api::research::ResearchSummary;
use dovetail_api::*;
use serde_json::json;

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
        driver: DriverKind::Claude,
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
            driver: DriverKind::Claude,
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

fn card(id: i64, kind: CardKind, block: Option<i64>, title: &str) -> Card {
    Card {
        id: CardId(id),
        effort: EffortId(2),
        block: block.map(BlockId),
        kind,
        x: 0,
        y: 0,
        reference: None,
        title: title.into(),
        text: String::new(),
        missing: false,
        added_by: "user".into(),
        created_ms: 1,
        seen: true,
    }
}

/// Blocks, cards and links: a card arrives in incoming, is placed and moved, linked, relabelled
/// and unlinked; removing a card drops its links.
fn canvas() -> Case {
    let mut placed = card(1, CardKind::Question, Some(1), "Which provider?");
    placed.text = "Which provider?".into();
    let mut incoming = card(2, CardKind::Finding, None, "Stripe fees");
    incoming.reference = Some(CardRef {
        kind: RefKind::Finding,
        id: "41".into(),
    });
    let mut on_canvas = incoming.clone();
    on_canvas.block = Some(BlockId(1));
    on_canvas.x = 320;
    on_canvas.y = 180;
    let mut moved = on_canvas.clone();
    moved.x = 340;
    let link = |label: &str| CardLink {
        from: CardId(1),
        to: CardId(2),
        label: label.into(),
    };
    let events = vec![
        Event::BlockUpserted(Block {
            id: BlockId(1),
            effort: EffortId(2),
            title: "guest-pay".into(),
            layout: Layout::QuestionTree,
            x: 0,
            y: 0,
        }),
        Event::CardUpserted(placed),
        Event::CardUpserted(incoming),
        Event::CardUpserted(on_canvas),
        Event::CardUpserted(moved),
        Event::CardLinked(link("")),
        Event::CardLinked(link("answers")),
        Event::CardLinked(CardLink {
            from: CardId(1),
            to: CardId(3),
            label: String::new(),
        }),
        Event::CardUnlinked {
            from: CardId(1),
            to: CardId(3),
        },
        Event::CardLoaded {
            id: CardId(2),
            body: "Stripe charges 1.5 %.".into(),
        },
        Event::CardRemoved(CardId(1)),
    ];
    Case {
        name: "canvas",
        snapshot: Snapshot::default(),
        events: events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    }
}

/// ADR 0026: a preview goes through its states and goes with its slice; declared variants are
/// replaced whole, a pick changes the selection, and a set with no slots removes the entry.
fn design() -> Case {
    let web = AgentId("web".into());
    let slot = |name: &str, level, options: &[&str]| VariantSlot {
        name: name.into(),
        level,
        options: options.iter().map(|o| (*o).into()).collect(),
        kept: None,
    };
    let declared = VariantSet::declared(
        web.clone(),
        vec![
            slot("theme", VariantLevel::Global, &["light", "dark"]),
            slot("header", VariantLevel::Component, &["plain", "bold"]),
        ],
        &Default::default(),
    );
    let mut picked = declared.clone();
    picked.selection.insert("theme".into(), "dark".into());
    let mut kept = picked.clone();
    kept.slots[0].kept = Some("dark".into());
    let preview = |state| Preview {
        agent: web.clone(),
        state,
    };
    let events = vec![
        Event::PreviewChanged(preview(PreviewState::Starting)),
        Event::PreviewChanged(preview(PreviewState::Running {
            url: "http://127.0.0.1:41000/".into(),
        })),
        Event::VariantsUpserted(declared.clone()),
        Event::VariantsUpserted(picked),
        Event::VariantsUpserted(kept),
        Event::VariantsUpserted(VariantSet {
            agent: web.clone(),
            slots: Vec::new(),
            selection: Default::default(),
        }),
        Event::VariantsUpserted(declared),
        Event::PreviewChanged(preview(PreviewState::Failed {
            output: "the preview exited".into(),
        })),
        Event::AgentRemoved(web),
    ];
    Case {
        name: "design",
        snapshot: Snapshot::default(),
        events: events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    }
}

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

/// ADR 0021: a changed project replaces its row in `projects`, and `project` too when it is
/// the open one; a new one is appended.
fn projects() -> Case {
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
    Case {
        name: "projects",
        snapshot,
        events: vec![
            (1, Event::ProjectUpserted(two)),
            (2, Event::ProjectUpserted(renamed)),
            (3, Event::ProjectUpserted(project("garden", &["garden"]))),
        ],
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
        canvas(),
        design(),
        projects(),
    ]
}

fn result_name(result: ApplyResult) -> &'static str {
    match result {
        ApplyResult::Applied => "applied",
        ApplyResult::Gap { .. } => "gap",
        ApplyResult::Stale => "stale",
    }
}

/// Replays each case through `apply` and checks every event's result: all applied, except the
/// deliberate gap and repeat in `agents_inbox_and_order`. Each case must also change the snapshot.
#[test]
fn every_case_replays_with_the_expected_results() {
    for case in cases() {
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
    }
}

#[test]
fn usage_loaded_leaves_snapshot_unchanged() {
    let mut state = Snapshot {
        seq: 4,
        agents: vec![agent("coordinator")],
        ..Snapshot::default()
    };
    let expected = Snapshot {
        seq: 5,
        ..state.clone()
    };
    let report = DriverUsage {
        driver: DriverKind::Claude,
        plan: Some("pro".into()),
        groups: vec![UsageGroup {
            name: "Claude Code".into(),
            models: String::new(),
            windows: vec![LimitWindow {
                name: "5-hour".into(),
                used_fraction: 0.42,
                resets_at_ms: Some(1_700_000_000_000),
            }],
        }],
        note: None,
        failed: false,
    };
    assert_eq!(
        apply(&mut state, 5, Event::UsageLoaded(vec![report])),
        ApplyResult::Applied
    );
    assert_eq!(state, expected);
}

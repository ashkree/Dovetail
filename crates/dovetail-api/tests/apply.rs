//! `apply()`: the reducer every client state change goes through.

use dovetail_api::*;

fn task(id: i64, title: &str) -> Task {
    Task {
        id: TaskId(id),
        title: title.into(),
        status: TaskStatus::Todo,
        p_level: Some(PLevel::P1),
        bucket: Some(Bucket::Now),
        order: 0,
        deps: vec![],
        notes: String::new(),
        pr: None,
        follow_up_of: None,
        coordinator_created: false,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    }
}

fn agent(id: &str, status: AgentStatus) -> Agent {
    Agent {
        id: AgentId(id.into()),
        name: id.into(),
        kind: AgentKind::Slice,
        parent: None,
        task: None,
        worktree: None,
        status,
        status_line: String::new(),
        memory_mb: Some(210),
        cost_usd: 0.0,
        permission_mode: PermissionMode::Auto,
        context_pct: Some(5),
        progress: None,
        last_activity_ms: 0,
        lock: SessionLock::Headless,
        model: None,
        effort: None,
        subagent_tokens: 0,
        tool: None,
    }
}

fn entry(id: u64, kind: EntryKind, text: &str) -> ConversationEntry {
    ConversationEntry {
        id: EntryId(id),
        at_ms: 1_000,
        kind,
        text: text.into(),
        attachments: Vec::new(),
        effort: None,
    }
}

fn question(id: u64, agent: &str) -> InboxItem {
    InboxItem {
        id: InboxId(id),
        agent: AgentId(agent.into()),
        at_ms: 1_000,
        kind: InboxKind::Question {
            text: "Which port?".into(),
        },
    }
}

/// Applies events with consecutive seqs, asserting each one applies.
fn run(state: &mut Snapshot, events: Vec<Event>) {
    for event in events {
        let seq = state.seq + 1;
        assert_eq!(apply(state, seq, event), ApplyResult::Applied);
    }
}

fn id(s: &str) -> AgentId {
    AgentId(s.into())
}

// Maps

fn shelf_entry(id: &str, version: u32) -> map::MapSummary {
    map::MapSummary {
        id: map::MapId(id.into()),
        title: id.into(),
        kind: map::MapKind::Structure,
        task: None,
        drawn_by: None,
        version,
        created_ms: 0,
        updated_ms: i64::from(version),
    }
}

#[test]
fn a_drawn_map_goes_to_the_front_of_the_shelf_and_a_redraw_replaces_it() {
    let mut s = Snapshot::default();
    run(
        &mut s,
        vec![
            Event::MapUpserted(shelf_entry("a", 1)),
            Event::MapUpserted(shelf_entry("b", 1)),
            Event::MapUpserted(shelf_entry("a", 2)),
        ],
    );
    let shelf: Vec<(&str, u32)> = s
        .maps
        .iter()
        .map(|m| (m.id.0.as_str(), m.version))
        .collect();
    assert_eq!(shelf, [("a", 2), ("b", 1)]);
    // Loaded content passes through without touching the snapshot.
    let before = s.maps.clone();
    let loaded = map::Map {
        id: map::MapId("a".into()),
        version: 2,
        content: map::MapContent {
            title: "a".into(),
            kind: map::MapKind::Structure,
            nodes: vec![],
            groups: vec![],
            connections: vec![],
            flows: vec![],
            lanes: vec![],
        },
    };
    run(&mut s, vec![Event::MapLoaded(loaded)]);
    assert_eq!(s.maps, before);
}

// Sequencing

#[test]
fn next_seq_applies_and_advances_seq() {
    let mut state = Snapshot {
        seq: 7,
        ..Snapshot::default()
    };
    assert_eq!(
        apply(&mut state, 8, Event::TaskUpserted(task(1, "a"))),
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
        apply(&mut state, 9, Event::TaskUpserted(task(1, "a"))),
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
        apply(&mut state, 7, Event::TaskUpserted(task(1, "a"))),
        ApplyResult::Stale
    );
    assert_eq!(
        apply(&mut state, 3, Event::TaskUpserted(task(1, "a"))),
        ApplyResult::Stale
    );
    assert_eq!(state, before);
}

// Tasks

#[test]
fn task_upsert_inserts_then_replaces_by_id() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::TaskUpserted(task(1, "first")),
            Event::TaskUpserted(task(2, "second")),
            Event::TaskUpserted(task(1, "renamed")),
        ],
    );
    let titles: Vec<_> = state.tasks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["renamed", "second"]);
}

#[test]
fn task_removed_drops_only_that_task() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::TaskUpserted(task(1, "a")),
            Event::TaskUpserted(task(2, "b")),
            Event::TaskRemoved(TaskId(1)),
        ],
    );
    assert_eq!(state.tasks, vec![task(2, "b")]);
}

// Agents

#[test]
fn agent_upsert_inserts_then_replaces_by_id() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::AgentUpserted(agent("s1", AgentStatus::Working)),
            Event::AgentUpserted(agent("s2", AgentStatus::Idle)),
            Event::AgentUpserted(agent("s1", AgentStatus::NeedsYou)),
        ],
    );
    assert_eq!(
        state.agents,
        vec![
            agent("s1", AgentStatus::NeedsYou),
            agent("s2", AgentStatus::Idle)
        ]
    );
}

#[test]
fn agent_removed_drops_the_agent_and_its_conversation() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::AgentUpserted(agent("s1", AgentStatus::Idle)),
            Event::AgentUpserted(agent("s2", AgentStatus::Idle)),
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(1, EntryKind::User, "hi"),
            },
            Event::ConversationAppended {
                agent: id("s2"),
                entry: entry(1, EntryKind::User, "hello"),
            },
            Event::AgentRemoved(id("s1")),
        ],
    );
    assert_eq!(state.agents, vec![agent("s2", AgentStatus::Idle)]);
    assert!(!state.conversations.contains_key(&id("s1")));
    assert!(state.conversations.contains_key(&id("s2")));
}

// Worktree view

#[test]
fn worktree_info_is_replaced_per_slice_and_goes_with_it() {
    let info = |agent: &str, staged: u32| WorktreeInfo {
        agent: id(agent),
        path: format!("/w/{agent}").into(),
        branch: format!("dovetail/{agent}"),
        base: "main".into(),
        sync: None,
        changes: Some(Changes {
            staged,
            ..Changes::default()
        }),
        unpushed: None,
        pr: PrLookup::None,
        setup: WorktreeSetup::None,
        checked_ms: 1_000,
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::AgentUpserted(agent("s1", AgentStatus::Idle)),
            Event::AgentUpserted(agent("s2", AgentStatus::Idle)),
            Event::WorktreeUpserted(info("s1", 1)),
            Event::WorktreeUpserted(info("s2", 1)),
            Event::WorktreeUpserted(info("s1", 4)),
        ],
    );
    assert_eq!(state.worktrees, vec![info("s1", 4), info("s2", 1)]);
    run(&mut state, vec![Event::AgentRemoved(id("s1"))]);
    assert_eq!(state.worktrees, vec![info("s2", 1)]);
}

// To-do items

#[test]
fn todos_replaced_rewrites_only_that_list() {
    let item = |list: &str, id: &str| TodoItem {
        list: TodoListId(list.into()),
        id: id.into(),
        subject: format!("{list}/{id}"),
        status: TodoStatus::Pending,
        blocked_by: vec![],
        created_by: None,
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::TodosReplaced {
                list: TodoListId("a".into()),
                items: vec![item("a", "1"), item("a", "2")],
            },
            Event::TodosReplaced {
                list: TodoListId("b".into()),
                items: vec![item("b", "1")],
            },
            Event::TodosReplaced {
                list: TodoListId("a".into()),
                items: vec![item("a", "3")],
            },
        ],
    );
    let subjects: Vec<_> = state.todos.iter().map(|t| t.subject.as_str()).collect();
    assert_eq!(subjects, ["b/1", "a/3"]);
}

// Inbox

#[test]
fn inbox_upserted_inserts_then_replaces_by_id() {
    let mut changed = question(1, "s1");
    changed.kind = InboxKind::Error {
        text: "crashed".into(),
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::InboxUpserted(question(1, "s1")),
            Event::InboxUpserted(question(2, "s2")),
            Event::InboxUpserted(changed.clone()),
        ],
    );
    assert_eq!(state.inbox, vec![changed, question(2, "s2")]);
}

#[test]
fn inbox_resolved_removes_the_item() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::InboxUpserted(question(1, "s1")),
            Event::InboxUpserted(question(2, "s1")),
            Event::InboxResolved { id: InboxId(1) },
        ],
    );
    assert_eq!(state.inbox, vec![question(2, "s1")]);
}

// Activity and messages

#[test]
fn activity_keeps_the_newest_500() {
    let mut state = Snapshot::default();
    let events = (1..=501)
        .map(|n| {
            Event::ActivityAdded(ActivityItem {
                id: n,
                at_ms: 0,
                agent: None,
                kind: ActivityKind::Ci,
                text: format!("ci {n}"),
            })
        })
        .collect();
    run(&mut state, events);
    assert_eq!(state.activity.len(), 500);
    assert_eq!(state.activity.first().map(|a| a.id), Some(2));
    assert_eq!(state.activity.last().map(|a| a.id), Some(501));
}

#[test]
fn messages_keep_the_newest_500() {
    let mut state = Snapshot::default();
    let events = (1..=501)
        .map(|n| {
            Event::MessageAdded(AgentMessage {
                id: n,
                at_ms: 0,
                from: id("coordinator"),
                to: id("s1"),
                text: format!("msg {n}"),
            })
        })
        .collect();
    run(&mut state, events);
    assert_eq!(state.messages.len(), 500);
    assert_eq!(state.messages.first().map(|m| m.id), Some(2));
    assert_eq!(state.messages.last().map(|m| m.id), Some(501));
}

// Conversations

#[test]
fn conversation_appended_creates_the_tail_then_appends() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(1, EntryKind::User, "hi"),
            },
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(2, EntryKind::Assistant, "hello"),
            },
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")],
        ConversationTail {
            entries: vec![
                entry(1, EntryKind::User, "hi"),
                entry(2, EntryKind::Assistant, "hello")
            ],
            has_more: false,
        }
    );
}

#[test]
fn conversation_appended_with_a_known_id_replaces_the_entry() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(1, EntryKind::Assistant, "draft"),
            },
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(1, EntryKind::Assistant, "final text"),
            },
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")].entries,
        vec![entry(1, EntryKind::Assistant, "final text")]
    );
}

#[test]
fn conversation_keeps_the_newest_2000_and_marks_more() {
    let mut state = Snapshot::default();
    let n = CONVERSATION_KEEP as u64 + 1;
    let events = (1..=n)
        .map(|n| Event::ConversationAppended {
            agent: id("s1"),
            entry: entry(n, EntryKind::User, "x"),
        })
        .collect();
    run(&mut state, events);
    let tail = &state.conversations[&id("s1")];
    assert_eq!(tail.entries.len(), CONVERSATION_KEEP);
    assert_eq!(tail.entries.first().map(|e| e.id), Some(EntryId(2)));
    assert!(tail.has_more);
}

#[test]
fn paged_history_survives_the_next_message() {
    // Plan section 7, decided 2026-11-03: the 200-entry tail is for snapshots; what a client
    // pages in stays (up to CONVERSATION_KEEP) when new entries arrive.
    let mut state = Snapshot::default();
    let tail: Vec<Event> = (300..500)
        .map(|n| Event::ConversationAppended {
            agent: id("s1"),
            entry: entry(n, EntryKind::User, "x"),
        })
        .collect();
    run(&mut state, tail);
    let older = (100..300)
        .map(|n| entry(n, EntryKind::User, "old"))
        .collect();
    run(
        &mut state,
        vec![
            Event::ConversationPage {
                agent: id("s1"),
                entries: older,
                has_more: false,
            },
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(500, EntryKind::User, "new"),
            },
        ],
    );
    let tail = &state.conversations[&id("s1")];
    assert_eq!(tail.entries.len(), 401);
    assert_eq!(tail.entries.first().map(|e| e.id), Some(EntryId(100)));
}

#[test]
fn a_trimmed_snapshot_carries_only_the_tail() {
    let mut state = Snapshot::default();
    let events = (1..=250)
        .map(|n| Event::ConversationAppended {
            agent: id("s1"),
            entry: entry(n, EntryKind::User, "x"),
        })
        .collect();
    run(&mut state, events);
    let sent = state.trimmed();
    let tail = &sent.conversations[&id("s1")];
    assert_eq!(tail.entries.len(), CONVERSATION_TAIL);
    assert_eq!(tail.entries.first().map(|e| e.id), Some(EntryId(51)));
    assert!(tail.has_more);
    assert_eq!(
        state.conversations[&id("s1")].entries.len(),
        250,
        "the original is untouched"
    );
}

#[test]
fn a_decision_is_added_once() {
    let mut state = Snapshot::default();
    let d = Decision {
        id: DecisionId(1),
        task: TaskId(1),
        at_ms: 0,
        by: DecisionAuthor::User,
        text: "x".into(),
        why: None,
        alternatives: vec![],
        supersedes: None,
        evidence: Vec::new(),
    };
    run(
        &mut state,
        vec![Event::DecisionAdded(d.clone()), Event::DecisionAdded(d)],
    );
    assert_eq!(state.decisions.len(), 1);
}

#[test]
fn assistant_delta_extends_its_entry() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(1, EntryKind::Assistant, ""),
            },
            Event::AssistantDelta {
                agent: id("s1"),
                entry: EntryId(1),
                text: "Hel".into(),
            },
            Event::AssistantDelta {
                agent: id("s1"),
                entry: EntryId(1),
                text: "lo **there**".into(),
            },
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")].entries,
        vec![entry(1, EntryKind::Assistant, "Hello **there**")]
    );
}

#[test]
fn assistant_delta_for_an_unknown_entry_starts_one_at_snapshot_time() {
    let mut state = Snapshot {
        now_ms: 5_000,
        ..Snapshot::default()
    };
    run(
        &mut state,
        vec![Event::AssistantDelta {
            agent: id("s1"),
            entry: EntryId(4),
            text: "Hi".into(),
        }],
    );
    assert_eq!(
        state.conversations[&id("s1")].entries,
        vec![ConversationEntry {
            id: EntryId(4),
            at_ms: 5_000,
            kind: EntryKind::Assistant,
            text: "Hi".into(),
            attachments: Vec::new(),
            effort: None,
        }]
    );
}

#[test]
fn conversation_page_puts_older_entries_before_the_tail() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(3, EntryKind::User, "new"),
            },
            Event::ConversationPage {
                agent: id("s1"),
                entries: vec![
                    entry(1, EntryKind::User, "old"),
                    entry(2, EntryKind::Assistant, "older reply"),
                ],
                has_more: false,
            },
        ],
    );
    let ids: Vec<_> = state.conversations[&id("s1")]
        .entries
        .iter()
        .map(|e| e.id.0)
        .collect();
    assert_eq!(ids, [1, 2, 3]);
    assert!(!state.conversations[&id("s1")].has_more);
}

#[test]
fn conversation_page_skips_entries_it_already_has() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::ConversationAppended {
                agent: id("s1"),
                entry: entry(2, EntryKind::User, "kept"),
            },
            Event::ConversationPage {
                agent: id("s1"),
                entries: vec![
                    entry(1, EntryKind::User, "old"),
                    entry(2, EntryKind::User, "duplicate"),
                ],
                has_more: true,
            },
        ],
    );
    assert_eq!(
        state.conversations[&id("s1")],
        ConversationTail {
            entries: vec![
                entry(1, EntryKind::User, "old"),
                entry(2, EntryKind::User, "kept")
            ],
            has_more: true,
        }
    );
}

// Totals, warnings, failures

#[test]
fn totals_changed_replaces_totals() {
    let totals = Totals {
        memory_mb: 640,
        needs_you: 3,
        cost_usd: 1.5,
    };
    let mut state = Snapshot::default();
    run(&mut state, vec![Event::TotalsChanged(totals.clone())]);
    assert_eq!(state.totals, totals);
}

#[test]
fn warnings_are_set_by_id_and_cleared() {
    let warning = |id: &str, text: &str| Warning {
        id: WarningId(id.into()),
        text: text.into(),
    };
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::WarningSet(warning("mode", "coordinator is not in auto mode")),
            Event::WarningSet(warning("cgroup", "memory caps unavailable")),
            Event::WarningSet(warning("mode", "slice-a is not in auto mode")),
            Event::WarningCleared(WarningId("cgroup".into())),
        ],
    );
    assert_eq!(
        state.warnings,
        vec![warning("mode", "slice-a is not in auto mode")]
    );
}

#[test]
fn command_failed_only_advances_seq() {
    let mut state = Snapshot {
        seq: 4,
        ..Snapshot::default()
    };
    let mut expected = state.clone();
    expected.seq = 5;
    run(
        &mut state,
        vec![Event::CommandFailed {
            message: "no such agent".into(),
        }],
    );
    assert_eq!(state, expected);
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

#[test]
fn effort_upsert_inserts_then_replaces_by_id() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::EffortUpserted(effort(1, "unsorted")),
            Event::EffortUpserted(effort(2, "guest-checkout")),
        ],
    );
    assert_eq!(state.efforts.len(), 2);

    // A rename, a new card and a claimed criterion all arrive as the whole effort.
    let mut changed = effort(2, "guest-pay");
    changed.where_was_i.next_step = "Order email".into();
    changed.criteria.push(Criterion {
        id: CriterionId(5),
        effort: EffortId(2),
        text: "Pay as guest end to end".into(),
        state: CriterionState::Claimed,
        evidence: Some("PR #41".into()),
    });
    run(&mut state, vec![Event::EffortUpserted(changed.clone())]);
    assert_eq!(state.efforts.len(), 2);
    assert_eq!(state.efforts[1], changed);
    assert_eq!(state.efforts[0].name, "unsorted");
}

#[test]
fn a_moved_task_arrives_as_a_task_upsert() {
    let mut state = Snapshot::default();
    run(&mut state, vec![Event::TaskUpserted(task(1, "Guest form"))]);
    assert_eq!(state.tasks[0].effort, UNSORTED_EFFORT);

    let mut moved = task(1, "Guest form");
    moved.effort = EffortId(2);
    moved.criterion = Some(CriterionId(5));
    run(&mut state, vec![Event::TaskUpserted(moved)]);
    assert_eq!(state.tasks.len(), 1);
    assert_eq!(state.tasks[0].effort, EffortId(2));
    assert_eq!(state.tasks[0].criterion, Some(CriterionId(5)));
}

#[test]
fn a_tagged_entry_keeps_its_tag_through_append_and_streaming() {
    let mut state = Snapshot::default();
    let mut you = entry(1, EntryKind::User, "Add Paddle");
    you.effort = Some(EffortId(2));
    let mut reply = entry(2, EntryKind::Assistant, "");
    reply.effort = Some(EffortId(2));
    run(
        &mut state,
        vec![
            Event::ConversationAppended {
                agent: id("coordinator"),
                entry: you,
            },
            Event::ConversationAppended {
                agent: id("coordinator"),
                entry: reply,
            },
            Event::AssistantDelta {
                agent: id("coordinator"),
                entry: EntryId(2),
                text: "Added.".into(),
            },
        ],
    );
    let entries = &state.conversations[&id("coordinator")].entries;
    assert_eq!(entries[0].effort, Some(EffortId(2)));
    assert_eq!(entries[1].effort, Some(EffortId(2)));
    assert_eq!(entries[1].text, "Added.");
}

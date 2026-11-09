//! `MockCore`: deterministic, and its stream always folds into a consistent state.

use std::time::Duration;

use dovetail_api::*;
use dovetail_mock::{MockConfig, MockCore, spawn};

fn config(seed: u64) -> MockConfig {
    MockConfig {
        seed,
        ..MockConfig::default()
    }
}

/// The first `n` scheduled messages after the opening ones, as JSON lines with their delays.
fn stream(seed: u64, n: usize) -> Vec<String> {
    let mut core = MockCore::new(config(seed));
    let mut lines: Vec<String> = core
        .connect()
        .iter()
        .map(|m| serde_json::to_string(m).expect("serialises"))
        .collect();
    for _ in 0..n {
        let next = core.next_message().expect("the mock runs until quit");
        lines.push(format!(
            "+{} {}",
            next.delay_ms,
            serde_json::to_string(&next.msg).expect("serialises")
        ));
    }
    lines
}

#[test]
fn same_seed_gives_the_same_stream() {
    assert_eq!(stream(7, 2_000), stream(7, 2_000));
}

#[test]
fn different_seeds_give_different_streams() {
    assert_ne!(stream(7, 500), stream(8, 500));
}

#[test]
fn the_opening_messages_are_hello_then_snapshot() {
    let core = MockCore::new(config(1));
    let opening = core.connect();
    assert_eq!(
        opening[0],
        ServerMsg::Hello {
            protocol: PROTOCOL_VERSION
        }
    );
    assert!(matches!(&opening[1], ServerMsg::Snapshot(s) if s.seq == 0));
}

/// A command a user might plausibly send, given the client's current state.
fn some_command(state: &Snapshot, round: usize) -> Option<Command> {
    let slice = state
        .agents
        .iter()
        .find(|a| a.kind == AgentKind::Slice)
        .map(|a| a.id.clone());
    let task = state.tasks.first().map(|t| t.id);
    match round % 7 {
        0 => state.inbox.first().map(|item| match item.kind {
            InboxKind::Permission { .. } => Command::AnswerPermission {
                item: item.id,
                answer: PermissionAnswer::Allow,
            },
            InboxKind::Question { .. } => Command::AnswerQuestion {
                item: item.id,
                text: "amber".into(),
            },
            _ => Command::DismissInbox { item: item.id },
        }),
        1 => Some(Command::Prompt {
            agent: AgentId("coordinator".into()),
            text: "How are the slices doing?".into(),
            attachments: Vec::new(),
        }),
        2 => slice.map(|agent| Command::Interrupt { agent }),
        3 => Some(Command::CreateTask(NewTask {
            title: format!("follow-up {round}"),
            p_level: Some(PLevel::P2),
            bucket: Some(Bucket::Next),
            notes: String::new(),
            follow_up_of: task,
            pr: None,
            related: vec![],
        })),
        4 => task.map(|id| Command::ReorderTask { id, delta: 1 }),
        5 => slice.map(|agent| Command::CyclePermissionMode { agent }),
        _ => Some(Command::NewSlice {
            task,
            name: format!("slice-{round}"),
            brief: "Do the thing.".into(),
        }),
    }
}

fn assert_consistent(state: &Snapshot) {
    let exists = |id: &AgentId| state.agents.iter().any(|a| &a.id == id);
    for item in &state.inbox {
        assert!(exists(&item.agent), "inbox item {:?} has no agent", item.id);
    }
    for agent in &state.agents {
        if let Some(parent) = &agent.parent {
            assert!(exists(parent), "{} has no parent {}", agent.id.0, parent.0);
        }
    }
    for agent in state.conversations.keys() {
        assert!(exists(agent), "conversation for missing agent {}", agent.0);
    }
    let finished_turns = |agent: &AgentId| {
        state
            .inbox
            .iter()
            .filter(|i| &i.agent == agent && matches!(i.kind, InboxKind::FinishedTurn { .. }))
            .count()
    };
    for agent in &state.agents {
        assert!(
            finished_turns(&agent.id) <= 1,
            "{} has two finished turns",
            agent.id.0
        );
    }
}

#[test]
fn the_stream_replays_through_apply_with_no_gaps_and_stays_consistent() {
    let mut core = MockCore::new(config(3));
    let ServerMsg::Snapshot(mut client) = core.connect().remove(1) else {
        panic!("second opening message is the snapshot");
    };
    for round in 0..3_000 {
        if round % 40 == 39
            && let Some(command) = some_command(&client, round)
        {
            core.handle(command);
        }
        let msg = core.next_message().expect("the mock runs until quit").msg;
        deliver(&mut client, msg);
    }
    // Deliver what the core has already produced, then the client matches it exactly, except
    // for the core's clock, which events do not carry.
    while core.peek_delay() == 0 {
        let msg = core.pop();
        deliver(&mut client, msg);
    }
    let mut expected = core.state().clone();
    expected.now_ms = client.now_ms;
    assert_eq!(client, expected);
}

/// Folds one message into the client the way the UI will, checking it on the way.
fn deliver(client: &mut Snapshot, msg: ServerMsg) {
    match msg {
        ServerMsg::Event { seq, event } => {
            if let Event::AssistantDelta { agent, entry, .. } = &event {
                let announced = client
                    .conversations
                    .get(agent)
                    .is_some_and(|tail| tail.entries.iter().any(|e| &e.id == entry));
                assert!(announced, "delta for an entry the mock never announced");
            }
            let totals_changed = matches!(event, Event::TotalsChanged(_));
            assert_eq!(apply(client, seq, event), ApplyResult::Applied);
            // Totals follow the change that caused them, so check them when they arrive.
            if totals_changed {
                assert_eq!(client.totals.needs_you as usize, client.inbox.len());
            }
        }
        ServerMsg::Snapshot(snapshot) => *client = snapshot,
        ServerMsg::Hello { .. } => panic!("hello mid-stream"),
    }
    assert_consistent(client);
}

#[test]
fn answering_a_permission_resolves_it_and_frees_the_agent() {
    let mut core = MockCore::new(config(1));
    let item = core
        .state()
        .inbox
        .iter()
        .find(|i| matches!(i.kind, InboxKind::Permission { .. }))
        .map(|i| (i.id, i.agent.clone()))
        .expect("the starting world has a permission request");
    core.handle(Command::AnswerPermission {
        item: item.0,
        answer: PermissionAnswer::Allow,
    });
    let first = core.next_message().expect("runs").msg;
    assert!(matches!(
        first,
        ServerMsg::Event { event: Event::InboxResolved { id }, .. } if id == item.0
    ));
    let agent = core
        .state()
        .agents
        .iter()
        .find(|a| a.id == item.1)
        .expect("agent exists");
    assert_eq!(agent.status, AgentStatus::Idle);
}

#[test]
fn a_command_on_a_missing_agent_fails_visibly() {
    let mut core = MockCore::new(config(1));
    core.handle(Command::Interrupt {
        agent: AgentId("nobody".into()),
    });
    assert!(matches!(
        core.next_message().expect("runs").msg,
        ServerMsg::Event {
            event: Event::CommandFailed { .. },
            ..
        }
    ));
}

#[test]
fn spawned_core_opens_with_hello_and_stops_on_quit() {
    let link = spawn(config(1), 1_000.0);
    let hello = link
        .messages
        .recv_timeout(Duration::from_secs(5))
        .expect("hello");
    assert!(matches!(hello, ServerMsg::Hello { .. }));
    let snapshot = link
        .messages
        .recv_timeout(Duration::from_secs(5))
        .expect("snapshot");
    assert!(matches!(snapshot, ServerMsg::Snapshot(_)));
    link.commands
        .send(Command::Quit {
            end_sessions: false,
        })
        .expect("core is listening");
    // After quit the thread ends and the channel closes, once anything in flight drains.
    let closed = (0..10_000).any(|_| link.messages.recv_timeout(Duration::from_secs(5)).is_err());
    assert!(closed, "core kept running after quit");
}

#[test]
fn starting_a_slice_on_a_todo_task_marks_it_in_progress() {
    let mut core = MockCore::new(config(1));
    let todo = core
        .state()
        .tasks
        .iter()
        .find(|t| t.status == TaskStatus::Todo)
        .map(|t| t.id)
        .expect("the starting world has a to-do task");
    core.handle(Command::NewSlice {
        task: Some(todo),
        name: "reading-column".into(),
        brief: "Build the reading column.".into(),
    });
    let status = |core: &MockCore| {
        core.state()
            .tasks
            .iter()
            .find(|t| t.id == todo)
            .map(|t| t.status)
    };
    assert_eq!(status(&core), Some(TaskStatus::InProgress));
    let bucket = core
        .state()
        .tasks
        .iter()
        .find(|t| t.id == todo)
        .and_then(|t| t.bucket);
    assert_eq!(bucket, Some(Bucket::Now));
    core.handle(Command::CloseSlice {
        agent: AgentId("reading-column".into()),
    });
    assert_eq!(status(&core), Some(TaskStatus::InProgress));
}

/// The kind of an event as it appears on the wire: the tag of its externally tagged JSON.
fn kind_of(event: &Event) -> String {
    match serde_json::to_value(event).expect("serialises") {
        serde_json::Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
        serde_json::Value::String(s) => s,
        other => panic!("unexpected event shape {other}"),
    }
}

#[test]
fn every_event_kind_the_mock_can_make_unprompted_shows_up_within_fifteen_virtual_minutes() {
    // M1 retro: two dump bugs (stalled activity, no background subagents) slipped past the
    // consistency tests. A mock that never produces a kind starves the UI of it.
    // Excluded: kinds only a command can cause (`conversation_page`, `command_failed`) and
    // `task_removed`, which nothing in v1 produces (tasks are archived, never deleted).
    let expected = [
        "task_upserted",
        "agent_upserted",
        "agent_removed",
        "todos_replaced",
        "inbox_upserted",
        "inbox_resolved",
        "activity_added",
        "message_added",
        "conversation_appended",
        "assistant_delta",
        "totals_changed",
        "warning_set",
        "warning_cleared",
        "decision_added",
    ];
    const WITHIN: usize = 800;
    let mut core = MockCore::new(config(1));
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for n in 1..=WITHIN {
        if let ServerMsg::Event { event, .. } = core.next_message().expect("runs").msg {
            seen.entry(kind_of(&event)).or_insert(n);
        }
    }
    let missing: Vec<_> = expected
        .iter()
        .filter(|k| !seen.contains_key(**k))
        .collect();
    assert!(
        missing.is_empty(),
        "no {missing:?} within {WITHIN} messages; first seen at {seen:?}"
    );
}

#[test]
fn background_and_foreground_subagents_and_several_inbox_kinds_appear_in_a_longer_run() {
    let mut core = MockCore::new(config(1));
    let mut background = false;
    let mut foreground = false;
    let mut kinds = std::collections::HashSet::new();
    for _ in 0..1_500 {
        if let ServerMsg::Event { event, .. } = core.next_message().expect("runs").msg {
            match &event {
                Event::AgentUpserted(a) => match a.kind {
                    AgentKind::Subagent { background: true } => background = true,
                    AgentKind::Subagent { background: false } => foreground = true,
                    _ => {}
                },
                Event::InboxUpserted(i) => {
                    kinds.insert(std::mem::discriminant(&i.kind));
                }
                _ => {}
            }
        }
    }
    assert!(background, "no background subagent in 1,500 messages");
    assert!(foreground, "no foreground subagent in 1,500 messages");
    assert!(
        kinds.len() >= 3,
        "only {} inbox kinds appeared",
        kinds.len()
    );
}

#[test]
fn relating_tasks_is_two_way_and_unrelating_undoes_it() {
    let mut core = MockCore::new(config(1));
    core.handle(Command::UpdateTask {
        id: TaskId(3),
        patch: vec![TaskChange::Relate(TaskId(4))],
    });
    let related = |core: &MockCore, id: i64| {
        core.state()
            .tasks
            .iter()
            .find(|t| t.id == TaskId(id))
            .map(|t| t.related.clone())
            .unwrap_or_default()
    };
    assert!(related(&core, 3).contains(&TaskId(4)));
    assert!(related(&core, 4).contains(&TaskId(3)));
    core.handle(Command::UpdateTask {
        id: TaskId(4),
        patch: vec![TaskChange::Unrelate(TaskId(3))],
    });
    assert!(!related(&core, 3).contains(&TaskId(4)));
    assert!(!related(&core, 4).contains(&TaskId(3)));
}

#[test]
fn a_logged_decision_lands_on_its_task_and_a_snapshot_is_trimmed() {
    let mut core = MockCore::new(config(1));
    core.handle(Command::LogDecision(NewDecision {
        task: TaskId(2),
        text: "Use one picker".into(),
        why: None,
        alternatives: vec![],
        supersedes: None,
    }));
    let d = core.state().decisions.last().cloned().expect("a decision");
    assert_eq!((d.task, d.by), (TaskId(2), DecisionAuthor::User));
    let ServerMsg::Snapshot(sent) = core.connect().remove(1) else {
        panic!("snapshot");
    };
    assert!(
        sent.conversations
            .values()
            .all(|t| t.entries.len() <= CONVERSATION_TAIL)
    );
}

#[test]
fn refreshing_worktrees_fills_one_entry_per_slice_worktree() {
    let mut core = MockCore::new(config(1));
    assert!(core.state().worktrees.is_empty());
    core.handle(Command::RefreshWorktrees);
    let slices = core
        .state()
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Slice && a.worktree.is_some())
        .count();
    assert!(slices > 0);
    assert_eq!(core.state().worktrees.len(), slices);
    for info in &core.state().worktrees {
        let agent = core.state().agents.iter().find(|a| a.id == info.agent);
        assert_eq!(
            agent.and_then(|a| a.worktree.as_ref()).map(|w| &w.branch),
            Some(&info.branch)
        );
    }
}

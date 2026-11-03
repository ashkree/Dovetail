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
        .find(|a| a.role == Role::Slice)
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
        1 => Some(Command::SendMessage {
            agent: AgentId("coordinator".into()),
            text: "How are the slices doing?".into(),
        }),
        2 => slice.map(|agent| Command::Interrupt { agent }),
        3 => Some(Command::CreateTask(NewTask {
            title: format!("follow-up {round}"),
            p_level: Some(PLevel::P2),
            bucket: Some(Bucket::Next),
            notes: String::new(),
            follow_up_of: task,
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

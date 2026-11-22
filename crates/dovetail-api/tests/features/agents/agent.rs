//! Tests for the `agent` feature.

use crate::common::{Case, agent, entry, id, roundtrip, run, subagent, todo};
use dovetail_api::agent::{
    Agent, AgentCommand, AgentEvent, AgentId, AgentKind, AgentStatus, PermissionMode, SessionLock,
    SlashCommand, TodoItem, TodoListId, TodoStatus,
};
use dovetail_api::app::AppEvent;
use dovetail_api::conversation::EntryKind;
use dovetail_api::inbox::{InboxEvent, InboxId, InboxItem, InboxKind};
use dovetail_api::status::{Warning, WarningId};
use dovetail_api::task::{TaskEvent, TaskId};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn sample_agent(name: &str) -> Agent {
    Agent {
        id: AgentId(name.into()),
        name: name.into(),
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

pub fn cases() -> Vec<Case> {
    let mut with_agent = Snapshot {
        seq: 3,
        agents: vec![sample_agent("slice-a"), sample_agent("slice-b")],
        ..Snapshot::default()
    };
    with_agent.conversations.insert(
        AgentId("slice-a".into()),
        dovetail_api::conversation::ConversationTail {
            entries: vec![entry(1, EntryKind::User, "go")],
            has_more: false,
        },
    );

    let a = AgentId("slice-a".into());
    let mut renamed = sample_agent("slice-a");
    renamed.status_line = "testing".into();
    let mut snapshot_for_agent = Snapshot {
        seq: 0,
        agents: vec![sample_agent("slice-a")],
        todos: vec![TodoItem {
            list: TodoListId("l1".into()),
            id: "1".into(),
            subject: "do 1".into(),
            status: TodoStatus::Pending,
            blocked_by: vec![],
            created_by: None,
        }],
        ..Snapshot::default()
    };
    snapshot_for_agent.commands.insert(a.clone(), vec![]);

    vec![
        Case::new(
            "agent",
            snapshot_for_agent,
            vec![
                (1, Event::Agent(AgentEvent::Upserted(renamed))),
                (
                    2,
                    Event::Agent(AgentEvent::TodosReplaced {
                        list: TodoListId("l1".into()),
                        items: vec![todo()],
                    }),
                ),
                (
                    3,
                    Event::Agent(AgentEvent::CommandsChanged {
                        agent: a.clone(),
                        commands: vec![SlashCommand {
                            name: "review".into(),
                            description: "review it".into(),
                            argument_hint: String::new(),
                        }],
                    }),
                ),
                (4, Event::Agent(AgentEvent::Removed(a))),
            ],
        ),
        Case::new(
            "agents_inbox_and_order",
            with_agent,
            vec![
                (
                    4,
                    Event::Inbox(InboxEvent::Upserted(InboxItem {
                        id: InboxId(1),
                        agent: AgentId("slice-a".into()),
                        at_ms: 7,
                        kind: InboxKind::Question {
                            text: "which one?".into(),
                        },
                    })),
                ),
                // A gap and a repeat change nothing.
                (9, Event::Task(TaskEvent::Removed(TaskId(1)))),
                (4, Event::Task(TaskEvent::Removed(TaskId(1)))),
                (
                    5,
                    Event::Agent(AgentEvent::Removed(AgentId("slice-a".into()))),
                ),
                (6, Event::Inbox(InboxEvent::Resolved { id: InboxId(1) })),
                (
                    7,
                    Event::Status(dovetail_api::status::StatusEvent::WarningSet(Warning {
                        id: WarningId("w".into()),
                        text: "careful".into(),
                    })),
                ),
                (
                    8,
                    Event::App(AppEvent::CommandFailed {
                        message: "nothing stored".into(),
                    }),
                ),
            ],
        ),
    ]
}

#[test]
fn agent_commands_and_events_roundtrip() {
    roundtrip(&AgentKind::Tangent);
    let events = vec![
        Event::Agent(AgentEvent::Upserted(subagent())),
        Event::Agent(AgentEvent::Removed(id("sub-1"))),
        Event::Agent(AgentEvent::TodosReplaced {
            list: TodoListId("list-a".into()),
            items: vec![todo()],
        }),
        Event::Agent(AgentEvent::CommandsChanged {
            agent: id("slice-a"),
            commands: vec![SlashCommand {
                name: "review".into(),
                description: "review it".into(),
                argument_hint: String::new(),
            }],
        }),
    ];
    for (n, event) in events.into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Agent(AgentCommand::Interrupt {
            agent: id("slice-a"),
        }),
        Command::Agent(AgentCommand::CyclePermissionMode {
            agent: id("slice-a"),
        }),
        Command::Agent(AgentCommand::NewSlice {
            task: Some(TaskId(3)),
            name: "tree-view".into(),
            brief: "Build the tree".into(),
            repo: Some("shop-web".into()),
        }),
        Command::Agent(AgentCommand::CloseSlice {
            agent: id("slice-a"),
        }),
        Command::Agent(AgentCommand::NewSideChat {
            question: "how does routing work?".into(),
        }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn agent_upsert_inserts_then_replaces_by_id() {
    let mut state = Snapshot::default();
    run(
        &mut state,
        vec![
            Event::Agent(AgentEvent::Upserted(agent("s1", AgentStatus::Working))),
            Event::Agent(AgentEvent::Upserted(agent("s2", AgentStatus::Idle))),
            Event::Agent(AgentEvent::Upserted(agent("s1", AgentStatus::NeedsYou))),
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
            Event::Agent(AgentEvent::Upserted(agent("s1", AgentStatus::Idle))),
            Event::Agent(AgentEvent::Upserted(agent("s2", AgentStatus::Idle))),
            Event::Conversation(dovetail_api::conversation::ConversationEvent::Appended {
                agent: id("s1"),
                entry: entry(1, EntryKind::User, "hi"),
            }),
            Event::Conversation(dovetail_api::conversation::ConversationEvent::Appended {
                agent: id("s2"),
                entry: entry(1, EntryKind::User, "hello"),
            }),
            Event::Agent(AgentEvent::Removed(id("s1"))),
        ],
    );
    assert_eq!(state.agents, vec![agent("s2", AgentStatus::Idle)]);
    assert!(!state.conversations.contains_key(&id("s1")));
    assert!(state.conversations.contains_key(&id("s2")));
}

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
            Event::Agent(AgentEvent::TodosReplaced {
                list: TodoListId("a".into()),
                items: vec![item("a", "1"), item("a", "2")],
            }),
            Event::Agent(AgentEvent::TodosReplaced {
                list: TodoListId("b".into()),
                items: vec![item("b", "1")],
            }),
            Event::Agent(AgentEvent::TodosReplaced {
                list: TodoListId("a".into()),
                items: vec![item("a", "3")],
            }),
        ],
    );
    let subjects: Vec<_> = state.todos.iter().map(|t| t.subject.as_str()).collect();
    assert_eq!(subjects, ["b/1", "a/3"]);
}

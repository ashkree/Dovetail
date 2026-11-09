//! Every message survives JSON unchanged, so a daemon can carry the same values over a socket.

use std::collections::BTreeMap;

use dovetail_api::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let text = serde_json::to_string(value).expect("serialises");
    let back: T = serde_json::from_str(&text).expect("deserialises");
    assert_eq!(&back, value, "changed in transit: {text}");
}

fn id(s: &str) -> AgentId {
    AgentId(s.into())
}

fn task() -> Task {
    Task {
        id: TaskId(3),
        title: "Tree view".into(),
        status: TaskStatus::InProgress,
        p_level: Some(PLevel::P0),
        bucket: Some(Bucket::Later),
        order: 2,
        deps: vec![TaskId(1)],
        notes: "Use **Layout B**".into(),
        pr: Some(1231),
        follow_up_of: Some(TaskId(2)),
        coordinator_created: true,
        related: vec![],
    }
}

fn subagent() -> Agent {
    Agent {
        id: id("sub-1"),
        name: "explore".into(),
        kind: AgentKind::Subagent { background: true },
        parent: Some(id("slice-a")),
        task: Some(TaskId(3)),
        worktree: Some(WorktreeRef {
            path: "/tmp/wt/slice-a".into(),
            branch: "dovetail/slice-a".into(),
        }),
        status: AgentStatus::SettingUp,
        status_line: "reading the brief".into(),
        memory_mb: None,
        cost_usd: 0.25,
        permission_mode: PermissionMode::AcceptEdits,
        context_pct: Some(40),
        progress: Some(SubagentProgress {
            description: "find the keymap".into(),
            last_tool: Some("Grep".into()),
            tool_uses: 4,
            summary: None,
        }),
        last_activity_ms: 1_700_000_000_000,
        lock: SessionLock::TakenOver,
    }
}

fn entry() -> ConversationEntry {
    ConversationEntry {
        id: EntryId(9),
        at_ms: 5,
        kind: EntryKind::ToolSummary,
        text: "Edit src/app.rs · Bash cargo test".into(),
        attachments: vec![Attachment {
            path: "/data/dovetail/attachments/1.png".into(),
            media_type: ImageType::Png,
            size: 421_888,
        }],
    }
}

fn inbox_items() -> Vec<InboxItem> {
    let kinds = vec![
        InboxKind::Permission {
            tool: "Bash".into(),
            input: json!({"command": "cargo test", "nested": [1, null, "x"]}),
            reason: Some("network".into()),
            suggestions: vec![RuleSuggestion {
                label: "Bash(cargo test:*)".into(),
                raw: json!({"type": "addRules"}),
            }],
        },
        InboxKind::Question {
            text: "Which port?".into(),
            blocking: false,
        },
        InboxKind::FinishedTurn {
            preview: "Done with the tree".into(),
        },
        InboxKind::Error {
            text: "memory cap hit".into(),
        },
    ];
    kinds
        .into_iter()
        .enumerate()
        .map(|(n, kind)| InboxItem {
            id: InboxId(n as u64),
            agent: id("slice-a"),
            at_ms: 7,
            kind,
        })
        .collect()
}

fn todo() -> TodoItem {
    TodoItem {
        list: TodoListId("list-a".into()),
        id: "1".into(),
        subject: "write tests".into(),
        status: TodoStatus::InProgress,
        blocked_by: vec!["0".into()],
        created_by: Some(id("slice-a")),
    }
}

fn activity() -> ActivityItem {
    ActivityItem {
        id: 1,
        at_ms: 2,
        agent: Some(id("slice-a")),
        kind: ActivityKind::Pr,
        text: "pushed #1231".into(),
    }
}

fn message() -> AgentMessage {
    AgentMessage {
        id: 1,
        at_ms: 2,
        from: id("coordinator"),
        to: id("slice-a"),
        text: "rebase on main".into(),
    }
}

fn totals() -> Totals {
    Totals {
        memory_mb: 900,
        needs_you: 4,
        cost_usd: 3.75,
    }
}

fn warning() -> Warning {
    Warning {
        id: WarningId("mode".into()),
        text: "not in auto mode".into(),
    }
}

fn decision() -> Decision {
    Decision {
        id: DecisionId(4),
        task: TaskId(1),
        at_ms: 1_700_000_000_000,
        by: DecisionAuthor::Agent(id("coordinator")),
        text: "Keep tasks one level".into(),
        why: Some("Nesting changes the glossary".into()),
        alternatives: vec![Alternative {
            option: "Nest related tasks".into(),
            why_not: None,
        }],
        supersedes: Some(DecisionId(2)),
    }
}

#[test]
fn hello_and_a_full_snapshot_roundtrip() {
    let project = ProjectSummary {
        id: ProjectId("dovetail".into()),
        name: "Dovetail".into(),
        repo_path: "/home/user/src/dovetail".into(),
    };
    let snapshot = Snapshot {
        seq: 42,
        now_ms: 1_700_000_000_000,
        project: Some(project.clone()),
        projects: vec![project],
        tasks: vec![task()],
        agents: vec![subagent()],
        todos: vec![todo()],
        inbox: inbox_items(),
        activity: vec![activity()],
        messages: vec![message()],
        decisions: vec![
            decision(),
            Decision {
                by: DecisionAuthor::User,
                ..decision()
            },
        ],
        conversations: BTreeMap::from([(
            id("slice-a"),
            ConversationTail {
                entries: vec![entry()],
                has_more: true,
            },
        )]),
        totals: totals(),
        warnings: vec![warning()],
    };
    roundtrip(&ServerMsg::Hello {
        protocol: PROTOCOL_VERSION,
    });
    roundtrip(&ServerMsg::Snapshot(snapshot));
}

#[test]
fn every_event_roundtrips() {
    let events = vec![
        Event::TaskUpserted(task()),
        Event::TaskRemoved(TaskId(3)),
        Event::AgentUpserted(subagent()),
        Event::AgentRemoved(id("sub-1")),
        Event::TodosReplaced {
            list: TodoListId("list-a".into()),
            items: vec![todo()],
        },
        Event::InboxResolved { id: InboxId(1) },
        Event::ActivityAdded(activity()),
        Event::MessageAdded(message()),
        Event::ConversationAppended {
            agent: id("slice-a"),
            entry: entry(),
        },
        Event::AssistantDelta {
            agent: id("slice-a"),
            entry: EntryId(9),
            text: "more *text*\n".into(),
        },
        Event::ConversationPage {
            agent: id("slice-a"),
            entries: vec![entry()],
            has_more: false,
        },
        Event::DecisionAdded(decision()),
        Event::TotalsChanged(totals()),
        Event::WarningSet(warning()),
        Event::WarningCleared(WarningId("mode".into())),
        Event::CommandFailed {
            message: "no such agent".into(),
        },
    ]
    .into_iter()
    .chain(inbox_items().into_iter().map(Event::InboxUpserted));
    for (n, event) in events.enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }
}

#[test]
fn every_command_roundtrips() {
    let commands = vec![
        Command::Prompt {
            agent: id("coordinator"),
            text: "plan M2".into(),
            attachments: Vec::new(),
        },
        Command::Prompt {
            agent: id("coordinator"),
            text: String::new(),
            attachments: vec![Attachment {
                path: "/data/dovetail/attachments/2.webp".into(),
                media_type: ImageType::Webp,
                size: 10,
            }],
        },
        Command::Interrupt {
            agent: id("slice-a"),
        },
        Command::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::Allow,
        },
        Command::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::AllowAlways,
        },
        Command::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::Deny {
                message: Some("use the test target".into()),
            },
        },
        Command::AnswerQuestion {
            item: InboxId(2),
            text: "8080".into(),
        },
        Command::DismissInbox { item: InboxId(3) },
        Command::CyclePermissionMode {
            agent: id("slice-a"),
        },
        Command::NewSlice {
            task: Some(TaskId(3)),
            name: "tree-view".into(),
            brief: "Build the tree".into(),
        },
        Command::CloseSlice {
            agent: id("slice-a"),
        },
        Command::TakeOver {
            agent: id("slice-a"),
        },
        Command::CreateTask(NewTask {
            title: "x".into(),
            p_level: Some(PLevel::P2),
            bucket: None,
            notes: String::new(),
            follow_up_of: Some(TaskId(1)),
            pr: Some(1231),
            related: vec![TaskId(2)],
        }),
        Command::LogDecision(NewDecision {
            task: TaskId(1),
            text: "Use the mock".into(),
            why: None,
            alternatives: vec![],
            supersedes: None,
        }),
        Command::UpdateTask {
            id: TaskId(3),
            patch: vec![
                TaskChange::Title("y".into()),
                TaskChange::Status(TaskStatus::Done),
                TaskChange::PLevel(None),
                TaskChange::PLevel(Some(PLevel::P3)),
                TaskChange::Bucket(None),
                TaskChange::Bucket(Some(Bucket::Next)),
                TaskChange::Notes("n".into()),
                TaskChange::Deps(vec![TaskId(1)]),
                TaskChange::Pr(None),
                TaskChange::Pr(Some(7)),
                TaskChange::Seen,
                TaskChange::Relate(TaskId(2)),
                TaskChange::Unrelate(TaskId(2)),
            ],
        },
        Command::ArchiveTask { id: TaskId(3) },
        Command::ReorderTask {
            id: TaskId(3),
            delta: -1,
        },
        Command::RemoveWorktree {
            agent: id("slice-a"),
            confirmed: true,
        },
        Command::OpenProject {
            id: ProjectId("dovetail".into()),
        },
        Command::RegisterProject {
            path: "/home/user/src/other".into(),
        },
        Command::LoadConversation {
            agent: id("slice-a"),
            before: Some(EntryId(9)),
        },
        Command::LoadConversation {
            agent: id("slice-a"),
            before: None,
        },
        Command::Resync,
        Command::Quit {
            end_sessions: false,
        },
    ];
    for command in &commands {
        roundtrip(command);
    }
}

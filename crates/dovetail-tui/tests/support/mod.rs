//! Deterministic fixtures and helpers shared by the TUI tests. Hand-built, seeded by nothing and
//! free of clocks, so snapshots never change by accident. The world mirrors `docs/screens/ui/b.png`.

#![allow(dead_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::*;
use dovetail_tui::App;
use jiff::tz::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// 2026-11-01 10:15:00 UTC.
pub const NOW_MS: i64 = 1_793_528_100_000;
const MIN: i64 = 60_000;

pub fn agent_id(s: &str) -> AgentId {
    AgentId(s.into())
}

fn agent(id: &str, kind: AgentKind, status: AgentStatus, task: Option<i64>) -> Agent {
    Agent {
        id: agent_id(id),
        name: id.into(),
        kind,
        parent: None,
        task: task.map(TaskId),
        worktree: None,
        status,
        status_line: String::new(),
        memory_mb: Some(210),
        cost_usd: 0.0,
        permission_mode: PermissionMode::Auto,
        context_pct: Some(10),
        progress: None,
        last_activity_ms: NOW_MS,
        lock: SessionLock::Headless,
    }
}

fn task(
    id: i64,
    title: &str,
    status: TaskStatus,
    p: Option<PLevel>,
    bucket: Option<Bucket>,
) -> Task {
    Task {
        id: TaskId(id),
        title: title.into(),
        status,
        p_level: p,
        bucket,
        order: id,
        deps: vec![],
        notes: String::new(),
        pr: None,
        follow_up_of: None,
        coordinator_created: false,
    }
}

fn entry(id: u64, at: i64, kind: EntryKind, text: &str) -> ConversationEntry {
    ConversationEntry {
        id: EntryId(id),
        at_ms: at,
        kind,
        text: text.into(),
    }
}

/// The b.png world: a coordinator, one task with a slice and subagents, one waiting slice, an
/// idea and a done task, and three inbox items.
pub fn world() -> Snapshot {
    let mut coordinator = agent(
        "coordinator",
        AgentKind::Coordinator,
        AgentStatus::Working,
        None,
    );
    coordinator.memory_mb = Some(1229);
    coordinator.cost_usd = 3.10;

    let mut slice4 = agent("slice-4", AgentKind::Slice, AgentStatus::Working, Some(1));
    slice4.memory_mb = Some(412);
    slice4.cost_usd = 1.84;
    slice4.permission_mode = PermissionMode::AcceptEdits;
    slice4.worktree = Some(WorktreeRef {
        path: "/wt/slice-4".into(),
        branch: "dovetail/slice-4".into(),
    });

    let mut rebase = agent(
        "rebase",
        AgentKind::Subagent { background: false },
        AgentStatus::Done,
        Some(1),
    );
    rebase.parent = Some(agent_id("slice-4"));
    rebase.memory_mb = None;
    rebase.cost_usd = 0.21;

    let mut explore = agent(
        "explore callers",
        AgentKind::Subagent { background: true },
        AgentStatus::Working,
        Some(1),
    );
    explore.id = agent_id("explore");
    explore.parent = Some(agent_id("slice-4"));
    explore.memory_mb = None;
    explore.progress = Some(SubagentProgress {
        description: "Find callers of get_decision".into(),
        last_tool: Some("Grep".into()),
        tool_uses: 12,
        summary: None,
    });

    let mut slice7 = agent("slice-7", AgentKind::Slice, AgentStatus::NeedsYou, Some(2));
    slice7.memory_mb = Some(388);
    slice7.cost_usd = 0.92;
    slice7.permission_mode = PermissionMode::Manual;
    slice7.worktree = Some(WorktreeRef {
        path: "/wt/slice-7".into(),
        branch: "dovetail/slice-7".into(),
    });

    let mut t1 = task(
        1,
        "business surface",
        TaskStatus::InProgress,
        Some(PLevel::P1),
        Some(Bucket::Now),
    );
    t1.pr = Some(1231);
    let t2 = task(
        2,
        "get_decision 500",
        TaskStatus::InProgress,
        Some(PLevel::P0),
        Some(Bucket::Now),
    );
    let t3 = task(
        3,
        "idea: eval key flow",
        TaskStatus::Todo,
        Some(PLevel::P2),
        Some(Bucket::Next),
    );
    let t4 = task(
        4,
        "docs sweep",
        TaskStatus::Todo,
        Some(PLevel::P0),
        Some(Bucket::Later),
    );
    let t5 = task(
        5,
        "release notes",
        TaskStatus::Done,
        Some(PLevel::P2),
        Some(Bucket::Now),
    );

    let todos = vec![
        todo("1", "write route tests", TodoStatus::Completed, vec![]),
        todo(
            "2",
            "wire handler to registry",
            TodoStatus::InProgress,
            vec![],
        ),
        todo("3", "update OpenAPI spec", TodoStatus::Pending, vec!["2"]),
    ];

    let inbox = vec![
        InboxItem {
            id: InboxId(1),
            agent: agent_id("slice-7"),
            at_ms: NOW_MS,
            kind: InboxKind::Permission {
                tool: "Bash".into(),
                input: serde_json::json!({"command": "FS_EVAL_VIRTUAL_KEY=dev cargo test -p api get_decision"}),
                reason: None,
                suggestions: vec![RuleSuggestion {
                    label: "Bash(cargo test:*)".into(),
                    raw: serde_json::json!({"type": "addRules"}),
                }],
            },
        },
        InboxItem {
            id: InboxId(2),
            agent: agent_id("slice-4"),
            at_ms: NOW_MS - 3 * MIN,
            kind: InboxKind::Question {
                text: "Which route prefix should the handler use?".into(),
                blocking: false,
            },
        },
        InboxItem {
            id: InboxId(3),
            agent: agent_id("slice-4"),
            at_ms: NOW_MS - 6 * MIN,
            kind: InboxKind::FinishedTurn {
                preview: "Handler wired; tests next.".into(),
            },
        },
    ];

    let mut snap = Snapshot {
        seq: 0,
        now_ms: NOW_MS,
        project: Some(ProjectSummary {
            id: ProjectId("business".into()),
            name: "business".into(),
            repo_path: "/src/business".into(),
        }),
        tasks: vec![t1, t2, t3, t4, t5],
        agents: vec![coordinator, slice4, rebase, explore, slice7],
        todos,
        inbox,
        totals: Totals {
            memory_mb: 2150,
            needs_you: 3,
            cost_usd: 6.0,
        },
        ..Snapshot::default()
    };
    snap.conversations.insert(
        agent_id("coordinator"),
        ConversationTail {
            entries: vec![
                entry(1, NOW_MS - 7 * MIN, EntryKind::User, "Make sure slice-4 doesn't touch the 500 path."),
                entry(2, NOW_MS - 7 * MIN, EntryKind::Assistant, "Told slice-4 to stay out of src/decision.rs until slice-7 hands it back."),
                entry(3, NOW_MS - MIN, EntryKind::User, "What's blocking slice-7?"),
                entry(4, NOW_MS - MIN, EntryKind::Assistant, "It needs a key. slice-7 can only reach the eval path with FS_EVAL_VIRTUAL_KEY set."),
            ],
            has_more: false,
        },
    );
    snap
}

fn todo(id: &str, subject: &str, status: TodoStatus, blocked_by: Vec<&str>) -> TodoItem {
    TodoItem {
        list: TodoListId("slice-4".into()),
        id: id.into(),
        subject: subject.into(),
        status,
        blocked_by: blocked_by.into_iter().map(String::from).collect(),
        created_by: Some(agent_id("slice-4")),
    }
}

/// An app showing `snap`, in UTC, with its clock set.
pub fn app_with(snap: Snapshot) -> App {
    let mut app = App::new(TimeZone::UTC);
    app.set_now(NOW_MS);
    app.on_server(ServerMsg::Hello {
        protocol: PROTOCOL_VERSION,
    });
    app.on_server(ServerMsg::Snapshot(snap));
    app.take_commands();
    app
}

/// A key event for one character, as a terminal sends it.
pub fn key(c: char) -> KeyEvent {
    if c.is_ascii_uppercase() {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    } else {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
}

pub fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

pub fn esc() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

pub fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Types characters one after another, a few milliseconds apart.
pub fn type_keys(app: &mut App, keys: &str) {
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 10 * i as u64);
    }
}

/// Draws the app on a test terminal and returns the screen as text, one line per row.
pub fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

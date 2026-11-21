//! Shared helpers, builders, and replay harness for per-feature tests.

use dovetail_api::activity::{ActivityItem, ActivityKind, AgentMessage};
use dovetail_api::agent::{
    Agent, AgentId, AgentKind, PermissionMode, SessionLock, SubagentProgress, TodoItem, TodoListId,
    TodoStatus, WorktreeRef,
};
use dovetail_api::canvas::{BlockId, Card, CardId, CardKind, CardRef, RefKind};
use dovetail_api::conversation::{Attachment, ConversationEntry, EntryId, EntryKind, ImageType};
use dovetail_api::decision::{Alternative, Decision, DecisionAuthor, DecisionId};
use dovetail_api::effort::{
    Criterion, CriterionId, CriterionState, Effort, EffortId, EffortKind, EffortStatus, WhereWasI,
};
use dovetail_api::inbox::{InboxId, InboxItem, InboxKind, RuleSuggestion};
use dovetail_api::map::{
    Connection, Flow, FlowStep, Group, Lane, LaneId, Map, MapContent, MapId, MapKind, MapSummary,
    Node, NodeId, NodeKind,
};
use dovetail_api::model::ReasoningEffort;
use dovetail_api::research::{
    FindingId, FindingSummary, ResearchKind, ResearchListing, ResearchSummary, RoundSummary,
    SourceId, SourceKind, SourceSummary,
};
use dovetail_api::status::{Totals, Warning, WarningId};
use dovetail_api::tangent::TangentSummary;
use dovetail_api::task::{Bucket, PLevel, Task, TaskId, TaskStatus};
use dovetail_api::worktree::{
    Changes, PrLookup, PrState, PullRequest, SyncState, WorktreeInfo, WorktreeSetup,
};
use dovetail_api::{ApplyResult, Event, Snapshot, apply};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

pub use dovetail_api::agent::AgentStatus;

pub fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let text = serde_json::to_string(value).expect("serialises");
    let back: T = serde_json::from_str(&text).expect("deserialises");
    assert_eq!(&back, value, "changed in transit: {text}");
}

/// One replay case: a name, the starting snapshot, the numbered events in order, and whether
/// the events are expected to leave everything except `seq` unchanged.
pub struct Case {
    pub name: &'static str,
    pub snapshot: Snapshot,
    pub events: Vec<(u64, Event)>,
    pub only_advances_seq: bool,
}

impl Case {
    pub fn new(name: &'static str, snapshot: Snapshot, events: Vec<(u64, Event)>) -> Self {
        Self {
            name,
            snapshot,
            events,
            only_advances_seq: false,
        }
    }

    pub fn no_op(name: &'static str, snapshot: Snapshot, events: Vec<(u64, Event)>) -> Self {
        Self {
            name,
            snapshot,
            events,
            only_advances_seq: true,
        }
    }
}

/// Applies events with consecutive seqs, asserting each one applies.
pub fn run(state: &mut Snapshot, events: Vec<Event>) {
    for event in events {
        let seq = state.seq + 1;
        assert_eq!(apply(state, seq, event), ApplyResult::Applied);
    }
}

pub fn result_name(result: ApplyResult) -> &'static str {
    match result {
        ApplyResult::Applied => "applied",
        ApplyResult::Gap { .. } => "gap",
        ApplyResult::Stale => "stale",
    }
}

pub fn id(s: &str) -> AgentId {
    AgentId(s.into())
}

pub fn task(id: i64, title: &str) -> Task {
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

pub fn rich_task() -> Task {
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
        effort: EffortId(7),
        criterion: Some(CriterionId(3)),
    }
}

pub fn agent(id: &str, status: AgentStatus) -> Agent {
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

pub fn subagent() -> Agent {
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
            ..SubagentProgress::default()
        }),
        last_activity_ms: 1_700_000_000_000,
        lock: SessionLock::TakenOver,
        model: Some("claude-opus-5-5".into()),
        effort: Some(ReasoningEffort::Xhigh),
        subagent_tokens: 12_345,
        tool: None,
    }
}

pub fn entry(id: u64, kind: EntryKind, text: &str) -> ConversationEntry {
    ConversationEntry {
        id: EntryId(id),
        at_ms: 1_000,
        kind,
        text: text.into(),
        attachments: Vec::new(),
        effort: None,
    }
}

pub fn rich_entry() -> ConversationEntry {
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
        effort: Some(EffortId(7)),
    }
}

pub fn effort(id: i64, name: &str) -> Effort {
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

pub fn rich_effort() -> Effort {
    Effort {
        id: EffortId(7),
        name: "payment-providers".into(),
        kind: EffortKind::Research,
        question_or_goal: "Which provider fits EU cards?".into(),
        status: EffortStatus::Open,
        where_was_i: WhereWasI {
            last_done: "Fees compared".into(),
            waits_on_you: "Approve criterion 3".into(),
            next_step: "EU tax handling".into(),
        },
        criteria: vec![
            Criterion {
                id: CriterionId(3),
                effort: EffortId(7),
                text: "Fees compared for 3 providers".into(),
                state: CriterionState::Claimed,
                evidence: Some("finding 41".into()),
            },
            Criterion {
                id: CriterionId(4),
                effort: EffortId(7),
                text: "EU tax handling known".into(),
                state: CriterionState::Open,
                evidence: None,
            },
        ],
    }
}

pub fn decision() -> Decision {
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
        evidence: Vec::new(),
    }
}

pub fn question(id: u64, agent: &str) -> InboxItem {
    InboxItem {
        id: InboxId(id),
        agent: AgentId(agent.into()),
        at_ms: 1_000,
        kind: InboxKind::Question {
            text: "Which port?".into(),
        },
    }
}

pub fn inbox_items() -> Vec<InboxItem> {
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

pub fn todo() -> TodoItem {
    TodoItem {
        list: TodoListId("list-a".into()),
        id: "1".into(),
        subject: "write tests".into(),
        status: TodoStatus::InProgress,
        blocked_by: vec!["0".into()],
        created_by: Some(id("slice-a")),
    }
}

pub fn activity_item() -> ActivityItem {
    ActivityItem {
        id: 1,
        at_ms: 2,
        agent: Some(id("slice-a")),
        kind: ActivityKind::Pr,
        text: "pushed #1231".into(),
    }
}

pub fn agent_message() -> AgentMessage {
    AgentMessage {
        id: 1,
        at_ms: 2,
        from: id("coordinator"),
        to: id("slice-a"),
        text: "rebase on main".into(),
    }
}

pub fn totals() -> Totals {
    Totals {
        memory_mb: 900,
        needs_you: 4,
        cost_usd: 3.75,
    }
}

pub fn warning() -> Warning {
    Warning {
        id: WarningId("mode".into()),
        text: "not in auto mode".into(),
    }
}

/// Worktree-view entries covering every PR and setup case.
pub fn worktree_infos() -> Vec<WorktreeInfo> {
    let base = WorktreeInfo {
        agent: id("slice-a"),
        path: "/data/worktrees/a".into(),
        branch: "dovetail/a".into(),
        base: "main".into(),
        sync: Some(SyncState {
            ahead: 2,
            behind: 1,
            behind_origin: Some(3),
        }),
        changes: Some(Changes {
            staged: 1,
            unstaged: 2,
            untracked: 3,
            conflicted: 0,
        }),
        unpushed: Some(2),
        pr: PrLookup::Found(PullRequest {
            number: 7,
            state: PrState::Merged,
            url: "https://github.com/o/r/pull/7".into(),
        }),
        setup: WorktreeSetup::Done,
        checked_ms: 1_700_000_000_000,
    };
    vec![
        base.clone(),
        WorktreeInfo {
            sync: None,
            changes: None,
            unpushed: None,
            pr: PrLookup::Unknown {
                reason: "gh is not installed".into(),
            },
            setup: WorktreeSetup::Failed {
                error: Some("exit 1".into()),
            },
            ..base.clone()
        },
        WorktreeInfo {
            pr: PrLookup::None,
            setup: WorktreeSetup::Running,
            ..base.clone()
        },
        WorktreeInfo {
            setup: WorktreeSetup::None,
            ..base
        },
    ]
}

pub fn map_summary() -> MapSummary {
    MapSummary {
        id: MapId("checkout".into()),
        title: "Checkout".into(),
        kind: MapKind::Structure,
        task: Some(TaskId(3)),
        drawn_by: Some(id("slice-a")),
        version: 2,
        created_ms: 1_700_000_000_000,
        updated_ms: 1_700_000_100_000,
    }
}

pub fn sample_map() -> Map {
    let node = |id: &str| Node {
        id: NodeId(id.into()),
        kind: NodeKind::Logic,
        label: id.into(),
        description: "d".into(),
        file: Some("src/a.rs".into()),
        pseudocode: Some("x = 1".into()),
    };
    Map {
        id: MapId("checkout".into()),
        version: 2,
        content: MapContent {
            title: "Checkout".into(),
            kind: MapKind::Swimlane,
            nodes: vec![node("a"), node("b")],
            groups: vec![Group {
                parent: NodeId("a".into()),
                children: vec![NodeId("b".into())],
            }],
            connections: vec![Connection {
                from: NodeId("a".into()),
                to: NodeId("b".into()),
                label: "calls".into(),
            }],
            flows: vec![Flow {
                name: "f".into(),
                steps: vec![FlowStep {
                    node: NodeId("b".into()),
                    connection: Some(0),
                    arrow: Some("calls".into()),
                    note: "n".into(),
                    lane: Some(LaneId("l".into())),
                }],
            }],
            lanes: vec![Lane {
                id: LaneId("l".into()),
                label: "L".into(),
                node: Some(NodeId("api".into())),
            }],
        },
    }
}

pub fn research_summary() -> ResearchSummary {
    ResearchSummary {
        task: TaskId(4),
        dir: "/data/projects/p/research/4-pricing".into(),
        rounds: vec![RoundSummary {
            id: 7,
            queued: true,
            n: 1,
            brief: "per seat or flat?".into(),
            dir: "rounds/01-2026-11-11".into(),
            opened_ms: 1,
            closed_ms: None,
            findings: 2,
        }],
        findings: 3,
        quick_findings: 1,
        sources: 5,
        report_ms: None,
        kind: Some(ResearchKind::Decision),
    }
}

pub fn research_listing() -> ResearchListing {
    ResearchListing {
        findings: vec![FindingSummary {
            id: FindingId(3),
            round: Some(1),
            n: 1,
            agent: id("coordinator:toolu_1"),
            label: "fetch-page".into(),
            path: "/r/4-pricing/rounds/01-2026-11-11/findings/01-fetch-page.md".into(),
            created_ms: 2,
            sources: vec![SourceId(1)],
        }],
        sources: vec![SourceSummary {
            id: SourceId(1),
            kind: SourceKind::Web,
            locator: "https://example.com".into(),
            tool: "WebFetch".into(),
            path: Some("/r/4-pricing/sources/1.md".into()),
            is_error: false,
            fetched_ms: 2,
        }],
    }
}

pub fn tangent_summary() -> TangentSummary {
    TangentSummary {
        task: Task {
            id: TaskId(2),
            notes: "Asked from dovetail on 2026-11-11.".into(),
            bucket: None,
            ..rich_task()
        },
        research: Some(research_summary()),
    }
}

pub fn canvas_card() -> Card {
    Card {
        id: CardId(90),
        effort: EffortId(7),
        block: Some(BlockId(4)),
        kind: CardKind::Finding,
        x: 320,
        y: 180,
        reference: Some(CardRef {
            kind: RefKind::Finding,
            id: "41".into(),
        }),
        title: "Stripe fees".into(),
        text: String::new(),
        missing: false,
        added_by: "user".into(),
        created_ms: 1_700_000_000_000,
        seen: true,
    }
}

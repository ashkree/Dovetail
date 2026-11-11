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

fn research_summary() -> research::ResearchSummary {
    research::ResearchSummary {
        task: TaskId(4),
        dir: "/data/projects/p/research/4-pricing".into(),
        rounds: vec![research::RoundSummary {
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
    }
}

fn research_listing() -> research::ResearchListing {
    research::ResearchListing {
        findings: vec![research::FindingSummary {
            id: research::FindingId(3),
            round: Some(1),
            n: 1,
            agent: id("coordinator:toolu_1"),
            label: "fetch-page".into(),
            path: "/r/4-pricing/rounds/01-2026-11-11/findings/01-fetch-page.md".into(),
            created_ms: 2,
            sources: vec![research::SourceId(1)],
        }],
        sources: vec![research::SourceSummary {
            id: research::SourceId(1),
            kind: research::SourceKind::Web,
            locator: "https://example.com".into(),
            tool: "WebFetch".into(),
            path: Some("/r/4-pricing/sources/1.md".into()),
            is_error: false,
            fetched_ms: 2,
        }],
    }
}

fn map_summary() -> map::MapSummary {
    map::MapSummary {
        id: map::MapId("checkout".into()),
        title: "Checkout".into(),
        kind: map::MapKind::Structure,
        task: Some(TaskId(3)),
        drawn_by: Some(id("slice-a")),
        version: 2,
        created_ms: 1_700_000_000_000,
        updated_ms: 1_700_000_100_000,
    }
}

fn map() -> map::Map {
    use map::*;
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
            ..SubagentProgress::default()
        }),
        last_activity_ms: 1_700_000_000_000,
        lock: SessionLock::TakenOver,
        model: Some("claude-opus-5-5".into()),
        subagent_tokens: 12_345,
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
        evidence: Vec::new(),
    }
}

/// Worktree-view entries covering every PR and setup case.
fn worktree_infos() -> Vec<WorktreeInfo> {
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

#[test]
fn hello_and_a_full_snapshot_roundtrip() {
    let project = ProjectSummary {
        id: ProjectId("dovetail".into()),
        name: "Dovetail".into(),
        repo_path: "/home/user/src/dovetail".into(),
        kind: ProjectKind::Code,
    };
    let research = ProjectSummary {
        id: ProjectId("market-sizing".into()),
        name: "Market sizing".into(),
        repo_path: "/home/user/.local/share/dovetail/projects/market-sizing".into(),
        kind: ProjectKind::Research,
    };
    let snapshot = Snapshot {
        seq: 42,
        now_ms: 1_700_000_000_000,
        project: Some(project.clone()),
        projects: vec![project, research],
        tasks: vec![task()],
        agents: vec![subagent()],
        todos: vec![todo()],
        inbox: inbox_items(),
        activity: vec![activity()],
        messages: vec![message()],
        usage: UsageLimits {
            five_hour: Some(UsageWindow {
                utilization: 0.34,
                resets_at_ms: Some(1_700_008_400_000),
                status: UsageStatus::Allowed,
            }),
            seven_day: None,
        },
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
        worktrees: worktree_infos(),
        commands: BTreeMap::from([(
            AgentId("coordinator".into()),
            vec![SlashCommand {
                name: "compact".into(),
                description: "Summarise".into(),
                argument_hint: "<instructions>".into(),
            }],
        )]),
        profile: Some("maveron".into()),
        journal: Some(JournalState {
            latest: Some("2026-11-10".into()),
            closing: false,
        }),
        maps: vec![map_summary()],
        research: vec![research_summary()],
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
        Event::MapUpserted(map_summary()),
        Event::MapLoaded(map()),
        Event::ResearchUpserted(research_summary()),
        Event::ResearchLoaded {
            task: TaskId(4),
            what: research::ResearchWhat::Listing,
            content: research::ResearchContent::Listing(research_listing()),
        },
        Event::ResearchLoaded {
            task: TaskId(4),
            what: research::ResearchWhat::Source(research::SourceId(1)),
            content: research::ResearchContent::Document(research::ResearchDocument {
                path: None,
                markdown: "# S1".into(),
            }),
        },
        Event::TotalsChanged(totals()),
        Event::UsageChanged(UsageLimits::default()),
        Event::WarningSet(warning()),
        Event::WarningCleared(WarningId("mode".into())),
        Event::CommandFailed {
            message: "no such agent".into(),
        },
        Event::TerminalReturned { error: None },
        Event::TerminalReturned {
            error: Some("nvim exited with status 1".into()),
        },
        Event::JournalChanged(JournalState {
            latest: None,
            closing: true,
        }),
        Event::OpeningProject {
            id: ProjectId("market-sizing".into()),
        },
    ]
    .into_iter()
    .chain(inbox_items().into_iter().map(Event::InboxUpserted))
    .chain(worktree_infos().into_iter().map(Event::WorktreeUpserted));
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
        Command::EndOfDay,
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
        Command::NewResearchProject {
            name: "Market sizing".into(),
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
        Command::RefreshWorktrees,
        Command::LoadMap {
            id: map::MapId("checkout".into()),
            version: Some(2),
        },
        Command::CopyToClipboard {
            text: "/tmp/wt".into(),
        },
        Command::OpenEditor {
            path: "/tmp/wt".into(),
        },
        Command::StartRound {
            task: TaskId(4),
            brief: "per seat or flat?".into(),
        },
        Command::CloseRound { task: TaskId(4) },
        Command::LoadResearch {
            task: TaskId(4),
            what: research::ResearchWhat::Brief { round: 1 },
        },
        Command::LoadResearch {
            task: TaskId(4),
            what: research::ResearchWhat::Finding(research::FindingId(3)),
        },
        Command::LoadResearch {
            task: TaskId(4),
            what: research::ResearchWhat::Report,
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

#[test]
fn an_old_blocking_ask_user_and_inbox_payload_still_parse() {
    use dovetail_api::tools::{AskUser, Tool};
    // `ask_user` never blocks (ADR 0007): an old caller's `blocking` is ignored, not refused.
    let tool = Tool::parse(
        "ask_user",
        json!({"question": "Which port?", "blocking": true}),
    );
    assert_eq!(
        tool,
        Ok(Tool::AskUser(AskUser {
            question: "Which port?".into()
        }))
    );
    // Inbox rows stored before the change carry the field too.
    let kind: InboxKind =
        serde_json::from_value(json!({"question": {"text": "Which port?", "blocking": true}}))
            .expect("old payload parses");
    assert_eq!(
        kind,
        InboxKind::Question {
            text: "Which port?".into()
        }
    );
}

#[test]
fn update_task_tells_an_absent_priority_from_a_null_one_and_survives_the_socket() {
    use dovetail_api::tools::{BucketFilter, Role, Tool, ToolError};
    let parse = |args| match Tool::parse("update_task", args) {
        Ok(Tool::UpdateTask(u)) => u,
        other => panic!("{other:?}"),
    };
    let absent = parse(json!({"id": 1, "status": "done"}));
    assert_eq!(absent.p_level, None);
    let cleared = parse(json!({"id": 1, "p_level": null}));
    assert_eq!(cleared.p_level, Some(None));
    let set = parse(json!({"id": 1, "p_level": "P1", "bucket": "unsorted"}));
    assert_eq!(set.p_level, Some(Some(PLevel::P1)));
    assert_eq!(set.bucket, Some(BucketFilter::Unsorted));
    assert_eq!(set.new_bucket(), Some(None));
    for u in [absent, cleared, set] {
        roundtrip(&Tool::UpdateTask(u));
    }

    let refused = |args| {
        Tool::parse("update_task", args)
            .expect("parses")
            .validate(Role::Coordinator)
    };
    assert_eq!(
        refused(json!({"id": 1, "bucket": "now"})),
        Err(ToolError::UpdateIntoNow)
    );
    assert_eq!(
        refused(json!({"id": 1, "status": "archived"})),
        Err(ToolError::UpdateToArchived)
    );
    assert_eq!(refused(json!({"id": 1})), Err(ToolError::NothingToUpdate));
    assert_eq!(
        refused(json!({"id": 1, "title": " "})),
        Err(ToolError::Empty("title"))
    );
}

#[test]
fn a_usage_status_word_this_version_does_not_know_parses_as_unknown() {
    let w: UsageWindow = serde_json::from_value(json!({
        "utilization": 0.5, "resets_at_ms": null, "status": "throttled_soon"
    }))
    .expect("an unknown word still parses");
    assert_eq!(w.status, UsageStatus::Unknown);
    assert_eq!(
        UsageStatus::from_word("allowed_warning"),
        UsageStatus::AllowedWarning
    );
    assert_eq!(UsageStatus::from_word(""), UsageStatus::Unknown);
}

#[test]
fn a_research_proposal_and_its_evidence_survive_json() {
    use research::{Evidence, FindingId, ProposalKind, SourceId};
    let kind = InboxKind::Proposal {
        task: TaskId(4),
        kind: ProposalKind::Decision,
        text: "Flat pricing".into(),
        evidence: vec![
            Evidence::Finding(FindingId(3)),
            Evidence::Source(SourceId(12)),
        ],
        why: Some("simpler".into()),
        alternatives: vec![],
    };
    roundtrip(&kind);
    // Evidence is written as the id the report cites.
    assert_eq!(
        serde_json::to_value(Evidence::Finding(FindingId(3))).unwrap(),
        json!("F3")
    );
    assert_eq!(
        Evidence::parse("[S12]"),
        Some(Evidence::Source(SourceId(12)))
    );
    for bad in ["", "F", "F0", "f3", "X3", "F-1", "F3x", "3"] {
        assert_eq!(Evidence::parse(bad), None, "{bad:?}");
    }
    assert!(serde_json::from_value::<Evidence>(json!("nope")).is_err());
}

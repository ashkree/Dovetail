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

fn tangent() -> TangentSummary {
    TangentSummary {
        task: Task {
            id: TaskId(2),
            notes: "Asked from dovetail on 2026-11-11.".into(),
            bucket: None,
            ..task()
        },
        research: Some(research_summary()),
    }
}

fn research_summary() -> research::ResearchSummary {
    research::ResearchSummary {
        task: TaskId(4),
        dir: "/data/projects/p/research/4-pricing".into(),
        rounds: vec![research::RoundSummary {
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
        kind: Some(research::ResearchKind::Decision),
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
        effort: EffortId(7),
        criterion: Some(CriterionId(3)),
    }
}

fn effort() -> Effort {
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
        effort: Some(ReasoningEffort::Xhigh),
        subagent_tokens: 12_345,
        tool: None,
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
        effort: Some(EffortId(7)),
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
        efforts: vec![effort()],
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
            fresh: false,
        }),
        upgrade_building: true,
        maps: vec![map_summary()],
        research: vec![research_summary()],
        tangents: vec![tangent()],
        models: vec![
            ModelOption::named("opus"),
            ModelOption {
                name: "claude-haiku-4-5-20251001".into(),
                resolved: Some("claude-haiku-4-5-20251001".into()),
                efforts: Some(vec![]),
            },
        ],
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
        Event::EffortUpserted(effort()),
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
                images: vec![research::DocImage {
                    src: "fig.png".into(),
                    path: "/r/fig.png".into(),
                    width: 640,
                    height: 480,
                }],
            }),
        },
        Event::TangentUpserted(tangent()),
        Event::TangentRemoved(TaskId(2)),
        Event::TangentLoaded {
            tangent: TaskId(2),
            what: research::ResearchWhat::Listing,
            content: research::ResearchContent::Listing(research_listing()),
        },
        Event::TotalsChanged(totals()),
        Event::UsageChanged(UsageLimits::default()),
        Event::WarningSet(warning()),
        Event::WarningCleared(WarningId("mode".into())),
        Event::ModelsChanged(vec![ModelOption::named("fable")]),
        Event::CommandFailed {
            message: "no such agent".into(),
        },
        Event::LendTerminal,
        Event::TerminalReturned { error: None },
        Event::TerminalReturned {
            error: Some("nvim exited with status 1".into()),
        },
        Event::JournalChanged(JournalState {
            latest: None,
            closing: true,
            fresh: true,
        }),
        Event::UpgradeBuilding(true),
        Event::OpeningProject {
            id: ProjectId("market-sizing".into()),
        },
        Event::RecipeStarted {
            project: ProjectId("other".into()),
            name: "other".into(),
            path: "/home/user/.config/dovetail/projects/other.toml".into(),
            profiles: vec!["work".into()],
        },
        Event::DocsListed {
            docs: vec![research::DocEntry {
                path: "adr/0018-reading-mode-layout.md".into(),
                title: "Reading mode".into(),
            }],
        },
        Event::DocLoaded {
            path: "research.md".into(),
            document: research::ResearchDocument::text(None, "# Research".into()),
        },
        Event::ProjectBrowsed(Box::new(research::BrowsedProject {
            project: ProjectSummary {
                id: ProjectId("market-sizing".into()),
                name: "Market sizing".into(),
                repo_path: "/d/projects/market-sizing".into(),
                kind: ProjectKind::Research,
            },
            tasks: Vec::new(),
            decisions: vec![decision()],
            research: vec![research_summary()],
            listings: vec![research::BrowsedListing {
                task: TaskId(4),
                listing: research_listing(),
            }],
            docs: Vec::new(),
        })),
        Event::BrowsedLoaded {
            doc: research::BrowsedDoc {
                project: ProjectId("notes-site".into()),
                what: research::BrowseWhat::Doc("intro.md".into()),
            },
            document: research::ResearchDocument::text(None, "# Notes".into()),
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
        Command::FreshStart,
        Command::Prompt {
            agent: id("coordinator"),
            text: "plan M2".into(),
            attachments: Vec::new(),
            effort: None,
        },
        Command::Prompt {
            agent: id("coordinator"),
            text: String::new(),
            attachments: vec![Attachment {
                path: "/data/dovetail/attachments/2.webp".into(),
                media_type: ImageType::Webp,
                size: 10,
            }],
            effort: Some(EffortId(7)),
        },
        Command::CreateEffort(NewEffort {
            name: "Guest Checkout".into(),
            kind: EffortKind::Feature,
            question_or_goal: "A buyer can pay without an account".into(),
        }),
        Command::RenameEffort {
            id: EffortId(7),
            name: "providers".into(),
        },
        Command::UpdateEffort {
            id: EffortId(7),
            patch: vec![
                EffortChange::Kind(EffortKind::Experiment),
                EffortChange::QuestionOrGoal("Which provider?".into()),
                EffortChange::Status(EffortStatus::Closed),
            ],
        },
        Command::SetWhereWasI {
            id: EffortId(7),
            where_was_i: effort().where_was_i,
        },
        Command::AddCriterion {
            effort: EffortId(7),
            text: "Fees compared".into(),
        },
        Command::ClaimCriterion {
            id: CriterionId(3),
            evidence: "finding 41".into(),
        },
        Command::ApproveCriterion { id: CriterionId(3) },
        Command::ReturnCriterion {
            id: CriterionId(3),
            reason: Some("Adyen is missing".into()),
        },
        Command::ReturnCriterion {
            id: CriterionId(3),
            reason: None,
        },
        Command::MoveTask {
            id: TaskId(3),
            effort: EffortId(7),
            criterion: Some(CriterionId(3)),
        },
        Command::MoveTask {
            id: TaskId(3),
            effort: UNSORTED_EFFORT,
            criterion: None,
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
        Command::SetModel {
            agent: id("slice-a"),
            model: "sonnet".into(),
            effort: Some(ReasoningEffort::Max),
        },
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
            effort: Default::default(),
            criterion: None,
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
                TaskChange::Unseen,
                TaskChange::Relate(TaskId(2)),
                TaskChange::Unrelate(TaskId(2)),
            ],
        },
        Command::ArchiveTask { id: TaskId(3) },
        Command::DropRound {
            task: TaskId(3),
            round: Some(2),
        },
        Command::SetResearchKind {
            task: TaskId(3),
            kind: research::ResearchKind::Question,
        },
        Command::FileUnfiled {
            item: InboxId(4),
            task: TaskId(3),
        },
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
        Command::SetRecipeProfile {
            project: ProjectId("other".into()),
            profile: "work".into(),
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
        Command::TerminalReleased,
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
        Command::OpenExternal {
            url: "https://docs.rs/nucleo".into(),
        },
        Command::StartAsk { task: TaskId(4) },
        Command::NewTangent {
            question: "sourdough hydration?".into(),
        },
        Command::AttachTangent {
            tangent: TaskId(2),
            task: TaskId(4),
        },
        Command::FileTangent {
            tangent: TaskId(2),
            project: ProjectId("market-sizing".into()),
        },
        Command::StartFromTangent {
            tangent: TaskId(2),
            project: ProjectId("dovetail".into()),
        },
        Command::DropTangent { tangent: TaskId(2) },
        Command::LoadTangent {
            tangent: TaskId(2),
            what: research::ResearchWhat::Finding(research::FindingId(1)),
        },
        Command::ListDocs,
        Command::BrowseProject {
            id: ProjectId("market-sizing".into()),
        },
        Command::LoadBrowsed {
            project: ProjectId("market-sizing".into()),
            what: research::BrowseWhat::Research {
                task: TaskId(4),
                what: research::ResearchWhat::Report,
            },
        },
        Command::LoadDoc {
            path: "adr/0018-reading-mode-layout.md".into(),
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

/// ADR 0016: each renamed tool still parses by its old name, as the new tool, both from the
/// bridge (`Tool::parse`) and on the socket (an old bridge sends the old name).
#[test]
fn old_tool_names_parse_as_the_new_tools() {
    use dovetail_api::socket::{SocketRequest, SocketRequestBody};
    use dovetail_api::tools::{TOOLS, Tool, deprecated_alias, deprecation_note, tool_spec};
    let renamed: Vec<_> = TOOLS.iter().filter(|t| t.old_name.is_some()).collect();
    assert_eq!(renamed.len(), 12);
    for spec in renamed {
        let old = spec.old_name.unwrap_or_default();
        assert!(
            spec.name.starts_with(spec.suite.as_str()),
            "{} lacks its prefix",
            spec.name
        );
        assert_eq!(tool_spec(old), Some(spec));
        assert_eq!(deprecated_alias(old), Some(spec));
        assert_eq!(deprecated_alias(spec.name), None);
        assert!(deprecation_note(spec).contains(&format!("call `{}` instead", spec.name)));
    }
    let new = Tool::parse("tasks_list", json!({})).expect("new name");
    assert_eq!(Tool::parse("list_tasks", json!({})), Ok(new.clone()));
    // An old bridge's request: `"tool": "list_tasks"`, no `called_as`.
    let line = json!({"id": 1, "body": {"kind": "tool_call", "agent": "a", "tool_use_id": null,
        "call": {"tool": "list_tasks", "input": {}}}});
    let request: SocketRequest = serde_json::from_value(line).expect("old wire name parses");
    assert!(matches!(
        request.body,
        SocketRequestBody::ToolCall { call, called_as: None, .. } if call == new
    ));
}

#[test]
fn an_old_blocking_ask_user_and_inbox_payload_still_parse() {
    use dovetail_api::tools::{AskUser, Tool};
    // `core_ask_user` never blocks (ADR 0007): an old caller's `blocking` is ignored, not refused.
    let tool = Tool::parse(
        "core_ask_user",
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
    let parse = |args| match Tool::parse("tasks_update", args) {
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
        Tool::parse("tasks_update", args)
            .expect("parses")
            .validate(Role::Coordinator)
    };
    assert_eq!(refused(json!({"id": 1, "bucket": "now"})), Ok(()));
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
fn an_agent_message_entry_roundtrips_and_old_kinds_still_read() {
    let entry = ConversationEntry {
        id: EntryId(4),
        at_ms: 9,
        kind: EntryKind::AgentMessage,
        text: agent_message_text("scout", true, "report\nmore"),
        attachments: Vec::new(),
        effort: None,
    };
    roundtrip(&entry);
    assert_eq!(
        serde_json::to_value(&entry).unwrap()["kind"],
        "agent_message"
    );
    let parts = entry.agent_message().unwrap();
    assert_eq!(
        (parts.from, parts.subagent, parts.body),
        ("scout", true, "report\nmore")
    );
    // An entry stored before the kind existed.
    let old: ConversationEntry = serde_json::from_value(
        json!({"id": 1, "at_ms": 0, "kind": "notice", "text": "Message from x: y"}),
    )
    .unwrap();
    assert_eq!(old.kind, EntryKind::Notice);
    assert_eq!(old.agent_message(), None);
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
    roundtrip(&InboxKind::UnfiledFinding {
        finding: 3,
        label: "pricing-pages".into(),
        sources: 2,
    });
    roundtrip(&InboxKind::NewTangents { count: 2 });
    roundtrip(&AgentKind::Tangent);
    // A tangent's session is named after its task in Tangents, and back.
    assert_eq!(tangent_agent(TaskId(12)), AgentId("tangent-12".into()));
    assert_eq!(
        tangent_task(&AgentId("tangent-12".into())),
        Some(TaskId(12))
    );
    assert_eq!(tangent_task(&AgentId("side-1".into())), None);
    assert_eq!(tangent_task(&AgentId("tangent-x".into())), None);
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

/// Values written before efforts existed still read: a task lands in Unsorted, a chat line and a
/// prompt carry no tag, and a snapshot has no efforts.
#[test]
fn values_from_before_efforts_still_parse() {
    let mut old_task = serde_json::to_value(task()).unwrap();
    let fields = old_task.as_object_mut().unwrap();
    fields.remove("effort");
    fields.remove("criterion");
    let parsed: Task = serde_json::from_value(old_task).unwrap();
    assert_eq!(parsed.effort, UNSORTED_EFFORT);
    assert_eq!(parsed.criterion, None);

    let line: ConversationEntry =
        serde_json::from_value(json!({"id": 1, "at_ms": 2, "kind": "user", "text": "hi"})).unwrap();
    assert_eq!(line.effort, None);

    let prompt: Command =
        serde_json::from_value(json!({"prompt": {"agent": "coordinator", "text": "hi"}})).unwrap();
    assert!(matches!(prompt, Command::Prompt { effort: None, .. }));

    let mut old_snapshot = serde_json::to_value(Snapshot::default()).unwrap();
    old_snapshot.as_object_mut().unwrap().remove("efforts");
    let parsed: Snapshot = serde_json::from_value(old_snapshot).unwrap();
    assert!(parsed.efforts.is_empty());
}

/// The names the GUI's TypeScript reads: kinds and states are lower snake case, ids are bare
/// numbers, and an unclaimed criterion has no `evidence` key.
#[test]
fn an_effort_has_the_json_shape_the_gui_reads() {
    let value = serde_json::to_value(effort()).unwrap();
    assert_eq!(value["id"], json!(7));
    assert_eq!(value["kind"], json!("research"));
    assert_eq!(value["status"], json!("open"));
    assert_eq!(
        value["where_was_i"]["waits_on_you"],
        json!("Approve criterion 3")
    );
    assert_eq!(value["criteria"][0]["state"], json!("claimed"));
    assert_eq!(value["criteria"][0]["evidence"], json!("finding 41"));
    assert!(value["criteria"][1].get("evidence").is_none());
    roundtrip(&effort());
}

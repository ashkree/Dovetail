//! The mock's starting state and the text it draws from.

use std::collections::BTreeMap;

use dovetail_api::*;
use serde_json::json;

pub(crate) const COORDINATOR: &str = "coordinator";
pub(crate) const PROJECT_DIR: &str = "/home/you/src/dovetail";
pub(crate) const WORKTREES_DIR: &str =
    "/home/you/.local/share/dovetail/projects/dovetail/worktrees";

pub(crate) fn agent_id(id: &str) -> AgentId {
    AgentId(id.into())
}

fn task(
    id: i64,
    title: &str,
    status: TaskStatus,
    p_level: Option<PLevel>,
    bucket: Option<Bucket>,
    order: i64,
) -> Task {
    Task {
        id: TaskId(id),
        title: title.into(),
        status,
        p_level,
        bucket,
        order,
        deps: vec![],
        notes: String::new(),
        pr: None,
        follow_up_of: None,
        coordinator_created: false,
        related: vec![],
    }
}

pub(crate) fn session(id: &str, kind: AgentKind, status: AgentStatus, now_ms: i64) -> Agent {
    Agent {
        id: agent_id(id),
        name: id.into(),
        kind,
        parent: None,
        task: None,
        worktree: None,
        status,
        status_line: String::new(),
        memory_mb: Some(210),
        cost_usd: 0.0,
        permission_mode: PermissionMode::Auto,
        context_pct: Some(0),
        progress: None,
        last_activity_ms: now_ms,
        lock: SessionLock::Headless,
        model: Some("claude-opus-5-5".into()),
        subagent_tokens: 0,
    }
}

pub(crate) fn worktree(name: &str) -> WorktreeRef {
    WorktreeRef {
        path: format!("{WORKTREES_DIR}/{name}").into(),
        branch: format!("dovetail/{name}"),
    }
}

pub(crate) fn subagent(id: &str, parent: &Agent, background: bool, now_ms: i64) -> Agent {
    Agent {
        id: agent_id(id),
        name: id.into(),
        kind: AgentKind::Subagent { background },
        parent: Some(parent.id.clone()),
        task: parent.task,
        worktree: None,
        status: AgentStatus::Working,
        status_line: String::new(),
        memory_mb: None,
        cost_usd: 0.0,
        permission_mode: parent.permission_mode,
        context_pct: None,
        progress: None,
        last_activity_ms: now_ms,
        lock: SessionLock::Headless,
        model: None,
        subagent_tokens: 0,
    }
}

fn entry(id: u64, at_ms: i64, kind: EntryKind, text: &str) -> ConversationEntry {
    ConversationEntry {
        id: EntryId(id),
        at_ms,
        kind,
        text: text.into(),
        attachments: Vec::new(),
    }
}

/// The world at time zero. Entry ids below 100 are used here; [`crate::MockCore`] counts on.
pub(crate) fn initial(now_ms: i64) -> Snapshot {
    let project = ProjectSummary {
        id: ProjectId("dovetail".into()),
        name: "Dovetail".into(),
        repo_path: PROJECT_DIR.into(),
    };
    let other = ProjectSummary {
        id: ProjectId("notes-site".into()),
        name: "notes-site".into(),
        repo_path: "/home/you/src/notes-site".into(),
    };

    let mut tasks = vec![
        task(
            1,
            "Task tree with bucket grouping",
            TaskStatus::InProgress,
            Some(PLevel::P1),
            Some(Bucket::Now),
            0,
        ),
        task(
            2,
            "Inbox keys and preview",
            TaskStatus::InProgress,
            Some(PLevel::P0),
            Some(Bucket::Now),
            1,
        ),
        task(
            3,
            "Fuzzy picker",
            TaskStatus::InProgress,
            Some(PLevel::P3),
            Some(Bucket::Now),
            2,
        ),
        task(
            4,
            "Conversation reading column",
            TaskStatus::Todo,
            Some(PLevel::P2),
            Some(Bucket::Next),
            0,
        ),
        task(
            5,
            "Quick capture parser",
            TaskStatus::Todo,
            Some(PLevel::P1),
            Some(Bucket::Next),
            1,
        ),
        task(
            6,
            "Memory caps per session",
            TaskStatus::Todo,
            Some(PLevel::P0),
            Some(Bucket::Later),
            0,
        ),
        task(
            7,
            "Map shelf",
            TaskStatus::Todo,
            Some(PLevel::P2),
            Some(Bucket::Later),
            1,
        ),
        task(
            8,
            "Check the kitty graphics fallback",
            TaskStatus::Todo,
            None,
            None,
            0,
        ),
        task(
            9,
            "Workspace skeleton",
            TaskStatus::Done,
            Some(PLevel::P1),
            Some(Bucket::Now),
            3,
        ),
        task(
            10,
            "Old spike notes",
            TaskStatus::Archived,
            Some(PLevel::P3),
            Some(Bucket::Later),
            2,
        ),
    ];
    tasks[0].pr = Some(1231);
    tasks[1].deps = vec![TaskId(9)];
    tasks[7].coordinator_created = true;
    tasks[7].follow_up_of = Some(TaskId(4));
    tasks[7].notes = "Raised while planning the reading column.".into();
    // Related both ways: the tree and the inbox keys are built side by side.
    tasks[0].related = vec![TaskId(2)];
    tasks[1].related = vec![TaskId(1)];

    let mut coordinator = session(
        COORDINATOR,
        AgentKind::Coordinator,
        AgentStatus::Idle,
        now_ms,
    );
    coordinator.status_line = "planning M2".into();
    coordinator.memory_mb = Some(245);
    coordinator.context_pct = Some(18);
    coordinator.cost_usd = 0.84;

    let mut tree = session("tree-view", AgentKind::Slice, AgentStatus::Working, now_ms);
    tree.task = Some(TaskId(1));
    tree.worktree = Some(worktree("tree-view"));
    tree.status_line = "grouping tasks by bucket".into();
    tree.memory_mb = Some(232);
    tree.context_pct = Some(34);
    tree.cost_usd = 0.41;

    let mut inbox_keys = session(
        "inbox-keys",
        AgentKind::Slice,
        AgentStatus::NeedsYou,
        now_ms,
    );
    inbox_keys.task = Some(TaskId(2));
    inbox_keys.worktree = Some(worktree("inbox-keys"));
    inbox_keys.status_line = "wiring y/A/d/D".into();
    inbox_keys.memory_mb = Some(218);
    inbox_keys.context_pct = Some(52);
    inbox_keys.cost_usd = 0.66;
    inbox_keys.permission_mode = PermissionMode::AcceptEdits;

    let mut picker = session("picker", AgentKind::Slice, AgentStatus::SettingUp, now_ms);
    picker.task = Some(TaskId(3));
    picker.worktree = Some(worktree("picker"));
    picker.status_line = "setup: cargo fetch".into();
    picker.memory_mb = Some(190);

    let mut explore = subagent("explore-keymap", &tree, false, now_ms);
    explore.status_line = "reading keymap.rs".into();
    explore.cost_usd = 0.05;

    let inbox = vec![
        InboxItem {
            id: InboxId(1),
            agent: agent_id("inbox-keys"),
            at_ms: now_ms - 40_000,
            kind: InboxKind::Permission {
                tool: "Bash".into(),
                input: json!({"command": "cargo test -p dovetail-tui", "description": "Run the TUI tests"}),
                reason: Some("acceptEdits mode asks before shell commands".into()),
                suggestions: vec![RuleSuggestion {
                    label: "Bash(cargo test:*)".into(),
                    raw: json!({"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "cargo test:*"}], "behavior": "allow", "destination": "session"}),
                }],
            },
        },
        InboxItem {
            id: InboxId(2),
            agent: agent_id(COORDINATOR),
            at_ms: now_ms - 90_000,
            kind: InboxKind::FinishedTurn {
                preview: "M2 splits into three slices: tree view, inbox keys and the picker."
                    .into(),
            },
        },
    ];

    let conversations = BTreeMap::from([
        (
            agent_id(COORDINATOR),
            ConversationTail {
                entries: vec![
                    entry(
                        1,
                        now_ms - 120_000,
                        EntryKind::User,
                        "Plan M2 from docs/implementation-plan.md as the slices it lists.",
                    ),
                    entry(
                        2,
                        now_ms - 110_000,
                        EntryKind::ToolSummary,
                        "Read docs/implementation-plan.md · Read AGENTS.md · Grep \"Layout B\"",
                    ),
                    entry(3, now_ms - 90_000, EntryKind::Assistant, COORDINATOR_PLAN),
                ],
                has_more: true,
            },
        ),
        (
            agent_id("tree-view"),
            ConversationTail {
                entries: vec![
                    entry(
                        4,
                        now_ms - 80_000,
                        EntryKind::User,
                        "Build the task tree: Unsorted, Now, Next, Later, Done, with P-levels and the mismatch marker.",
                    ),
                    entry(
                        5,
                        now_ms - 70_000,
                        EntryKind::Assistant,
                        "Starting with the grouping, then the keys. I'll keep the coordinator row pinned at the top.",
                    ),
                    entry(
                        6,
                        now_ms - 60_000,
                        EntryKind::ToolSummary,
                        "Read buffers/tree.rs · Edit buffers/tree.rs · Bash cargo test",
                    ),
                ],
                has_more: false,
            },
        ),
        (
            agent_id("inbox-keys"),
            ConversationTail {
                entries: vec![
                    entry(
                        7,
                        now_ms - 50_000,
                        EntryKind::User,
                        "Wire the inbox keys y A d D r x and Enter.",
                    ),
                    entry(
                        8,
                        now_ms - 45_000,
                        EntryKind::Assistant,
                        "The keys are in. I'd like to run the TUI tests next.",
                    ),
                ],
                has_more: false,
            },
        ),
    ]);

    let list = TodoListId("tree-view".into());
    let todo = |id: &str, subject: &str, status| TodoItem {
        list: list.clone(),
        id: id.into(),
        subject: subject.into(),
        status,
        blocked_by: vec![],
        created_by: Some(agent_id("tree-view")),
    };
    let todos = vec![
        todo("1", "Group tasks by bucket", TodoStatus::Completed),
        todo(
            "2",
            "Sort by P-level within a bucket",
            TodoStatus::InProgress,
        ),
        todo(
            "3",
            "Mark P-level and bucket mismatches",
            TodoStatus::Pending,
        ),
        todo("4", "Fold Done into one row", TodoStatus::Pending),
    ];

    let activity = vec![
        ActivityItem {
            id: 1,
            at_ms: now_ms - 100_000,
            agent: Some(agent_id("tree-view")),
            kind: ActivityKind::Pr,
            text: "opened #1231 Task tree with bucket grouping".into(),
        },
        ActivityItem {
            id: 2,
            at_ms: now_ms - 30_000,
            agent: Some(agent_id("tree-view")),
            kind: ActivityKind::Ci,
            text: "#1231 CI passed".into(),
        },
    ];
    let messages = vec![AgentMessage {
        id: 1,
        at_ms: now_ms - 85_000,
        from: agent_id(COORDINATOR),
        to: agent_id("tree-view"),
        text: "Keep the keymap table data-like; inbox-keys builds on it.".into(),
    }];

    let mut snapshot = Snapshot {
        seq: 0,
        now_ms,
        project: Some(project.clone()),
        projects: vec![project, other],
        tasks,
        agents: vec![coordinator, tree, inbox_keys, picker, explore],
        todos,
        inbox,
        activity,
        messages,
        conversations,
        totals: Totals::default(),
        warnings: vec![],
        decisions: decisions(now_ms),
        usage: usage(now_ms, 0.34),
        profile: Some("maveron".into()),
        // Like the core: empty until the worktree view asks for a refresh.
        worktrees: vec![],
        commands: BTreeMap::from([(agent_id(COORDINATOR), commands())]),
        // No journal, so quitting the mock never asks to close the day; `:eod` still works.
        journal: None,
    };
    snapshot.totals = totals_of(&snapshot);
    snapshot
}

/// What the coordinator lists for `/` completion: a few built-ins and skills.
fn commands() -> Vec<SlashCommand> {
    [
        (
            "compact",
            "Clear conversation history but keep a summary in context",
            "<instructions>",
        ),
        (
            "code-review",
            "Review the changes since a fixed point along two axes",
            "",
        ),
        ("context", "Show what fills the context window", ""),
        ("grilling", "Grill the user relentlessly about a plan", ""),
    ]
    .into_iter()
    .map(|(name, description, hint)| SlashCommand {
        name: name.into(),
        description: description.into(),
        argument_hint: hint.into(),
    })
    .collect()
}

/// Files the mock lists for `@` completion: the session's tree (`dir: None`) or one directory.
pub(crate) fn paths(dir: Option<&str>) -> Vec<String> {
    match dir {
        None => [
            "Cargo.toml",
            "README.md",
            "crates/dovetail-tui/src/app.rs",
            "docs/design.md",
        ]
        .map(String::from)
        .to_vec(),
        Some(_) => ["notes.md", "projects/"].map(String::from).to_vec(),
    }
}

/// Plausible worktree-view data for slice `agent` with worktree `wt`: the seeded slices each
/// show a different case (an open PR with work in progress, behind main with no PR, a failed
/// setup whose PR lookup failed); any other slice is fresh.
pub(crate) fn worktree_info(agent: &Agent, wt: &WorktreeRef, now_ms: i64) -> WorktreeInfo {
    let pr = |number: u32, state: PrState| {
        PrLookup::Found(PullRequest {
            number,
            state,
            url: format!("https://github.com/you/dovetail/pull/{number}"),
        })
    };
    let (sync, changes, unpushed, pr, setup) = match agent.name.as_str() {
        "tree-view" => (
            (3, 0, Some(0)),
            Changes {
                staged: 1,
                unstaged: 2,
                untracked: 1,
                conflicted: 0,
            },
            1,
            pr(1231, PrState::Open),
            WorktreeSetup::Done,
        ),
        "inbox-keys" => (
            (1, 4, Some(2)),
            Changes::default(),
            1,
            PrLookup::None,
            WorktreeSetup::Done,
        ),
        "picker" => (
            (0, 0, None),
            Changes {
                untracked: 3,
                ..Changes::default()
            },
            0,
            PrLookup::Unknown {
                reason: "gh is not installed".into(),
            },
            WorktreeSetup::Failed {
                error: Some("`cargo fetch` exited with exit status: 101".into()),
            },
        ),
        _ => (
            (0, 0, Some(0)),
            Changes::default(),
            0,
            PrLookup::None,
            WorktreeSetup::Done,
        ),
    };
    WorktreeInfo {
        agent: agent.id.clone(),
        path: wt.path.clone(),
        branch: wt.branch.clone(),
        base: "main".into(),
        sync: Some(SyncState {
            ahead: sync.0,
            behind: sync.1,
            behind_origin: sync.2,
        }),
        changes: Some(changes),
        unpushed: Some(unpushed),
        pr,
        setup,
        checked_ms: now_ms,
    }
}

/// Plausible usage limits: the 5-hour window `five_hour` used and resetting in about 2 h 20 m,
/// the week 18 % used and resetting in 3 days.
pub(crate) fn usage(now_ms: i64, five_hour: f64) -> UsageLimits {
    UsageLimits {
        five_hour: Some(UsageWindow {
            utilization: five_hour,
            resets_at_ms: Some(now_ms + (2 * 3_600 + 20 * 60) * 1_000),
            status: UsageStatus::Allowed,
        }),
        seven_day: Some(UsageWindow {
            utilization: 0.18,
            resets_at_ms: Some(now_ms + 3 * 86_400_000),
            status: UsageStatus::Allowed,
        }),
    }
}

/// Rounds a dollar amount to whole cents.
pub(crate) fn round_cents(usd: f64) -> f64 {
    (usd * 100.0).round() / 100.0
}

/// Totals as the core would compute them.
pub(crate) fn totals_of(state: &Snapshot) -> Totals {
    Totals {
        memory_mb: state.agents.iter().filter_map(|a| a.memory_mb).sum(),
        needs_you: state.inbox.len() as u32,
        cost_usd: round_cents(state.agents.iter().map(|a| a.cost_usd).sum()),
    }
}

const COORDINATOR_PLAN: &str = "## M2 plan\n\nM2 splits into **three slices**:\n\n1. `tree-view` — grouping, P-levels and the *mismatch marker*\n2. `inbox-keys` — `y A d D r x` and the preview\n3. `picker` — the shared picker component\n\nThe keymap table lands first, so the other two build on it.";

/// Replies the mock streams, in markdown.
pub(crate) const REPLIES: &[&str] = &[
    "Grouping works. Each bucket sorts by **P-level**, then by `order`.\n\n- `Now` starts expanded\n- `Later` shows a count\n- `Done` folds into one row\n\nNext I'll add the *mismatch marker*.",
    "## Findings\n\nThe redraw only happens when the **dirty flag** is set. Streaming is coalesced to about 30 fps.\n\n```rust\nif app.dirty {\n    terminal.draw(|f| ui::render(f, &app))?;\n    app.dirty = false;\n}\n```\n\nIdle CPU stays near zero.",
    "Tests pass: `cargo test -p dovetail-tui` reports **41 passed**. I'll push the branch and open the PR.",
    "I hit a snag: the keymap needs a *pending sequence* state for `gg` and `dd`.\n\n1. Store the keys typed so far\n2. Resolve on each key, or on the **1000 ms** timeout\n3. Fall back to the single-key action\n\nThis also leaves room for which-key later.",
    "Done. The inbox preview shows the full tool input, and `Enter` opens the agent's conversation.",
    "Rebased on `main`. One conflict in `theme.rs`, resolved by keeping both colour names.",
    "**Two things differ** from the brief, both in `crates/dovetail-tui/src/keymap.rs`.\n\n| Key | Brief | Built |\n| --- | --- | --- |\n| `h` | focus left | returns to the last left region |\n| `Ctrl-j` | unbound | next region in the column |\n\n> `j` and `k` never leave a list; that is the rule behind both rows.\n\n- keys stay in *one* table\n  - movement is global\n  - actions are buffer-local\n- the picker comes next",
];

/// Status lines the mock reports for slices.
pub(crate) const STATUS_LINES: &[&str] = &[
    "running cargo test",
    "reading the UI brief",
    "editing buffers/tree.rs",
    "fixing a clippy warning",
    "writing snapshot tests",
    "waiting for CI",
    "rebasing on main",
];

/// Tool uses the mock asks permission for: tool name and input.
pub(crate) fn permission_input(n: u64) -> (&'static str, serde_json::Value) {
    match n % 4 {
        0 => (
            "Bash",
            json!({"command": "cargo clippy --workspace --all-targets", "description": "Lint"}),
        ),
        1 => (
            "Edit",
            json!({"file_path": "crates/dovetail-tui/src/keymap.rs", "old_string": "Esc", "new_string": "Escape"}),
        ),
        2 => (
            "WebFetch",
            json!({"url": "https://docs.rs/ratatui/latest/ratatui/", "prompt": "List the layout constraints"}),
        ),
        _ => (
            "Bash",
            json!({"command": "gh pr create --fill", "description": "Open the PR"}),
        ),
    }
}

/// Questions the mock asks with `ask_user`.
pub(crate) const QUESTIONS: &[&str] = &[
    "Should Done tasks fold into one row, or show the last three?",
    "Which colour should the mismatch marker use: amber or the dim accent?",
    "The picker preview needs 40 columns. Is that OK on narrow terminals?",
];

/// The starting decision log: one entry superseded by a later one, both on the tree task.
fn decisions(now_ms: i64) -> Vec<Decision> {
    let first = Decision {
        id: DecisionId(1),
        task: TaskId(1),
        at_ms: now_ms - 3_600_000,
        by: DecisionAuthor::Agent(agent_id(COORDINATOR)),
        text: "Sort every bucket by P-level".into(),
        why: Some("The brief says so.".into()),
        alternatives: vec![],
        supersedes: None,
    };
    let second = Decision {
        id: DecisionId(2),
        task: TaskId(1),
        at_ms: now_ms - 1_800_000,
        by: DecisionAuthor::User,
        text: "Now keeps your own order; the other buckets sort by P-level".into(),
        why: Some("J and K reorder Now, which a P-level sort would undo.".into()),
        alternatives: vec![
            Alternative {
                option: "P-level everywhere".into(),
                why_not: Some("J and K would do nothing in Now".into()),
            },
            Alternative {
                option: "Own order everywhere".into(),
                why_not: None,
            },
        ],
        supersedes: Some(DecisionId(1)),
    };
    vec![first, second]
}

/// Decisions the coordinator logs as the mock runs.
pub(crate) const DECISIONS: &[(&str, &str, &str)] = &[
    (
        "Keep the keymap a constant table",
        "It becomes TOML in v1.1 without a rewrite.",
        "Build bindings in code",
    ),
    (
        "One picker component for both pickers",
        "The brief asks for one.",
        "A separate switcher widget",
    ),
    (
        "Overlays read INSERT",
        "They take typed text.",
        "A fourth mode",
    ),
];

/// Background subagent descriptions.
pub(crate) const SUBAGENT_TASKS: &[&str] = &[
    "find every use of Region::Left",
    "check the crossterm 0.29 changelog",
    "list keys the brief leaves unbound",
];

/// Follow-up task titles the coordinator files.
pub(crate) const FOLLOW_UPS: &[&str] = &[
    "rename the sequence-timeout constant",
    "check kitty with a 200-column pane",
    "decide whether questions beep",
    "trim the preview in the switcher",
];

/// Tools a subagent reports using.
pub(crate) const TOOLS: &[&str] = &["Read", "Grep", "Glob", "Bash", "WebFetch"];

/// Messages between agents.
pub(crate) const MESSAGES: &[&str] = &[
    "The keymap table is merged; rebase when you can.",
    "I need the Action enum to have an InboxAllowAlways variant.",
    "Hold off on theme.rs, I'm renaming the colours.",
    "PR is up for review.",
];

/// The UTC date of a Unix time in milliseconds, `YYYY-MM-DD` (the mock's journal dates).
pub(crate) fn civil_date(ms: i64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

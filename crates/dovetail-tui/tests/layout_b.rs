//! Snapshot of Layout B from the b.png world, plus structure checks.

mod support;

use dovetail_tui::layout::region::RegionId;
use support::*;

#[test]
fn layout_b_at_120_columns() {
    let mut app = app_with(world());
    insta::assert_snapshot!(render(&mut app, 120, 32));
}

#[test]
fn layout_b_at_80_columns() {
    let mut app = app_with(world());
    insta::assert_snapshot!(render(&mut app, 80, 28));
}

#[test]
fn the_tree_groups_tasks_and_hides_empty_unsorted() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 40);
    assert!(
        !screen.contains("Unsorted"),
        "empty Unsorted shows no header"
    );
    let now = screen.find("Now (2)").expect("Now group");
    let next = screen.find("Next (1)").expect("Next group");
    let later = screen.find("Later (1)").expect("Later group");
    let done = screen.find("Done (1)").expect("Done group");
    assert!(now < next && next < later && later < done);
    // Later and Done start collapsed, so their tasks are hidden.
    assert!(!screen.contains("docs sweep"));
    assert!(!screen.contains("release notes"));
}

#[test]
fn unsorted_tasks_show_above_now_with_their_markers() {
    let mut snap = world();
    snap.tasks.push(dovetail_api::tracking::task::Task {
        id: dovetail_api::tracking::task::TaskId(9),
        title: "rename the flag".into(),
        status: dovetail_api::tracking::task::TaskStatus::Todo,
        p_level: None,
        bucket: None,
        order: 9,
        deps: vec![],
        notes: String::new(),
        pr: None,
        follow_up_of: Some(dovetail_api::tracking::task::TaskId(1)),
        coordinator_created: true,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    });
    let mut app = app_with(snap);
    let screen = render(&mut app, 120, 40);
    let unsorted = screen.find("Unsorted (1)").expect("Unsorted group");
    let now = screen.find("Now (2)").expect("Now group");
    assert!(unsorted < now);
    assert!(screen.contains("rename the flag ✦ ↳#1"));
}

#[test]
fn a_p0_in_later_and_a_p3_in_now_are_flagged() {
    let mut snap = world();
    snap.tasks[2].p_level = Some(dovetail_api::tracking::task::PLevel::P3);
    snap.tasks[2].bucket = Some(dovetail_api::tracking::task::Bucket::Now);
    let mut app = app_with(snap);
    let screen = render(&mut app, 120, 40);
    let line = screen
        .lines()
        .find(|l| l.contains("idea: eval key flow"))
        .expect("task row");
    assert!(line.contains("P3 ≠"), "{line}");

    // P0 in Later: unfold Later and look.
    let mut app = app_with(world());
    app.on_key(key('G'), 0);
    for _ in 0..2 {
        app.on_key(key('k'), 0);
    }
    let later_row = render(&mut app, 120, 40);
    assert!(later_row.contains("Later (1)"));
}

#[test]
fn needs_you_is_derived_from_the_agents_inbox_items() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 40);
    let line = screen
        .lines()
        .find(|l| l.contains("get_decision 500"))
        .expect("task row");
    assert!(line.contains('⚠'), "{line}");
    // The first task has agents with inbox items too (slice-4's question), so it is flagged.
    let line = screen
        .lines()
        .find(|l| l.contains("business surface"))
        .expect("task row");
    assert!(line.contains('⚠'), "{line}");
}

#[test]
fn a_coordinator_item_marks_no_task() {
    let mut snap = world();
    snap.inbox.clear();
    snap.inbox.push(dovetail_api::activity::inbox::InboxItem {
        id: dovetail_api::activity::inbox::InboxId(7),
        agent: agent_id("coordinator"),
        at_ms: NOW_MS,
        kind: dovetail_api::activity::inbox::InboxKind::FinishedTurn {
            preview: "done".into(),
        },
    });
    let mut app = app_with(snap);
    let screen = render(&mut app, 120, 40);
    let line = screen
        .lines()
        .find(|l| l.contains("business surface"))
        .expect("task row");
    assert!(!line.contains('⚠'), "{line}");
}

#[test]
fn the_tasks_header_counts_tasks_and_agents_and_the_status_bar_does_not() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 32);
    let bar = screen.lines().last().expect("status bar");
    assert!(bar.contains("NORMAL"), "{bar}");
    assert!(bar.contains("business"), "{bar}");
    for gone in ["tasks", "agents", "⚠", "2.1G"] {
        assert!(!bar.contains(gone), "{gone} left the status bar: {bar}");
    }
    assert!(screen.contains("┌ tasks 4 · agents 3 "), "{screen}");
    assert!(screen.contains("┌ inbox "), "{screen}");
    assert!(screen.contains("┌ screens "), "{screen}");
}

#[test]
fn the_selected_row_is_highlighted_only_when_its_region_has_focus() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = app_with(world());
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    // Row 1 inside the tree (coordinator) is selected and focused: it has a background.
    let tree = dovetail_tui::layout::region::layout(terminal.backend().buffer().area, false, true)
        .side_bottom;
    let (side_x, row) = (tree.x, tree.y + 1);
    let cell = &terminal.backend().buffer()[(side_x + 3, row)];
    assert_ne!(cell.bg, ratatui::style::Color::Reset);
    // Focus the main region: the tree selection loses its background.
    app.on_key(key('h'), 0);
    terminal.draw(|f| app.draw(f)).expect("draw");
    let cell = &terminal.backend().buffer()[(side_x + 3, row)];
    assert_eq!(cell.bg, ratatui::style::Color::Reset);
}

/// `core_report_status` lines show dim after the agent's name, cut to the pane, and go first when
/// the pane is narrow (live check, 2026-11-06).
#[test]
fn status_lines_show_on_agent_rows_and_drop_first_when_narrow() {
    let mut snap = world();
    for a in &mut snap.agents {
        if a.name == "slice-4" {
            a.status_line =
                "rebasing the business surface onto main after the schema change".into();
        }
    }
    let mut app = app_with(snap.clone());
    let wide = render(&mut app, 200, 32);
    insta::assert_snapshot!(wide);
    assert!(wide.contains("rebasing the business"));
    let mut app = app_with(snap);
    let narrow = render(&mut app, 80, 28);
    assert!(!narrow.contains("rebasing"), "dropped in a narrow pane");
    assert!(narrow.contains("slice-4"));
}

/// Live check 2026-11-06: Dovetail's own Permission items and "always allow" hints showed raw
/// JSON and type names. The inbox rows, then each item opened in full (the main pane).
#[test]
fn dovetail_permission_items_read_as_plain_lines() {
    use dovetail_api::activity::inbox::{InboxId, InboxItem, InboxKind, RuleSuggestion};
    let mut snap = world();
    snap.inbox.clear();
    let item = |id, agent: &str, tool: &str, input, suggestions| InboxItem {
        id: InboxId(id),
        agent: agent_id(agent),
        at_ms: NOW_MS,
        kind: InboxKind::Permission {
            tool: tool.into(),
            input,
            reason: None,
            suggestions,
        },
    };
    snap.inbox.push(item(
        1,
        "coordinator",
        dovetail_api::activity::inbox::START_SLICE_PERMISSION,
        serde_json::json!({"brief": "reply ok", "name": "demo-1", "task": 1}),
        vec![],
    ));
    snap.inbox.push(item(
        2,
        "slice-4",
        dovetail_api::activity::inbox::REMOVE_WORKTREE_PERMISSION,
        serde_json::json!({"branch": "dovetail/demo-1", "uncommitted": [],
            "unpushed_commits": 1, "worktree": "/w/demo-1"}),
        vec![],
    ));
    snap.inbox.push(item(
        3,
        "slice-7",
        "Write",
        serde_json::json!({"file_path": "/w/a.txt", "content": "hi"}),
        vec![RuleSuggestion {
            label: "switch to acceptEdits".into(),
            raw: serde_json::json!({"type": "setMode", "mode": "acceptEdits"}),
        }],
    ));
    let mut app = app_with(snap);
    type_keys(&mut app, " n");
    let side = |screen: &str| -> String {
        screen
            .lines()
            .skip_while(|l| !l.contains("inbox"))
            .take(5)
            .map(|l| l.chars().skip(75).collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let main = |screen: &str| -> String {
        screen
            .lines()
            .skip(1)
            .take(14)
            .map(|l| l.chars().take(75).collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut views = vec![side(&render(&mut app, 120, 40))];
    for _ in 0..3 {
        app.on_key(enter(), 0);
        views.push(main(&render(&mut app, 120, 40)));
        app.on_key(esc(), 0);
        assert_eq!(app.focus(), RegionId::SideTop, "Esc goes back to the inbox");
        type_keys(&mut app, "j");
    }
    let all = views.join("\n----\n");
    assert!(!all.contains('{'), "no raw JSON:\n{all}");
    assert!(all.contains("start slice demo-1"), "{all}");
    assert!(all.contains("dovetail/demo-1 · 1 unpushed commit"), "{all}");
    assert!(all.contains("no uncommitted changes"), "{all}");
    assert!(all.contains("A → always allow: switch to"), "{all}");
    assert!(!all.contains("setMode"), "{all}");
    insta::assert_snapshot!(all);
}

/// Task #5: long items filled the inbox. Each is one row, cut with …; Enter shows it in full.
fn long_items() -> dovetail_api::Snapshot {
    use dovetail_api::activity::inbox::{InboxId, InboxItem, InboxKind};
    let mut snap = world();
    snap.inbox.clear();
    let item = |id: u64, agent: &str, kind| InboxItem {
        id: InboxId(id),
        agent: agent_id(agent),
        at_ms: NOW_MS - id as i64 * 60_000,
        kind,
    };
    snap.inbox.push(item(
        1,
        "slice-4",
        InboxKind::Question {
            text: "Should the new route live under /v2/decisions or stay under /api? \
                   The old clients still call /api, and moving it means a redirect.\n\n\
                   - /v2 matches the OpenAPI spec draft\n\
                   - /api keeps the gateway config as it is\n\n\
                   I'd go with /v2 and add the redirect in the same PR."
                .into(),
        },
    ));
    snap.inbox.push(item(
        2,
        "slice-7",
        InboxKind::Permission {
            tool: "Bash".into(),
            input: serde_json::json!({"command":
                "FS_EVAL_VIRTUAL_KEY=dev cargo test -p api get_decision -- --nocapture --test-threads=1"}),
            reason: Some("Bash commands with environment variables need approval".into()),
            suggestions: vec![],
        },
    ));
    snap.inbox.push(item(
        3,
        "slice-7",
        InboxKind::Error {
            text: "Claude Code exited with status 1: the session ran out of memory while \
                   compiling the workspace and was killed by the cgroup limit (2G)."
                .into(),
        },
    ));
    snap.inbox.push(item(
        4,
        "slice-4",
        InboxKind::FinishedTurn {
            preview: "Handler wired to the registry and the route tests pass. \
                      Next I'll update the OpenAPI spec."
                .into(),
        },
    ));
    snap
}

#[test]
fn long_inbox_items_take_one_line_each() {
    let mut app = app_with(long_items());
    type_keys(&mut app, " n");
    let screen = render(&mut app, 120, 32);
    let rows: Vec<&str> = screen
        .lines()
        .skip_while(|l| !l.contains("┌ inbox "))
        .skip(1)
        .take(5)
        .collect();
    for (row, needle) in rows
        .iter()
        .zip(["slice-7", "slice-4", "slice-7", "slice-4"])
    {
        assert!(row.contains(needle) && row.contains('…'), "{row}");
    }
    assert!(
        rows[4].chars().skip(75).all(|c| c == ' ' || c == '│'),
        "nothing under the four rows: {}",
        rows[4]
    );
    assert!(
        !screen.contains("gateway config"),
        "full text stays in the detail"
    );
    insta::assert_snapshot!(screen);
}

#[test]
fn enter_shows_a_long_item_in_full_with_its_keys() {
    let mut app = app_with(long_items());
    // Permission requests sort first; the question is the second row.
    type_keys(&mut app, " nj");
    app.on_key(enter(), 0);
    assert_eq!(app.focus(), RegionId::Main);
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("gateway config"), "{screen}");
    insta::assert_snapshot!(screen);
    // The answer keys work in the detail too.
    assert_eq!(press(&mut app, "r"), vec![], "r opens a prompt");
    type_keys(&mut app, "/v2 please");
    app.on_key(enter(), 1);
    assert_eq!(
        app.take_commands(),
        vec![dovetail_api::Command::Inbox(
            dovetail_api::activity::inbox::InboxCommand::AnswerQuestion {
                item: dovetail_api::activity::inbox::InboxId(1),
                text: "/v2 please".into()
            }
        )]
    );
}

#[test]
fn y_in_a_permission_items_detail_allows_it() {
    let mut app = app_with(long_items());
    type_keys(&mut app, " n");
    app.on_key(enter(), 0);
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("--test-threads=1"), "{screen}");
    assert_eq!(
        press(&mut app, "y"),
        vec![dovetail_api::Command::Inbox(
            dovetail_api::activity::inbox::InboxCommand::AnswerPermission {
                item: dovetail_api::activity::inbox::InboxId(2),
                answer: dovetail_api::activity::inbox::PermissionAnswer::Allow
            }
        )]
    );
}

#[test]
fn focus_mode_hides_the_side_column_and_l_brings_it_back() {
    let mut app = app_with(world());
    press(&mut app, " z");
    assert!(app.focus_mode());
    let screen = render(&mut app, 100, 30);
    assert!(!screen.contains("┌ inbox "), "{screen}");
    press(&mut app, "l");
    assert!(!app.focus_mode());
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("┌ inbox "), "{screen}");
}

#[test]
fn picking_inbox_item_from_finder_in_focus_mode_exits_focus_mode() {
    let mut app = app_with(world());
    press(&mut app, " z");
    assert!(app.focus_mode());
    // Open the finder (<leader><leader>), filter to the question inbox item, and pick it.
    type_keys(&mut app, "  route prefix");
    app.on_key(enter(), 0);
    assert!(!app.focus_mode());
    assert_eq!(app.focus(), RegionId::SideTop);
    let screen = render(&mut app, 120, 32);
    assert!(screen.contains("┌ inbox "), "{screen}");
}

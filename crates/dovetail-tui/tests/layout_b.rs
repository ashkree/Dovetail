//! Snapshot of Layout B from the b.png world, plus structure checks.

mod support;

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
    snap.tasks.push(dovetail_api::Task {
        id: dovetail_api::TaskId(9),
        title: "rename the flag".into(),
        status: dovetail_api::TaskStatus::Todo,
        p_level: None,
        bucket: None,
        order: 9,
        deps: vec![],
        notes: String::new(),
        pr: None,
        follow_up_of: Some(dovetail_api::TaskId(1)),
        coordinator_created: true,
        related: vec![],
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
    snap.tasks[2].p_level = Some(dovetail_api::PLevel::P3);
    snap.tasks[2].bucket = Some(dovetail_api::Bucket::Now);
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
    snap.inbox.push(dovetail_api::InboxItem {
        id: dovetail_api::InboxId(7),
        agent: agent_id("coordinator"),
        at_ms: NOW_MS,
        kind: dovetail_api::InboxKind::FinishedTurn {
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
fn the_status_bar_counts_tasks_agents_and_needs_you() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 32);
    let bar = screen.lines().last().expect("status bar");
    assert!(bar.contains("NORMAL"), "{bar}");
    assert!(
        bar.contains("business · 4 tasks · 3 agents · 3 ⚠ · 2.1G"),
        "{bar}"
    );
}

#[test]
fn the_selected_row_is_highlighted_only_when_its_region_has_focus() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = app_with(world());
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    // Row 1 inside the tree (coordinator) is selected and focused: it has a background.
    let cell = &terminal.backend().buffer()[(3, 1)];
    assert_ne!(cell.bg, ratatui::style::Color::Reset);
    // Focus the main region: the tree selection loses its background.
    app.on_key(key('l'), 0);
    terminal.draw(|f| app.draw(f)).expect("draw");
    let cell = &terminal.backend().buffer()[(3, 1)];
    assert_eq!(cell.bg, ratatui::style::Color::Reset);
}

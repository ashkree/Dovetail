//! M12a: the board buffer. `<leader>b` shows it in the main region; `j k` move within a column,
//! `H L` between columns, `< >` move the card to the previous or next status, `s` cycles it,
//! `a` adds a task and `Enter` opens the task detail. Needs you is a marker on the card (B4).

mod support;

use dovetail_api::*;
use dovetail_tui::App;
use dovetail_tui::region::RegionId;
use support::*;

fn leader(app: &mut App, keys: &str) {
    app.on_key(key(' '), 0);
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 1 + i as u64);
    }
}

/// The world with the board open and focused. The cursor starts on the Todo column, whose first
/// card is #4 "docs sweep" (P0 before P2).
fn board() -> App {
    let mut app = app_with(world());
    leader(&mut app, "b");
    app
}

fn status_to(id: i64, status: TaskStatus) -> Command {
    Command::UpdateTask {
        id: TaskId(id),
        patch: vec![TaskChange::Status(status)],
    }
}

/// The screen line holding `needle`.
fn line_with(screen: &str, needle: &str) -> String {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not on screen:\n{screen}"))
        .to_string()
}

#[test]
fn leader_b_opens_the_board_focused_and_again_goes_back() {
    let mut app = board();
    assert_eq!(app.focus(), RegionId::Main);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("board · business · 4 open"), "{screen}");
    leader(&mut app, "b");
    assert!(!render(&mut app, 160, 40).contains("board ·"));
}

#[test]
fn esc_goes_back_to_the_conversation() {
    let mut app = board();
    app.on_key(esc(), 5);
    assert!(!render(&mut app, 160, 40).contains("board ·"));
}

#[test]
fn greater_than_moves_the_card_to_the_next_status() {
    let mut app = board();
    assert_eq!(press(&mut app, ">"), vec![status_to(4, TaskStatus::InProgress)]);
}

#[test]
fn less_than_moves_the_card_back_and_stops_at_the_first_column() {
    let mut app = board();
    assert_eq!(press(&mut app, "<"), vec![]);
    assert_eq!(app.note(), Some("already in the first column"));
    // In progress: #2 (P0) before #1 (P1).
    assert_eq!(press(&mut app, "L<"), vec![status_to(2, TaskStatus::Todo)]);
}

#[test]
fn greater_than_stops_at_the_last_column() {
    let mut app = board();
    assert_eq!(press(&mut app, "LLL>"), vec![]);
    assert_eq!(app.note(), Some("already in the last column"));
}

#[test]
fn j_and_k_pick_the_card_within_the_column() {
    let mut app = board();
    assert_eq!(press(&mut app, "j>"), vec![status_to(3, TaskStatus::InProgress)]);
    let mut app = board();
    assert_eq!(press(&mut app, "jkk>"), vec![status_to(4, TaskStatus::InProgress)]);
}

#[test]
fn s_cycles_the_status_and_wraps_from_done_to_todo() {
    let mut app = board();
    assert_eq!(press(&mut app, "s"), vec![status_to(4, TaskStatus::InProgress)]);
    let mut app = board();
    assert_eq!(press(&mut app, "LLs"), vec![status_to(5, TaskStatus::Todo)]);
}

#[test]
fn the_cursor_follows_a_moved_card_once_the_core_applies_it() {
    let mut app = board();
    assert_eq!(press(&mut app, ">"), vec![status_to(4, TaskStatus::InProgress)]);
    let mut snap = app.snapshot().clone();
    let task = snap.tasks.iter_mut().find(|t| t.id == TaskId(4)).expect("#4");
    task.status = TaskStatus::InProgress;
    app.on_server(ServerMsg::Snapshot(snap));
    // #4 is now in progress, and still the card the keys act on.
    assert_eq!(press(&mut app, ">"), vec![status_to(4, TaskStatus::Done)]);
}

#[test]
fn a_task_that_needs_you_shows_the_marker_on_its_card() {
    let mut app = board();
    let screen = render(&mut app, 160, 40);
    // slice-7 has a permission request waiting and slice-4 a question, so tasks #2 and #1 are
    // marked, in their own column; #3 has no agent and no mark.
    assert!(line_with(&screen, "#2 get_decision").contains("│ ⚠ #2"), "{screen}");
    assert!(line_with(&screen, "#1 business").contains("│ ⚠ #1"), "{screen}");
    assert!(line_with(&screen, "#3 idea").contains("│ #3"), "{screen}");
    // There is no needs-you column.
    assert!(!screen.contains("Needs you"));
}

#[test]
fn a_done_task_shows_no_marker_even_with_an_item_waiting() {
    let mut snap = world();
    let t2 = snap.tasks.iter_mut().find(|t| t.id == TaskId(2)).expect("#2");
    t2.status = TaskStatus::Done;
    let mut app = app_with(snap);
    leader(&mut app, "b");
    let screen = render(&mut app, 160, 40);
    assert!(line_with(&screen, "#2 get_decision").contains("│ #2"), "{screen}");
}

#[test]
fn enter_opens_the_task_detail_and_esc_returns_to_the_board() {
    let mut app = board();
    app.on_key(enter(), 5);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("docs sweep"));
    assert!(!screen.contains("board ·"), "{screen}");
    app.on_key(esc(), 6);
    assert!(render(&mut app, 160, 40).contains("board · business"));
}

#[test]
fn a_opens_quick_capture() {
    let mut app = board();
    type_keys(&mut app, "a");
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("linked to"), "{screen}");
}

#[test]
fn snapshot_board_at_120() {
    let mut app = board();
    insta::assert_snapshot!(render(&mut app, 120, 34));
}

#[test]
fn snapshot_board_at_200() {
    let mut app = board();
    type_keys(&mut app, "L");
    insta::assert_snapshot!(render(&mut app, 200, 34));
}

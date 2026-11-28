//! M12a: the board buffer. `<leader>b` shows it in the main region; `j k` move within a column,
//! `Tab` and `Shift-Tab` between columns, `H L` move the card to the previous or next status, `< >` move it up or
//! down its column, `s` cycles the status, `a` adds a task and `Enter` opens the task detail.
//! Needs you is a marker on the card (B4).

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    Command, ServerMsg,
    tracking::task::{TaskChange, TaskCommand, TaskId, TaskStatus},
};
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
/// card is #3 "idea: eval key flow" (Next before Later).
fn board() -> App {
    let mut app = app_with(world());
    leader(&mut app, "b");
    app
}

fn status_to(id: i64, status: TaskStatus) -> Command {
    Command::Task(TaskCommand::Update {
        id: TaskId(id),
        patch: vec![TaskChange::Status(status)],
    })
}

fn reorder(id: i64, delta: i32) -> Command {
    Command::Task(TaskCommand::Reorder {
        id: TaskId(id),
        delta,
    })
}

/// Presses `keys` in order and returns the commands that produced. `⇥` is Tab and `⇤` is
/// Shift-Tab (sent as BackTab with Shift, as crossterm reports it).
fn go(app: &mut App, keys: &str) -> Vec<Command> {
    for (i, c) in keys.chars().enumerate() {
        let event = match c {
            '⇥' => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            '⇤' => KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            c => key(c),
        };
        app.on_key(event, 10 * i as u64);
    }
    app.take_commands()
}

/// Gives task `id` a new status, as the core does when it confirms a change.
fn confirm(app: &mut App, id: i64, status: TaskStatus) {
    let mut snap = app.snapshot().clone();
    let task = snap
        .tasks
        .iter_mut()
        .find(|t| t.id == TaskId(id))
        .expect("task");
    task.status = status;
    app.on_server(ServerMsg::Snapshot(snap));
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
    assert!(!render(&mut app, 160, 40).contains("board · business"));
}

#[test]
fn esc_goes_back_to_the_conversation() {
    let mut app = board();
    app.on_key(esc(), 5);
    assert!(!render(&mut app, 160, 40).contains("board · business"));
}

// --- H and L: status ---

#[test]
fn shift_l_moves_the_card_to_the_next_status() {
    let mut app = board();
    assert_eq!(
        go(&mut app, "L"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
}

#[test]
fn shift_h_moves_the_card_back_and_stops_at_the_first_column() {
    let mut app = board();
    assert_eq!(go(&mut app, "H"), vec![]);
    assert_eq!(app.note(), Some("already in the first column"));
    // In progress: #1 then #2, both in Now, in the user's order.
    assert_eq!(go(&mut app, "⇥H"), vec![status_to(1, TaskStatus::Todo)]);
}

#[test]
fn shift_l_stops_at_the_last_column() {
    let mut app = board();
    assert_eq!(go(&mut app, "⇥⇥L"), vec![]);
    assert_eq!(app.note(), Some("already in the last column"));
}

#[test]
fn j_and_k_pick_the_card_within_the_column() {
    let mut app = board();
    assert_eq!(
        go(&mut app, "jL"),
        vec![status_to(4, TaskStatus::InProgress)]
    );
    let mut app = board();
    assert_eq!(
        go(&mut app, "jkkL"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
}

#[test]
fn tab_and_shift_tab_move_between_columns_and_wrap() {
    let mut app = board();
    // Shift-Tab from Todo wraps to Done (#5); Tab from Done wraps back to Todo (#3).
    assert_eq!(go(&mut app, "⇤s"), vec![status_to(5, TaskStatus::Todo)]);
    assert_eq!(
        go(&mut app, "⇥s"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
}

#[test]
fn h_and_l_stay_region_focus() {
    let mut app = board();
    type_keys(&mut app, "l");
    assert_ne!(app.focus(), RegionId::Main);
}

#[test]
fn s_cycles_the_status_and_wraps_from_done_to_todo() {
    let mut app = board();
    assert_eq!(
        go(&mut app, "s"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
    let mut app = board();
    assert_eq!(go(&mut app, "⇥⇥s"), vec![status_to(5, TaskStatus::Todo)]);
}

#[test]
fn the_cursor_follows_a_moved_card_once_the_core_confirms_it() {
    let mut app = board();
    assert_eq!(
        go(&mut app, "L"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
    confirm(&mut app, 3, TaskStatus::InProgress);
    // #3 is now in progress, and still the card the keys act on.
    assert_eq!(go(&mut app, "L"), vec![status_to(3, TaskStatus::Done)]);
}

#[test]
fn a_refused_move_leaves_the_cursor_where_the_card_is() {
    let mut app = board();
    assert_eq!(
        go(&mut app, "L"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
    // The core refuses: nothing changes, so the cursor stays on #3 in Todo.
    render(&mut app, 160, 40);
    assert_eq!(
        go(&mut app, "L"),
        vec![status_to(3, TaskStatus::InProgress)]
    );
    // The refused move is forgotten at the next key: a later change to the same status does
    // not pull the cursor along.
    type_keys(&mut app, "j");
    confirm(&mut app, 3, TaskStatus::InProgress);
    assert_eq!(
        go(&mut app, "L"),
        vec![status_to(4, TaskStatus::InProgress)]
    );
}

// --- < and >: order within the column ---

#[test]
fn greater_than_and_less_than_reorder_within_the_column() {
    // In progress holds #1 then #2, both in Now: `>` moves #1 past #2, `<` moves #2 above #1.
    let mut app = board();
    assert_eq!(go(&mut app, "⇥>"), vec![reorder(1, 1)]);
    let mut app = board();
    assert_eq!(go(&mut app, "⇥j<"), vec![reorder(2, -1)]);
}

#[test]
fn reordering_counts_places_in_the_whole_bucket() {
    // Done #5 sits between #1 and #2 in Now, so moving #1 past #2 is two places.
    let mut snap = world();
    for (id, order) in [(1, 0), (5, 1), (2, 2)] {
        snap.tasks
            .iter_mut()
            .find(|t| t.id == TaskId(id))
            .expect("task")
            .order = order;
    }
    let mut app = app_with(snap);
    leader(&mut app, "b");
    assert_eq!(go(&mut app, "⇥>"), vec![reorder(1, 2)]);
}

#[test]
fn reordering_stops_at_the_ends_and_at_a_bucket_boundary() {
    let mut app = board();
    assert_eq!(go(&mut app, "<"), vec![]);
    assert_eq!(app.note(), Some("already at the top"));
    // Todo holds #3 (Next) then #4 (Later).
    assert_eq!(go(&mut app, ">"), vec![]);
    assert!(
        app.note()
            .is_some_and(|n| n.contains("within their bucket"))
    );
    assert_eq!(go(&mut app, "j>"), vec![]);
    assert_eq!(app.note(), Some("already at the bottom"));
}

// --- needs you ---

#[test]
fn a_task_that_needs_you_shows_the_marker_on_its_card() {
    let mut app = board();
    let screen = render(&mut app, 160, 40);
    // slice-7 has a permission request waiting and slice-4 a question, so tasks #2 and #1 are
    // marked, in their own column; #3 has no agent and no mark.
    assert!(
        line_with(&screen, "#2 get_decision").contains("│ ⚠ #2"),
        "{screen}"
    );
    assert!(
        line_with(&screen, "#1 business").contains("│ ⚠ #1"),
        "{screen}"
    );
    assert!(line_with(&screen, "#3 idea").contains("│ #3"), "{screen}");
    // There is no needs-you column.
    assert!(!screen.contains("Needs you"));
}

#[test]
fn a_done_task_shows_no_marker_even_with_an_item_waiting() {
    let mut snap = world();
    let t2 = snap
        .tasks
        .iter_mut()
        .find(|t| t.id == TaskId(2))
        .expect("#2");
    t2.status = TaskStatus::Done;
    let mut app = app_with(snap);
    leader(&mut app, "b");
    let screen = render(&mut app, 160, 40);
    assert!(
        line_with(&screen, "#2 get_decision").contains("│ #2"),
        "{screen}"
    );
}

// --- opening and adding ---

#[test]
fn enter_opens_the_task_detail_and_esc_returns_to_the_board() {
    let mut app = board();
    app.on_key(enter(), 5);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("idea: eval key flow"));
    assert!(!screen.contains("board · business"), "{screen}");
    app.on_key(esc(), 6);
    assert!(render(&mut app, 160, 40).contains("board · business"));
}

#[test]
fn a_task_opened_from_the_picker_over_the_board_goes_back_to_the_conversation() {
    let mut app = board();
    leader(&mut app, " ");
    type_keys(&mut app, "docs sweep");
    app.on_key(enter(), 50);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("docs sweep"), "{screen}");
    assert!(!screen.contains("board · business"), "{screen}");
    app.on_key(esc(), 51);
    let screen = render(&mut app, 160, 40);
    assert!(!screen.contains("board · business"), "{screen}");
}

#[test]
fn a_opens_quick_capture() {
    let mut app = board();
    type_keys(&mut app, "a");
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("linked to"), "{screen}");
}

// --- snapshots ---

#[test]
fn snapshot_board_at_120() {
    let mut app = board();
    insta::assert_snapshot!(render(&mut app, 120, 34));
}

#[test]
fn snapshot_board_at_200() {
    let mut app = board();
    go(&mut app, "⇥");
    insta::assert_snapshot!(render(&mut app, 200, 34));
}

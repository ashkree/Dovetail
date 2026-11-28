//! Task #19: making features discoverable. The which-key popup, `:keys` and the per-view hints
//! on the status bar all read the keymap table.

mod support;

use std::time::Duration;

use dovetail_api::{Command, code::worktree::WorktreeCommand};
use dovetail_tui::App;
use dovetail_tui::event_loop::next_wake;
use dovetail_tui::keymap::help::which_key;
use dovetail_tui::keymap::mode::Mode;
use dovetail_tui::keymap::{Key, Keymap, Scope, WHICH_KEY_DELAY_MS};
use dovetail_tui::layout::region::BufferKind;
use support::*;

/// What the popup lists after `keys`, from the keymap table alone, one row per line.
fn popup_text(scopes: &[Scope], keys: &str) -> String {
    let pending = Key::parse_sequence(keys).expect("valid keys");
    which_key(&Keymap::default_map(), Mode::Normal, scopes, &pending)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

const CONVERSATION: [Scope; 2] = [Scope::Buffer(BufferKind::Conversation), Scope::Global];

#[test]
fn which_key_after_leader() {
    insta::assert_snapshot!(popup_text(&CONVERSATION, "<leader>"));
}

#[test]
fn which_key_after_leader_s() {
    insta::assert_snapshot!(popup_text(&CONVERSATION, "<leader>s"));
}

/// Types `keys` at `t0`, as the terminal would.
fn press_at(app: &mut App, keys: &str, t0: u64) {
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), t0 + i as u64);
    }
}

#[test]
fn the_popup_opens_once_after_the_delay_and_not_before() {
    let mut app = app_with(world());
    // Nothing pending: the loop has no deadline and sleeps.
    assert_eq!(app.next_deadline_ms(), None);
    assert!(app.which_key().is_empty());

    app.on_key(key(' '), 100);
    assert_eq!(app.next_deadline_ms(), Some(100 + WHICH_KEY_DELAY_MS));
    // The loop wakes once, WHICH_KEY_DELAY_MS after the key.
    assert_eq!(
        next_wake(
            false,
            Duration::ZERO,
            Some(Duration::from_millis(WHICH_KEY_DELAY_MS)),
            None
        ),
        Some(Duration::from_millis(WHICH_KEY_DELAY_MS))
    );
    app.on_timeout(100 + WHICH_KEY_DELAY_MS - 1);
    assert!(app.which_key().is_empty());

    app.take_dirty();
    app.on_timeout(100 + WHICH_KEY_DELAY_MS);
    assert!(!app.which_key().is_empty());
    assert!(app.is_dirty(), "opening the popup redraws");
    // Open: the sequence waits for the user, and nothing wakes the loop again.
    assert_eq!(app.next_deadline_ms(), None);
}

#[test]
fn the_popup_follows_more_keys_and_goes_on_completion_or_esc() {
    let mut app = app_with(world());
    app.on_key(key(' '), 0);
    app.on_timeout(WHICH_KEY_DELAY_MS);
    let first = app.which_key();
    app.on_key(key('s'), 2_000);
    let second = app.which_key();
    assert!(!second.is_empty());
    assert_ne!(first, second, "typing s narrows the list to the s group");
    assert_eq!(app.next_deadline_ms(), None);

    // Esc cancels the sequence.
    app.on_key(esc(), 2_100);
    assert!(app.which_key().is_empty());
    assert_eq!(app.next_deadline_ms(), None);

    // Completing a binding closes it too.
    app.on_key(key(' '), 3_000);
    app.on_timeout(3_000 + WHICH_KEY_DELAY_MS);
    assert!(!app.which_key().is_empty());
    app.on_key(key('b'), 4_000);
    assert!(app.which_key().is_empty());
}

#[test]
fn a_quick_sequence_never_shows_the_popup() {
    let mut app = app_with(world());
    press_at(&mut app, " b", 0);
    assert!(app.which_key().is_empty());
    assert_eq!(app.next_deadline_ms(), None);
}

#[test]
fn the_popup_is_drawn_bottom_right_above_the_status_bar() {
    let mut app = app_with(world());
    app.on_key(key(' '), 0);
    app.on_timeout(WHICH_KEY_DELAY_MS);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("show the coordinator"), "{screen}");
    assert!(screen.contains("Agents"), "{screen}");
    insta::assert_snapshot!("popup_after_leader_at_120x40", screen);

    app.on_key(key('s'), 1_000);
    insta::assert_snapshot!("popup_after_leader_s_at_120x40", render(&mut app, 120, 40));
}

#[test]
fn a_short_terminal_flows_the_popup_into_columns() {
    let mut app = app_with(world());
    app.on_key(key(' '), 0);
    app.on_timeout(WHICH_KEY_DELAY_MS);
    insta::assert_snapshot!(render(&mut app, 100, 18));
}

// --- :keys ---

fn run_command(app: &mut App, line: &str) {
    app.on_key(key(':'), 0);
    type_keys(app, line);
    app.on_key(enter(), 1_000);
}

#[test]
fn keys_opens_the_picker_and_filters_by_what_you_type() {
    let mut app = app_with(world());
    run_command(&mut app, "keys");
    let all = render(&mut app, 120, 34);
    assert!(all.contains("Keys"), "{all}");
    type_keys(&mut app, "worktrees");
    let filtered = render(&mut app, 120, 34);
    assert!(filtered.contains("worktrees"), "{filtered}");
    insta::assert_snapshot!(filtered);
}

#[test]
fn choosing_a_runnable_entry_runs_it() {
    let mut app = app_with(world());
    run_command(&mut app, "keys");
    type_keys(&mut app, "ShowWorktrees");
    app.on_key(enter(), 2_000);
    assert!(
        app.take_commands()
            .iter()
            .any(|c| matches!(c, Command::Worktree(WorktreeCommand::Refresh))),
        "the worktree view opened"
    );
}

#[test]
fn choosing_an_entry_that_works_elsewhere_only_closes() {
    let mut app = app_with(world());
    run_command(&mut app, "keys");
    type_keys(&mut app, "MapZoomIn");
    app.on_key(enter(), 2_000);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.note().is_some_and(|n| n.contains("another view")));
}

//! The mode state machine (Normal, Insert, Command, Map) driven through `App`: every
//! transition, what `Esc` does in each mode, and the transitions that must not happen.

mod support;

use dovetail_mock::{MockConfig, MockCore};
use dovetail_tui::{App, Mode};
use support::*;

/// An app in MAP mode: the shelf (`<leader>M`), `Enter` on the first map, the core's answer.
fn in_map_mode() -> App {
    let mut core = MockCore::new(MockConfig::default());
    let mut app = app_with(core.state().clone());
    type_keys(&mut app, " M");
    app.on_key(enter(), 100);
    for command in app.take_commands() {
        core.handle(command);
    }
    while core.peek_delay() == 0 {
        let Some(scheduled) = core.next_message() else {
            break;
        };
        app.on_server(scheduled.msg);
    }
    assert_eq!(app.mode(), Mode::Map, "setup");
    app
}

#[test]
fn normal_to_insert_and_back_by_submit_and_by_esc() {
    let mut app = app_with(world());
    assert_eq!(app.mode(), Mode::Normal);
    app.on_key(key('a'), 0);
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "x");
    app.on_key(enter(), 50);
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.take_commands().len(), 1, "submit sends the new task");

    app.on_key(key('a'), 100);
    assert_eq!(app.mode(), Mode::Insert);
    app.on_key(esc(), 101);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(
        app.take_commands().is_empty(),
        "Esc cancels: nothing is sent"
    );
}

#[test]
fn normal_to_command_and_back_by_enter_and_by_esc() {
    let mut app = app_with(world());
    app.on_key(key(':'), 0);
    assert_eq!(app.mode(), Mode::Command);
    app.on_key(enter(), 1);
    assert_eq!(
        app.mode(),
        Mode::Normal,
        "an empty command still ends the mode"
    );

    app.on_key(key(':'), 2);
    type_keys(&mut app, "nosuchcommand");
    app.on_key(esc(), 50);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.take_commands().is_empty());
}

#[test]
fn an_unknown_command_returns_to_normal_and_sends_nothing() {
    let mut app = app_with(world());
    type_keys(&mut app, ":nosuchcommand");
    app.on_key(enter(), 50);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.take_commands().is_empty());
}

#[test]
fn esc_in_normal_mode_stays_in_normal_mode() {
    let mut app = app_with(world());
    app.on_key(esc(), 0);
    app.on_key(esc(), 1);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.take_commands().is_empty());
}

#[test]
fn esc_in_map_mode_goes_to_normal_and_stays_there() {
    let mut app = in_map_mode();
    app.on_key(esc(), 0);
    assert_eq!(app.mode(), Mode::Normal);
    app.on_key(esc(), 1);
    app.on_key(esc(), 2);
    assert_eq!(app.mode(), Mode::Normal, "Esc never re-enters MAP mode");
}

#[test]
fn map_mode_to_command_and_to_insert_goes_through_normal() {
    let mut app = in_map_mode();
    app.on_key(key(':'), 0);
    assert_eq!(app.mode(), Mode::Command);
    app.on_key(esc(), 1);
    assert_eq!(app.mode(), Mode::Normal);
    app.on_key(key('i'), 2);
    assert_eq!(app.mode(), Mode::Insert);
    app.on_key(esc(), 3);
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn map_mode_cannot_be_entered_without_a_map() {
    let mut app = app_with(world());
    // `<leader>v` is "back to the map"; with no map open it does nothing.
    type_keys(&mut app, " v");
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn a_command_prompt_is_not_left_by_normal_mode_keys() {
    let mut app = app_with(world());
    app.on_key(key(':'), 0);
    // `a`, `i`, `j` and `:` are text here, not mode switches.
    type_keys(&mut app, "aij:");
    assert_eq!(app.mode(), Mode::Command);
    app.on_key(esc(), 50);
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn an_insert_prompt_is_not_left_by_a_colon() {
    let mut app = app_with(world());
    app.on_key(key('a'), 0);
    type_keys(&mut app, ":q");
    assert_eq!(app.mode(), Mode::Insert, "':' is text while typing a title");
    app.on_key(esc(), 50);
    assert_eq!(app.mode(), Mode::Normal);
}

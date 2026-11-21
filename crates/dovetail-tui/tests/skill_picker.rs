//! Task #43: `/` at the start of the chat bar opens the skill picker (the same picker as the
//! agent switcher); picking inserts the command, Esc leaves the `/` alone.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    Command,
    agent::SlashCommand,
    conversation::ConversationCommand,
    model::{ModelOption, ReasoningEffort},
};
use dovetail_tui::Mode;
use support::*;

fn skill(name: &str, description: &str, hint: &str) -> SlashCommand {
    SlashCommand {
        name: name.into(),
        description: description.into(),
        argument_hint: hint.into(),
    }
}

fn app_with_skills() -> dovetail_tui::App {
    let mut snap = world();
    snap.commands.insert(
        agent_id("coordinator"),
        vec![
            skill("compact", "Compact the conversation", "[focus]"),
            skill("grilling", "Grill the user about a plan", ""),
            // Claude Code's own /model is hidden behind Dovetail's.
            skill("model", "Claude Code's model command", ""),
        ],
    );
    snap.models = vec![ModelOption {
        name: "haiku".into(),
        resolved: Some("claude-haiku-5-5".into()),
        efforts: Some(ReasoningEffort::ALL.to_vec()),
    }];
    let mut app = app_with(snap);
    press(&mut app, "i");
    app
}

fn prompt(text: &str) -> Vec<Command> {
    vec![Command::Conversation(ConversationCommand::Prompt {
        agent: agent_id("coordinator"),
        text: text.into(),
        attachments: Vec::new(),
        effort: None,
    })]
}

#[test]
fn a_slash_opens_the_picker_with_dovetail_commands_marked() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/");
    let screen = render(&mut app, 120, 30);
    assert!(screen.contains("Skills"), "{screen}");
    assert!(screen.contains("◆ /handoff"), "{screen}");
    assert!(screen.contains("◆ /model"), "{screen}");
    assert!(screen.contains("/ /grilling"), "{screen}");
    assert!(!screen.contains("Claude Code's model command"), "{screen}");
}

#[test]
fn typing_filters_and_the_preview_shows_arguments() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/comp");
    let screen = render(&mut app, 120, 30);
    assert!(screen.contains("/compact"), "{screen}");
    assert!(!screen.contains("/grilling"), "{screen}");
    assert!(screen.contains("/compact [focus]"), "{screen}");
    assert!(screen.contains("Compact the conversation"), "{screen}");
}

#[test]
fn picking_replaces_the_slash_and_the_command_can_take_arguments() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/gril");
    app.on_key(enter(), 100);
    type_keys(&mut app, "the plan");
    app.on_key(enter(), 200);
    assert_eq!(app.take_commands(), prompt("/grilling the plan"));
}

#[test]
fn esc_closes_the_picker_and_leaves_the_slash() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/");
    app.on_key(esc(), 100);
    assert_eq!(app.mode(), Mode::Insert);
    let screen = render(&mut app, 120, 30);
    assert!(!screen.contains("Skills"), "{screen}");
    type_keys(&mut app, "compact");
    app.on_key(enter(), 200);
    assert_eq!(app.take_commands(), prompt("/compact"));
}

#[test]
fn a_slash_later_in_the_text_does_not_open_it() {
    let mut app = app_with_skills();
    type_keys(&mut app, "a/");
    assert!(!render(&mut app, 120, 30).contains("Skills"));
}

#[test]
fn skills_command_opens_the_same_picker() {
    let mut app = app_with_skills();
    app.on_key(esc(), 1);
    type_keys(&mut app, ":skills");
    app.on_key(enter(), 2);
    assert!(render(&mut app, 120, 30).contains("◆ /handoff"));
}

#[test]
fn backspacing_to_a_lone_slash_does_not_open_the_picker() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/a");
    app.on_key(esc(), 1);
    // Esc closed the picker; backspace back to a lone `/`.
    app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), 2);
    let screen = render(&mut app, 120, 30);
    assert!(!screen.contains("Skills"), "{screen}");
    assert_eq!(app.mode(), Mode::Insert);
}

#[test]
fn skills_without_session_commands_lists_dovetails_own() {
    let mut app = app_with(world());
    type_keys(&mut app, ":skills");
    app.on_key(enter(), 1);
    let screen = render(&mut app, 120, 30);
    for name in ["/model", "/mcp", "/handoff", "/fresh"] {
        assert!(screen.contains(&format!("◆ {name}")), "{name}: {screen}");
    }
}

#[test]
fn a_picker_opened_from_normal_closes_to_normal_even_with_a_chat_bar_open() {
    let mut app = app_with_skills();
    type_keys(&mut app, "hi");
    app.on_key(esc(), 1);
    assert_eq!(app.mode(), Mode::Normal);
    type_keys(&mut app, " ss");
    assert!(render(&mut app, 120, 30).contains("Switch"));
    app.on_key(esc(), 2);
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn choosing_a_command_from_the_slash_picker_leaves_typing_in_the_bar() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/");
    app.on_key(enter(), 1);
    assert_eq!(app.mode(), Mode::Insert);
}

#[test]
fn a_space_hands_the_typing_to_the_bar_so_a_whole_command_can_be_typed_fast() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/gril ");
    assert_eq!(app.mode(), Mode::Insert);
    let screen = render(&mut app, 120, 30);
    assert!(!screen.contains("Skills"), "{screen}");
    assert!(screen.contains("/gril "), "{screen}");
    // The text continues in the bar as typed, with no command picked.
    type_keys(&mut app, "x");
    app.on_key(enter(), 1);
    assert_eq!(app.take_commands(), prompt("/gril x"));
}

#[test]
fn slash_model_haiku_typed_at_full_speed_goes_through_the_model_intercept() {
    let mut app = app_with_skills();
    type_keys(&mut app, "/model haiku");
    app.on_key(enter(), 1);
    assert!(app.take_commands().is_empty(), "/model is never sent");
    assert!(render(&mut app, 120, 30).contains("model · coordinator"));
}

#[test]
fn a_space_in_the_skills_picker_stays_in_its_filter() {
    let mut app = app_with_skills();
    app.on_key(esc(), 1);
    type_keys(&mut app, ":skills");
    app.on_key(enter(), 2);
    type_keys(&mut app, "comp ");
    let screen = render(&mut app, 120, 30);
    assert!(
        screen.contains("Skills") && screen.contains("/compact"),
        "{screen}"
    );
}

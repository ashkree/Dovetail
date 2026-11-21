//! The side panel (decided 2026-11-11): side chats in the side column beside the coordinator's
//! conversation, toggled with `<leader>?`.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    Command, Event, ServerMsg, Snapshot,
    agent::{AgentCommand, AgentEvent, AgentKind, AgentStatus, PermissionMode},
    conversation::{ConversationCommand, ConversationTail, EntryKind},
};
use dovetail_tui::mode::Mode;
use dovetail_tui::region::RegionId;
use support::*;

/// The b.png world with two side chats, the first with a question and an answer.
fn with_side_chats() -> Snapshot {
    let mut snap = world();
    let mut first = agent("side-1", AgentKind::SideChat, AgentStatus::Idle, None);
    first.permission_mode = PermissionMode::Manual;
    first.cost_usd = 0.03;
    let mut second = agent("side-2", AgentKind::SideChat, AgentStatus::Working, None);
    second.permission_mode = PermissionMode::Manual;
    snap.agents.push(first);
    snap.agents.push(second);
    snap.conversations.insert(
        agent_id("side-1"),
        ConversationTail {
            entries: vec![
                entry(
                    500,
                    NOW_MS - 4 * 60_000,
                    EntryKind::User,
                    "Why does the reducer refuse a gap in seq?",
                ),
                entry(
                    501,
                    NOW_MS - 3 * 60_000,
                    EntryKind::Assistant,
                    "**A gap means an event was lost.** `apply` returns `Gap` instead of \
                     guessing, and the UI asks for a resync (`crates/dovetail-api/src/apply.rs`).",
                ),
            ],
            has_more: false,
        },
    );
    snap.conversations.insert(
        agent_id("side-2"),
        ConversationTail {
            entries: vec![entry(
                502,
                NOW_MS - 60_000,
                EntryKind::User,
                "Which crates pull in tokio?",
            )],
            has_more: false,
        },
    );
    snap
}

fn panel_app() -> dovetail_tui::App {
    let mut app = app_with(with_side_chats());
    assert!(press(&mut app, " ?").is_empty());
    app
}

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn back_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
}

#[test]
fn snapshot_side_panel_at_120() {
    let mut app = panel_app();
    insta::assert_snapshot!(render(&mut app, 120, 34));
}

#[test]
fn snapshot_side_panel_at_200() {
    let mut app = panel_app();
    insta::assert_snapshot!(render(&mut app, 200, 40));
}

#[test]
fn leader_question_mark_toggles_the_panel_in_place_of_the_tree() {
    let mut app = app_with(with_side_chats());
    assert!(!app.side_panel_open());
    press(&mut app, " ?");
    assert!(app.side_panel_open());
    assert_eq!(app.focus(), RegionId::SideBottom);
    assert_eq!(app.panel_agent(), Some(&agent_id("side-1")));
    let screen = render(&mut app, 120, 34);
    // The conversation stays on the left, the inbox on top, the panel below it.
    assert!(screen.contains("┌ coordinator "), "{screen}");
    assert!(screen.contains("┌ inbox "), "{screen}");
    assert!(screen.contains("┌ side-1 "), "{screen}");
    assert!(!screen.contains("Now (2)"), "the tree gives way: {screen}");
    press(&mut app, " ?");
    assert!(!app.side_panel_open());
    let screen = render(&mut app, 120, 34);
    assert!(screen.contains("Now (2)"), "the tree is back: {screen}");
    assert!(!screen.contains("┌ side-1 "), "{screen}");
}

#[test]
fn an_empty_panel_says_how_to_start_a_side_chat() {
    let mut app = app_with(world());
    press(&mut app, " ?");
    let screen = render(&mut app, 120, 34);
    assert!(screen.contains("No side chats or tangents."), "{screen}");
}

#[test]
fn focus_keys_move_between_panel_conversation_and_inbox() {
    let mut app = panel_app();
    press(&mut app, "h");
    assert_eq!(app.focus(), RegionId::Main);
    press(&mut app, "l");
    assert_eq!(app.focus(), RegionId::SideBottom, "l returns to the panel");
    app.on_key(ctrl('k'), 0);
    assert_eq!(
        app.focus(),
        RegionId::SideTop,
        "Ctrl-k goes up to the inbox"
    );
    app.on_key(ctrl('j'), 0);
    assert_eq!(app.focus(), RegionId::SideBottom, "Ctrl-j comes back down");
    // In the panel, Ctrl-j goes to its chat bar, as in the conversation.
    app.on_key(ctrl('j'), 0);
    assert_eq!(app.mode(), Mode::Insert);
    assert_eq!(app.focus(), RegionId::SideBottom);
    app.on_key(ctrl('k'), 0);
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.focus(), RegionId::SideBottom);
}

#[test]
fn typing_in_the_panel_asks_the_side_chat_and_leaves_the_conversation_alone() {
    let mut app = panel_app();
    press(&mut app, "i");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "and the fix?");
    app.on_key(enter(), 100);
    assert_eq!(
        app.take_commands(),
        vec![Command::Conversation(ConversationCommand::Prompt {
            agent: agent_id("side-1"),
            text: "and the fix?".into(),
            attachments: Vec::new(),
            effort: None,
        })]
    );
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
    assert_eq!(app.focus(), RegionId::SideBottom);
    // `i` from the conversation still talks to the coordinator.
    app.on_key(esc(), 0);
    press(&mut app, "h");
    press(&mut app, "i");
    type_keys(&mut app, "hi");
    app.on_key(enter(), 200);
    assert_eq!(
        app.take_commands(),
        vec![Command::Conversation(ConversationCommand::Prompt {
            agent: agent_id("coordinator"),
            text: "hi".into(),
            attachments: Vec::new(),
            effort: None,
        })]
    );
}

#[test]
fn tab_switches_side_chats_inside_the_panel() {
    let mut app = panel_app();
    app.on_key(tab(), 0);
    assert_eq!(app.panel_agent(), Some(&agent_id("side-2")));
    let screen = render(&mut app, 120, 34);
    assert!(screen.contains("┌ side-2 "), "{screen}");
    assert!(screen.contains("2/2 ⇥"), "{screen}");
    app.on_key(tab(), 1);
    assert_eq!(app.panel_agent(), Some(&agent_id("side-1")), "it wraps");
    app.on_key(back_tab(), 2);
    assert_eq!(app.panel_agent(), Some(&agent_id("side-2")));
}

#[test]
fn dd_closes_the_side_chat_shown_and_the_panel_moves_on() {
    let mut app = panel_app();
    assert_eq!(
        press(&mut app, "dd"),
        vec![Command::Agent(AgentCommand::CloseSlice {
            agent: agent_id("side-1")
        })]
    );
    app.on_server(ServerMsg::Event {
        seq: 1,
        event: Event::Agent(AgentEvent::Removed(agent_id("side-1"))),
    });
    assert_eq!(app.panel_agent(), Some(&agent_id("side-2")));
    app.on_server(ServerMsg::Event {
        seq: 2,
        event: Event::Agent(AgentEvent::Removed(agent_id("side-2"))),
    });
    assert_eq!(app.panel_agent(), None);
    assert!(render(&mut app, 120, 34).contains("No side chats or tangents."));
}

#[test]
fn focus_mode_hides_the_panel_and_brings_it_back() {
    let mut app = panel_app();
    press(&mut app, " z");
    assert!(app.focus_mode());
    let screen = render(&mut app, 120, 34);
    assert!(!screen.contains("┌ side-1 "), "{screen}");
    press(&mut app, " z");
    assert!(app.side_panel_open());
    press(&mut app, "l");
    assert_eq!(app.focus(), RegionId::SideBottom);
    assert!(render(&mut app, 120, 34).contains("┌ side-1 "));
}

#[test]
fn ask_opens_the_new_side_chat_in_the_panel() {
    let mut app = app_with(world());
    type_keys(&mut app, ":ask why so slow?");
    app.on_key(enter(), 100);
    assert_eq!(
        app.take_commands(),
        vec![Command::Agent(AgentCommand::NewSideChat {
            question: "why so slow?".into()
        })]
    );
    let chat = agent("side-1", AgentKind::SideChat, AgentStatus::Working, None);
    app.on_server(ServerMsg::Event {
        seq: 1,
        event: Event::Agent(AgentEvent::Upserted(chat)),
    });
    assert!(app.side_panel_open());
    assert_eq!(app.panel_agent(), Some(&agent_id("side-1")));
    assert_eq!(app.focus(), RegionId::SideBottom);
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
}

#[test]
fn leader_y_is_gone() {
    let mut app = app_with(with_side_chats());
    assert!(press(&mut app, " y").is_empty());
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn a_side_chat_mode_does_not_cycle() {
    let mut app = panel_app();
    assert!(press(&mut app, " sm").is_empty());
    assert!(app.note().is_some_and(|n| n.contains("read-only")));
}

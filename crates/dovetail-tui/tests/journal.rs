//! The end-of-day journal in the UI (ADR 0012): "Close the day?" on quit and `:eod`.

mod support;

use dovetail_api::{
    Command, Event, ServerMsg, Snapshot,
    agents::agent::{AgentKind, AgentStatus},
    agents::conversation::{Attachment, ConversationCommand, ImageType},
    agents::journal::{JournalCommand, JournalEvent, JournalState},
    app::{AppEvent, Paste},
};
use support::*;

/// The b.png world with every session idle (so `:q` does not ask about a running turn), and a
/// journal whose newest file is from `latest`. `NOW_MS` is 2026-11-01 in UTC.
fn idle_world(latest: Option<&str>) -> Snapshot {
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    snap.journal = Some(JournalState {
        latest: latest.map(str::to_owned),
        closing: false,
        fresh: false,
    });
    snap
}

const PROMPT: &str = "Close the day? y: handoff and journal, then quit · n: quit now · Esc: later";

/// Runs a `:` command line and returns the commands it produced.
fn run(app: &mut dovetail_tui::App, line: &str) -> Vec<Command> {
    type_keys(app, &format!(":{line}"));
    app.on_key(enter(), 40);
    app.take_commands()
}

fn journal_event(app: &mut dovetail_tui::App, latest: Option<&str>, closing: bool) {
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::Journal(JournalEvent::Changed(JournalState {
            latest: latest.map(str::to_owned),
            closing,
            fresh: false,
        })),
    });
}

#[test]
fn quitting_on_a_day_with_no_journal_asks_to_close_it_and_n_quits_now() {
    let mut app = app_with(idle_world(Some("2026-10-31")));
    assert_eq!(run(&mut app, "q"), vec![]);
    assert!(!app.should_quit());
    assert!(render(&mut app, 120, 30).contains(PROMPT));
    assert_eq!(press(&mut app, "n"), vec![]);
    assert!(app.should_quit());
}

#[test]
fn esc_on_close_the_day_is_later() {
    let mut app = app_with(idle_world(None));
    run(&mut app, "q");
    app.on_key(esc(), 50);
    assert!(!app.should_quit());
    assert_eq!(app.take_commands(), vec![]);
    assert!(!render(&mut app, 120, 30).contains("Close the day?"));
}

#[test]
fn y_closes_the_day_and_quits_once_the_journal_is_written() {
    let mut app = app_with(idle_world(Some("2026-10-31")));
    run(&mut app, "q");
    assert_eq!(
        press(&mut app, "y"),
        vec![Command::Journal(JournalCommand::EndOfDay)]
    );
    assert!(!app.should_quit());
    journal_event(&mut app, Some("2026-10-31"), true);
    assert!(!app.should_quit(), "still waiting for the handoff");
    journal_event(&mut app, Some("2026-11-01"), false);
    assert!(app.should_quit());
}

#[test]
fn a_journal_that_was_not_written_does_not_quit() {
    let mut app = app_with(idle_world(Some("2026-10-31")));
    run(&mut app, "q");
    press(&mut app, "y");
    journal_event(&mut app, Some("2026-10-31"), true);
    journal_event(&mut app, Some("2026-10-31"), false);
    assert!(!app.should_quit());
    assert_eq!(
        app.note(),
        Some("the journal was not written; :q to quit anyway")
    );
}

#[test]
fn a_day_already_closed_quits_without_asking() {
    let mut app = app_with(idle_world(Some("2026-11-01")));
    run(&mut app, "q");
    assert!(app.should_quit());
}

#[test]
fn while_the_day_closes_quit_asks_only_about_the_running_turn() {
    let mut app = app_with(world());
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::Journal(JournalEvent::Changed(JournalState {
            latest: None,
            closing: true,
            fresh: false,
        })),
    });
    run(&mut app, "q");
    let screen = render(&mut app, 120, 30);
    assert!(!screen.contains("Close the day?"));
    assert!(screen.contains("A turn is running. Quit anyway? (y/n)"));
}

#[test]
fn n_on_close_the_day_still_asks_about_a_running_turn() {
    let mut snap = world();
    snap.journal = Some(JournalState::default());
    let mut app = app_with(snap);
    run(&mut app, "q");
    press(&mut app, "n");
    assert!(!app.should_quit());
    assert!(render(&mut app, 120, 30).contains("A turn is running. Quit anyway? (y/n)"));
    press(&mut app, "y");
    assert!(app.should_quit());
}

#[test]
fn colon_eod_closes_the_day_without_quitting() {
    let mut app = app_with(idle_world(None));
    assert_eq!(
        run(&mut app, "eod"),
        vec![Command::Journal(JournalCommand::EndOfDay)]
    );
    journal_event(&mut app, Some("2026-11-01"), false);
    assert!(!app.should_quit());
}

#[test]
fn a_folder_with_no_journal_never_asks() {
    let mut snap = idle_world(None);
    snap.journal = None;
    let mut app = app_with(snap);
    assert_eq!(run(&mut app, "eod"), vec![]);
    assert_eq!(
        app.note(),
        Some("not a Dovetail project yet, so it has no journal")
    );
    run(&mut app, "q");
    assert!(app.should_quit());
}

#[test]
fn a_failed_write_keeps_its_reason() {
    let mut app = app_with(idle_world(Some("2026-10-31")));
    run(&mut app, "q");
    press(&mut app, "y");
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::App(AppEvent::CommandFailed {
            message: "writing journal/2026-11-01.md: disk full".into(),
        }),
    });
    journal_event(&mut app, Some("2026-10-31"), false);
    assert!(!app.should_quit());
    assert_eq!(
        app.note(),
        Some("writing journal/2026-11-01.md: disk full; :q to quit anyway")
    );
}

#[test]
fn fresh_asks_the_core_for_a_mid_day_fresh_start() {
    let mut app = app_with(idle_world(None));
    assert_eq!(
        run(&mut app, "handoff"),
        vec![Command::Journal(JournalCommand::Handoff)]
    );
}

#[test]
fn fresh_asks_the_core_for_a_new_session_with_no_handoff() {
    let mut app = app_with(idle_world(None));
    assert_eq!(
        run(&mut app, "fresh"),
        vec![Command::Journal(JournalCommand::FreshSession)]
    );
}

/// Opens the coordinator's chat bar, types `text` and presses Enter.
fn chat_line(app: &mut dovetail_tui::App, text: &str) -> Vec<Command> {
    type_keys(app, " c");
    press(app, "i");
    type_keys(app, &format!("{text} "));
    app.on_key(enter(), 500);
    app.take_commands()
}

#[test]
fn slash_handoff_and_slash_fresh_run_the_commands_and_are_never_sent() {
    let mut app = app_with(idle_world(None));
    assert_eq!(
        chat_line(&mut app, "/handoff"),
        vec![Command::Journal(JournalCommand::Handoff)]
    );
    let mut app = app_with(idle_world(None));
    assert_eq!(
        chat_line(&mut app, "/fresh"),
        vec![Command::Journal(JournalCommand::FreshSession)]
    );
}

#[test]
fn slash_fresh_with_trailing_text_is_refused_and_the_text_stays_in_the_bar() {
    let mut app = app_with(idle_world(None));
    assert_eq!(chat_line(&mut app, "/fresh now"), vec![]);
    assert_eq!(
        app.note(),
        Some("/fresh takes nothing after it; nothing was sent")
    );
    assert_eq!(app.mode(), dovetail_tui::keymap::mode::Mode::Insert);
    assert!(render(&mut app, 120, 30).contains("/fresh now"));
}

#[test]
fn with_a_slice_focused_both_commands_send_nothing_and_say_why() {
    let mut app = app_with(idle_world(None));
    select(&mut app, "slice-4");
    assert_eq!(run(&mut app, "handoff"), vec![]);
    assert_eq!(
        app.note(),
        Some("`:handoff` and `:fresh` act on the coordinator")
    );
    assert_eq!(run(&mut app, "fresh"), vec![]);

    let mut app = app_with(idle_world(None));
    select(&mut app, "slice-4");
    press(&mut app, "i");
    type_keys(&mut app, "/fresh ");
    app.on_key(enter(), 500);
    assert_eq!(app.take_commands(), vec![]);
    assert_eq!(
        app.note(),
        Some("`:handoff` and `:fresh` act on the coordinator")
    );
}

/// The core's answer to a Ctrl-V: one small PNG for `who`.
fn paste_image(app: &mut dovetail_tui::App, who: &str) {
    app.on_key(ctrl('v'), 10);
    app.take_commands();
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::App(AppEvent::Pasted {
            agent: agent_id(who),
            paste: Paste::Image(Attachment {
                path: std::path::PathBuf::from("/data/attachments/1.png"),
                media_type: ImageType::Png,
                size: 24,
            }),
        }),
    });
}

#[test]
fn a_refused_slash_fresh_keeps_its_image_in_the_bar() {
    let mut app = app_with(idle_world(None));
    type_keys(&mut app, " c");
    press(&mut app, "i");
    type_keys(&mut app, "/fresh now");
    paste_image(&mut app, "coordinator");
    app.on_key(enter(), 500);
    assert_eq!(app.take_commands(), vec![]);
    assert!(render(&mut app, 120, 30).contains("[image 1"));
    // Sending it again, edited to a plain prompt, still carries the image.
    for _ in 0.."/fresh now".len() {
        app.on_key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Backspace,
                crossterm::event::KeyModifiers::NONE,
            ),
            501,
        );
    }
    type_keys(&mut app, "look");
    app.on_key(enter(), 502);
    let sent = app.take_commands();
    assert!(
        matches!(&sent[..], [Command::Conversation(ConversationCommand::Prompt { text, attachments, .. })]
            if text == "look" && attachments.len() == 1),
        "{sent:?}"
    );
}

#[test]
fn a_refused_slash_fresh_with_an_image_in_a_slice_does_not_claim_it_ran() {
    let mut app = app_with(idle_world(None));
    select(&mut app, "slice-4");
    press(&mut app, "i");
    type_keys(&mut app, "/fresh ");
    paste_image(&mut app, "slice-4");
    app.on_key(enter(), 500);
    assert_eq!(app.take_commands(), vec![]);
    assert_eq!(
        app.note(),
        Some("`:handoff` and `:fresh` act on the coordinator")
    );
}

/// The b.png world with the coordinator's context fill set.
fn world_at_context(pct: u8) -> Snapshot {
    let mut snap = idle_world(None);
    let coordinator = snap
        .agents
        .iter_mut()
        .find(|a| a.kind == AgentKind::Coordinator)
        .expect("a coordinator");
    coordinator.context_pct = Some(pct);
    snap
}

#[test]
fn the_status_bar_hints_at_fresh_above_80_percent_context() {
    let mut app = app_with(world_at_context(80));
    let at_80 = render(&mut app, 160, 30);
    assert!(at_80.contains("ctx 80%"), "{at_80}");
    assert!(!at_80.contains(":handoff"), "{at_80}");

    let mut app = app_with(world_at_context(81));
    let at_81 = render(&mut app, 160, 30);
    assert!(at_81.contains("ctx 81% :handoff"), "{at_81}");
}

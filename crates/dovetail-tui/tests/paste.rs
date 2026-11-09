//! Ctrl-V in the chat bar (task #1, moved into the core in task #13): the UI sends
//! `PasteClipboard`; the core's answer becomes a chip, text at the cursor, or a notice. Nowhere
//! else takes Ctrl-V.

mod support;

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::*;
use dovetail_mock::{MockConfig, MockCore};
use support::*;

const PNG_SIZE: u64 = 24;

fn image(n: u32) -> Paste {
    Paste::Image(Attachment {
        path: PathBuf::from(format!("/data/attachments/{n}.png")),
        media_type: ImageType::Png,
        size: PNG_SIZE,
    })
}

/// Ctrl-V, then the core's answer: the app must ask for the coordinator's chat bar.
fn paste(app: &mut dovetail_tui::App, at: u64, answer: Paste) {
    app.on_key(ctrl('v'), at);
    let agent = agent_id("coordinator");
    let commands = app.take_commands();
    assert!(
        commands.contains(&Command::PasteClipboard {
            agent: agent.clone()
        }),
        "{commands:?}"
    );
    reply(app, answer);
}

/// The core's answer to a paste, arriving as the next event.
fn reply(app: &mut dovetail_tui::App, answer: Paste) {
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::ClipboardPasted {
            agent: agent_id("coordinator"),
            paste: answer,
        },
    });
}

fn backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
}

#[test]
fn a_pasted_image_shows_as_a_chip_and_goes_out_with_the_prompt() {
    let mut app = app_with(world());
    press(&mut app, "i");
    paste(&mut app, 10, image(1));
    paste(&mut app, 11, image(2));
    let screen = render(&mut app, 100, 30);
    assert!(
        screen.contains("[image 1 · 1 KB]  [image 2 · 1 KB]"),
        "{screen}"
    );

    // Backspace on an empty input drops the last chip, then a third paste adds one back.
    app.on_key(backspace(), 12);
    assert!(!render(&mut app, 100, 30).contains("[image 2"));
    paste(&mut app, 13, image(3));
    type_keys(&mut app, "what is this?");
    app.on_key(enter(), 14);

    let commands = app.take_commands();
    let [
        Command::Prompt {
            agent,
            text,
            attachments,
        },
    ] = commands.as_slice()
    else {
        panic!("one prompt: {commands:?}");
    };
    assert_eq!(agent, &agent_id("coordinator"));
    assert_eq!(text, "what is this?");
    let paths: Vec<_> = attachments.iter().map(|a| a.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            PathBuf::from("/data/attachments/1.png"),
            PathBuf::from("/data/attachments/3.png")
        ]
    );
}

#[test]
fn an_image_only_message_is_sent_and_the_history_shows_its_chip() {
    let mut app = app_with(world());
    press(&mut app, "i");
    paste(&mut app, 10, image(1));
    app.on_key(enter(), 11);
    let commands = app.take_commands();
    let Some(Command::Prompt {
        text, attachments, ..
    }) = commands.first()
    else {
        panic!("a prompt: {commands:?}");
    };
    assert!(text.is_empty());
    assert_eq!(attachments.len(), 1);

    // The core echoes it back as a user entry: the transcript shows the same chip.
    let mut entry = entry(900, NOW_MS, EntryKind::User, "");
    entry.attachments = attachments.clone();
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::ConversationAppended {
            agent: agent_id("coordinator"),
            entry,
        },
    });
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("[image 1 · 1 KB]"), "{screen}");
}

#[test]
fn text_on_the_clipboard_is_pasted_at_the_cursor() {
    let mut app = app_with(world());
    press(&mut app, "i");
    type_keys(&mut app, "hello  world");
    for _ in 0.."world".len() {
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 1);
    }
    app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 1);
    paste(&mut app, 2, Paste::Text("brave new".into()));
    app.on_key(enter(), 3);
    assert_eq!(
        app.take_commands(),
        vec![Command::Prompt {
            agent: agent_id("coordinator"),
            text: "hello brave new world".into(),
            attachments: Vec::new(),
        }]
    );
}

#[test]
fn refusals_show_a_notice_and_attach_nothing() {
    let mut app = app_with(world());
    press(&mut app, "i");
    let bmp = "can't attach image/bmp: only PNG, JPEG, GIF or WebP";
    paste(&mut app, 1, Paste::Refused(bmp.into()));
    assert_eq!(app.note(), Some(bmp));
    // Nothing attached, nothing typed: Enter sends nothing.
    app.on_key(enter(), 3);
    assert!(app.take_commands().is_empty());
}

#[test]
fn at_most_ten_images_per_message() {
    let mut app = app_with(world());
    press(&mut app, "i");
    for t in 0..11 {
        paste(&mut app, t as u64, image(t));
    }
    assert_eq!(app.note(), Some("at most 10 images per message"));
    app.on_key(enter(), 20);
    let commands = app.take_commands();
    let Some(Command::Prompt { attachments, .. }) = commands.first() else {
        panic!("a prompt: {commands:?}");
    };
    assert_eq!(attachments.len(), MAX_ATTACHMENTS);
}

#[test]
fn ctrl_v_outside_the_chat_bar_asks_for_nothing() {
    let mut app = app_with(world());
    // Normal mode.
    app.on_key(ctrl('v'), 1);
    // The command row.
    press(&mut app, ":");
    app.on_key(ctrl('v'), 2);
    app.on_key(esc(), 3);
    assert!(
        !app.take_commands()
            .iter()
            .any(|c| matches!(c, Command::PasteClipboard { .. }))
    );
}

#[test]
fn esc_keeps_the_chips_with_the_draft() {
    let mut app = app_with(world());
    press(&mut app, "i");
    paste(&mut app, 1, image(1));
    app.on_key(esc(), 2);
    press(&mut app, "i");
    assert!(render(&mut app, 100, 30).contains("[image 1 · 1 KB]"));
}

#[test]
fn an_answer_that_arrives_after_esc_lands_in_the_draft() {
    let mut app = app_with(world());
    press(&mut app, "i");
    app.on_key(ctrl('v'), 1);
    app.on_key(esc(), 2);
    reply(&mut app, image(1));
    press(&mut app, "i");
    assert!(render(&mut app, 100, 30).contains("[image 1 · 1 KB]"));
}

/// The whole round trip through the mock core: Ctrl-V sends the command, the core answers
/// with its queued clipboard, the chip goes out with the prompt.
#[test]
fn a_paste_round_trips_through_the_mock_core() {
    let mut core = MockCore::new(MockConfig::default());
    let saved = Attachment {
        path: PathBuf::from("/data/attachments/mock.png"),
        media_type: ImageType::Png,
        size: PNG_SIZE,
    };
    core.queue_clipboard(Paste::Image(saved.clone()));
    let mut app = dovetail_tui::App::new(jiff::tz::TimeZone::UTC);
    for msg in core.connect() {
        app.on_server(msg);
    }
    let pump = |app: &mut dovetail_tui::App, core: &mut MockCore| {
        for command in app.take_commands() {
            core.handle(command);
        }
        while core.peek_delay() == 0 {
            app.on_server(core.pop());
        }
    };
    pump(&mut app, &mut core);
    press(&mut app, "i");
    app.on_key(ctrl('v'), 1);
    pump(&mut app, &mut core);
    assert!(render(&mut app, 100, 30).contains("[image 1 · 1 KB]"));

    // Nothing more queued: the mock refuses, like a core with no clipboard tool.
    app.on_key(ctrl('v'), 2);
    pump(&mut app, &mut core);
    assert_eq!(app.note(), Some("the mock core has no clipboard"));

    app.on_key(enter(), 3);
    let commands = app.take_commands();
    assert!(
        commands.iter().any(|c| matches!(
            c,
            Command::Prompt { attachments, .. } if attachments == &vec![saved.clone()]
        )),
        "{commands:?}"
    );
}

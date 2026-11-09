//! Ctrl-V in the chat bar (task #1): images become chips and go out with the prompt; text is
//! pasted at the cursor; nowhere else takes it.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::*;
use dovetail_tui::clipboard::{Clip, ClipError};
use support::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pretend-pixels-";

/// An app whose clipboard returns the queued clips in turn, saving images under a temp dir.
fn app_with_clips(clips: Vec<Result<Clip, ClipError>>) -> (dovetail_tui::App, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(world());
    app.set_attachments_dir(dir.path().join("attachments"));
    let queue = Rc::new(RefCell::new(clips));
    app.set_clipboard(Box::new(move || {
        let mut q = queue.borrow_mut();
        if q.is_empty() {
            Err(ClipError::Empty)
        } else {
            q.remove(0)
        }
    }));
    (app, dir)
}

fn png() -> Result<Clip, ClipError> {
    Ok(Clip::Image {
        media_type: ImageType::Png,
        bytes: PNG.to_vec(),
    })
}

fn backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
}

#[test]
fn a_pasted_image_shows_as_a_chip_and_goes_out_with_the_prompt() {
    let (mut app, dir) = app_with_clips(vec![png(), png(), png()]);
    press(&mut app, "i");
    app.on_key(ctrl('v'), 10);
    app.on_key(ctrl('v'), 11);
    let screen = render(&mut app, 100, 30);
    assert!(
        screen.contains("[image 1 · 1 KB]  [image 2 · 1 KB]"),
        "{screen}"
    );

    // Backspace on an empty input drops the last chip, then a third paste adds one back.
    app.on_key(backspace(), 12);
    assert!(!render(&mut app, 100, 30).contains("[image 2"));
    app.on_key(ctrl('v'), 13);
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
    assert_eq!(attachments.len(), 2);
    for a in attachments {
        assert_eq!(a.media_type, ImageType::Png);
        assert_eq!(a.size, PNG.len() as u64);
        assert!(a.path.starts_with(dir.path().join("attachments")));
        assert_eq!(std::fs::read(&a.path).expect("saved"), PNG);
    }
}

#[test]
fn an_image_only_message_is_sent_and_the_history_shows_its_chip() {
    let (mut app, _dir) = app_with_clips(vec![png()]);
    press(&mut app, "i");
    app.on_key(ctrl('v'), 10);
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
    app.on_server(ServerMsg::Event {
        seq: 1,
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
    let (mut app, _dir) = app_with_clips(vec![Ok(Clip::Text("brave new".into()))]);
    press(&mut app, "i");
    type_keys(&mut app, "hello  world");
    for _ in 0.."world".len() {
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 1);
    }
    app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 1);
    app.on_key(ctrl('v'), 2);
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
    let (mut app, _dir) = app_with_clips(vec![
        Err(ClipError::Unsupported("image/bmp".into())),
        Err(ClipError::NoTool),
    ]);
    press(&mut app, "i");
    app.on_key(ctrl('v'), 1);
    assert_eq!(
        app.note(),
        Some("can't attach image/bmp: only PNG, JPEG, GIF or WebP")
    );
    app.on_key(ctrl('v'), 2);
    assert_eq!(
        app.note(),
        Some("can't read the clipboard: install wl-clipboard or xclip")
    );
    // Nothing attached, nothing typed: Enter sends nothing.
    app.on_key(enter(), 3);
    assert!(app.take_commands().is_empty());
}

#[test]
fn at_most_ten_images_per_message() {
    let (mut app, _dir) = app_with_clips((0..11).map(|_| png()).collect());
    press(&mut app, "i");
    for t in 0..11 {
        app.on_key(ctrl('v'), t);
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
fn ctrl_v_outside_the_chat_bar_reads_nothing() {
    let (mut app, _dir) = app_with_clips(vec![Ok(Clip::Text("leak".into()))]);
    // Normal mode.
    app.on_key(ctrl('v'), 1);
    // The command row.
    press(&mut app, ":");
    app.on_key(ctrl('v'), 2);
    app.on_key(esc(), 3);
    // The clip is still queued: the chat bar is the first to read it.
    press(&mut app, "i");
    app.on_key(ctrl('v'), 4);
    app.on_key(enter(), 5);
    let commands = app.take_commands();
    assert!(
        matches!(commands.as_slice(), [Command::Prompt { text, .. }] if text == "leak"),
        "{commands:?}"
    );
}

#[test]
fn esc_keeps_the_chips_with_the_draft() {
    let (mut app, _dir) = app_with_clips(vec![png()]);
    press(&mut app, "i");
    app.on_key(ctrl('v'), 1);
    app.on_key(esc(), 2);
    press(&mut app, "i");
    assert!(render(&mut app, 100, 30).contains("[image 1 · 1 KB]"));
}

//! The conversation's entry cursor and per-entry folding: `zc`/`zo` act on the entry under the
//! cursor, the newest reply is never cut by the length rule, `zC`/`zO` fold the whole turn.

mod support;

use dovetail_api::*;
use support::*;

/// A prompt and a reply of 100 one-line paragraphs (about 200 rows), then optionally a second
/// prompt with a short reply, so the long reply is the newest one or an older one.
fn long_reply_app(then_a_second_turn: bool) -> dovetail_tui::App {
    let mut snap = world();
    let long = (0..100)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut entries = vec![
        entry(1, NOW_MS - 60_000, EntryKind::User, "first question"),
        entry(2, NOW_MS - 59_000, EntryKind::Assistant, &long),
    ];
    if then_a_second_turn {
        entries.push(entry(
            3,
            NOW_MS - 30_000,
            EntryKind::User,
            "second question",
        ));
        entries.push(entry(
            4,
            NOW_MS - 29_000,
            EntryKind::Assistant,
            "short answer",
        ));
    }
    snap.conversations.insert(
        agent_id("coordinator"),
        ConversationTail {
            entries,
            has_more: false,
        },
    );
    snap.agents[0].status = AgentStatus::Idle;
    let mut app = app_with(snap);
    app.on_key(key('h'), 0);
    app
}

const CUT: &str = "more lines · zo to show";

/// The text of the rows that carry the cursor bar, bar removed.
fn marked_rows(screen: &str) -> Vec<String> {
    screen
        .lines()
        .filter(|l| l.contains('▎'))
        .map(|l| l.replace('▎', "").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn the_newest_reply_is_never_cut_even_when_long() {
    let mut app = long_reply_app(false);
    render(&mut app, 100, 60);
    type_keys(&mut app, "gg");
    let top = render(&mut app, 100, 60);
    assert!(!top.contains(CUT), "{top}");
    type_keys(&mut app, "G");
    assert!(render(&mut app, 100, 60).contains("line 99"));
}

#[test]
fn an_older_long_reply_is_cut_once_a_newer_one_exists() {
    let mut app = long_reply_app(true);
    render(&mut app, 100, 60);
    type_keys(&mut app, "gg");
    let screen = render(&mut app, 100, 60);
    assert!(screen.contains(CUT), "{screen}");
    assert!(screen.contains("line 0"));
    assert!(!screen.contains("line 99"));
}

#[test]
fn zo_and_zc_act_on_the_long_entry_under_the_cursor() {
    let mut app = long_reply_app(true);
    render(&mut app, 100, 60);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 60);
    // The viewport top is the prompt, not the long reply: the cursor (middle of the view) is.
    type_keys(&mut app, "zo");
    let open = render(&mut app, 100, 60);
    assert!(!open.contains(CUT), "{open}");
    type_keys(&mut app, "zc");
    assert!(render(&mut app, 100, 60).contains(CUT));
}

#[test]
fn zc_cuts_the_newest_reply_when_asked_and_zo_restores_it() {
    let mut app = long_reply_app(false);
    render(&mut app, 100, 60);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 60);
    type_keys(&mut app, "zc");
    assert!(render(&mut app, 100, 60).contains(CUT));
    type_keys(&mut app, "zo");
    assert!(!render(&mut app, 100, 60).contains(CUT));
}

#[test]
fn the_cursor_bar_marks_one_entry_and_follows_the_movement_keys() {
    let mut app = long_reply_app(true);
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    let first = marked_rows(&render(&mut app, 100, 30));
    assert!(!first.is_empty());
    type_keys(&mut app, "G");
    let last = marked_rows(&render(&mut app, 100, 30));
    assert!(!last.is_empty());
    assert_ne!(first, last);
    // ]] [[ move it between turns too.
    type_keys(&mut app, "[[");
    let prev = marked_rows(&render(&mut app, 100, 30));
    assert_ne!(prev, last);
}

#[test]
fn zo_on_a_folded_turn_opens_the_turn() {
    let mut app = long_reply_app(true);
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 30);
    type_keys(&mut app, "zC");
    let folded = render(&mut app, 100, 30);
    assert!(folded.contains("entries folded"), "{folded}");
    type_keys(&mut app, "zo");
    assert!(!render(&mut app, 100, 30).contains("entries folded"));
}

#[test]
fn the_cursor_bar_snapshot() {
    let mut snap = world();
    snap.conversations.insert(
        agent_id("coordinator"),
        ConversationTail {
            entries: vec![
                entry(1, NOW_MS - 60_000, EntryKind::User, "what changed?"),
                entry(2, NOW_MS - 59_000, EntryKind::Assistant, "Two things."),
            ],
            has_more: false,
        },
    );
    snap.agents[0].status = AgentStatus::Idle;
    let mut app = app_with(snap);
    app.on_key(key('h'), 0);
    insta::assert_snapshot!(render(&mut app, 80, 24));
}

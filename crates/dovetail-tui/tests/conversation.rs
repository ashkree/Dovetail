//! The conversation buffer: reading column, scrolling and folding, chat input, agent switching.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::*;
use dovetail_tui::Mode;
use dovetail_tui::region::RegionId;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use support::*;

/// The b.png world with a rich coordinator conversation, focus on the main region.
fn reading_app() -> dovetail_tui::App {
    let mut snap = world();
    snap.conversations
        .insert(agent_id("coordinator"), rich_conversation(NOW_MS));
    snap.agents[0].status = AgentStatus::Idle;
    let mut app = app_with(snap);
    app.on_key(key('h'), 0);
    assert_eq!(app.focus(), RegionId::Main);
    app
}

fn cell_bg(app: &mut dovetail_tui::App, w: u16, h: u16, x: u16, y: u16) -> ratatui::style::Color {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    terminal.backend().buffer()[(x, y)].bg
}

// --- snapshots at three widths ---

#[test]
fn reading_at_80_columns() {
    insta::assert_snapshot!(render(&mut reading_app(), 80, 40));
}

#[test]
fn reading_at_120_columns() {
    insta::assert_snapshot!(render(&mut reading_app(), 120, 40));
}

#[test]
fn reading_at_200_columns() {
    insta::assert_snapshot!(render(&mut reading_app(), 200, 40));
}

#[test]
fn a_wide_pane_centres_the_reading_column_at_120_columns() {
    let mut app = reading_app();
    let screen = render(&mut app, 260, 40);
    let line = screen
        .lines()
        .find(|l| l.contains("What's blocking slice-7?"))
        .expect("prompt line");
    // Side column 98 wide on the right, main pane 162 on the left; less border and inset (2)
    // and CHAT_PADDING (2 a side) leaves 154; a 120 column centred in that starts 17 in.
    let at = line.chars().take_while(|c| *c != 'W').count();
    assert_eq!(at, 2 + 2 + 17, "{line}");
}

#[test]
fn rendering_shows_labels_blank_rows_and_folded_tool_lines() {
    let mut app = reading_app();
    let screen = render(&mut app, 120, 50);
    assert!(screen.contains("── You · 10:06 ─"));
    assert!(screen.contains("── coordinator · 10:14 ─"));
    assert!(screen.contains("▸ read 2 files · checked slice-7's stream · 4 s"));
    assert!(screen.contains("• the handler in"));
    assert!(screen.contains("│ Merge order: #1231 first, then the 500 fix."));
}

// --- colours ---

#[test]
fn bold_italic_and_code_get_their_own_colours_on_screen() {
    let mut app = reading_app();
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let theme = dovetail_tui::Theme::dusk();
    let buffer = terminal.backend().buffer();
    let find = |needle: &str| {
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if let Some(at) = row.find(needle) {
                let x = row[..at].chars().count() as u16;
                return buffer[(x, y)].clone();
            }
        }
        panic!("{needle} not on screen");
    };
    assert_eq!(find("It needs a key.").fg, theme.bold);
    assert_eq!(find("FS_EVAL_VIRTUAL_KEY").fg, theme.code);
    assert_eq!(find("eval path, which").fg, theme.italic);
    assert_eq!(
        find("Make sure slice-4").fg,
        theme.dim,
        "older turns are dimmed"
    );
}

// --- solid background ---

#[test]
fn the_solid_background_is_off_by_default_and_toggles_with_leader_b_and_solid() {
    let mut app = reading_app();
    let theme = dovetail_tui::Theme::dusk();
    let probe = |app: &mut dovetail_tui::App| cell_bg(app, 120, 40, 60, 5);
    assert_eq!(probe(&mut app), ratatui::style::Color::Reset);
    type_keys(&mut app, " B");
    assert_eq!(probe(&mut app), theme.reading_bg);
    type_keys(&mut app, ":solid");
    app.on_key(enter(), 99);
    assert_eq!(probe(&mut app), ratatui::style::Color::Reset);
}

// --- scrolling and turns ---

fn long_app() -> dovetail_tui::App {
    let mut snap = world();
    let mut entries = Vec::new();
    for turn in 0..12u64 {
        entries.push(entry(
            turn * 10 + 1,
            NOW_MS - 600_000 + turn as i64 * 1000,
            EntryKind::User,
            &format!("question {turn}"),
        ));
        entries.push(entry(
            turn * 10 + 2,
            NOW_MS - 600_000 + turn as i64 * 1000,
            EntryKind::Assistant,
            &format!("answer {turn}\n\nsecond paragraph {turn}\n\nthird paragraph {turn}"),
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

#[test]
fn the_view_follows_the_bottom_until_you_scroll_up() {
    let mut app = long_app();
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("third paragraph 11"));
    assert!(!screen.contains("question 0"));
    app.on_key(key('k'), 0);
    let screen = render(&mut app, 100, 30);
    assert!(
        screen.contains("third paragraph 11"),
        "one line up still shows the end"
    );
    app.on_key(key('G'), 1);
    assert!(render(&mut app, 100, 30).contains("third paragraph 11"));
}

#[test]
fn gg_goes_to_the_top_and_capital_g_back_to_the_bottom() {
    let mut app = long_app();
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    let top = render(&mut app, 100, 30);
    assert!(top.contains("question 0"));
    assert!(!top.contains("third paragraph 11"));
    app.on_key(key('G'), 9);
    assert!(render(&mut app, 100, 30).contains("third paragraph 11"));
}

#[test]
fn j_scrolls_down_and_stops_at_the_bottom() {
    let mut app = long_app();
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 30);
    for _ in 0..5 {
        app.on_key(key('j'), 0);
    }
    let screen = render(&mut app, 100, 30);
    assert!(
        !screen.contains("question 0"),
        "scrolled past the first line"
    );
    for _ in 0..500 {
        app.on_key(key('j'), 0);
    }
    assert!(render(&mut app, 100, 30).contains("third paragraph 11"));
}

#[test]
fn double_bracket_jumps_between_turns() {
    let mut app = long_app();
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 30);
    type_keys(&mut app, "]]");
    let screen = render(&mut app, 100, 30);
    let first_row = screen
        .lines()
        .nth(4)
        .expect("header row below the turn's blank row");
    assert!(first_row.contains("── You · "), "{first_row}");
    assert!(screen.contains("question 1"), "{screen}");
    type_keys(&mut app, "[[");
    assert!(render(&mut app, 100, 30).contains("question 0"));
}

#[test]
fn zc_folds_the_turn_and_zo_unfolds_it() {
    let mut app = long_app();
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 30);
    type_keys(&mut app, "zc");
    let folded = render(&mut app, 100, 30);
    assert!(folded.contains("question 0"));
    assert!(folded.contains("▸ 1 entries folded · zo to unfold"));
    assert!(!folded.contains("answer 0"));
    type_keys(&mut app, "zo");
    assert!(render(&mut app, 100, 30).contains("answer 0"));
}

#[test]
fn every_agent_keeps_its_own_place() {
    let mut app = long_app();
    render(&mut app, 100, 30);
    type_keys(&mut app, "gg");
    render(&mut app, 100, 30);
    // Open slice-7 and come back.
    type_keys(&mut app, " c");
    app.on_key(key('l'), 0);
    select(&mut app, "slice-7");
    app.on_key(enter(), 0);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-7")));
    render(&mut app, 100, 30);
    app.on_key(key(' '), 50);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 51);
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
    assert!(render(&mut app, 100, 30).contains("question 0"));
}

// --- agent switching ---

#[test]
fn leader_c_jumps_to_the_coordinator_and_leader_tab_alternates() {
    let mut app = app_with(world());
    select(&mut app, "slice-4");
    app.on_key(enter(), 0);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-4")));
    type_keys(&mut app, " c");
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
    assert_eq!(app.focus(), RegionId::Main);
    app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), 0);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 1);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-4")));
    app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), 5);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 6);
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
}

#[test]
fn alternating_with_no_previous_agent_says_so() {
    let mut app = app_with(world());
    app.on_key(key(' '), 0);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 1);
    assert_eq!(app.note(), Some("no previous agent"));
}

// --- insert mode (plan A11) ---

#[test]
fn i_from_the_tree_targets_the_coordinator_unless_an_agent_is_selected() {
    let mut app = app_with(world());
    press(&mut app, "i");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "hello");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::Prompt {
            agent: agent_id("coordinator"),
            text: "hello".into(),
            attachments: Vec::new()
        }]
    );
    // Typing moved focus to the main pane; `h` goes back to the tree.
    app.on_key(key('l'), 300);
    select(&mut app, "slice-7");
    press(&mut app, "i");
    type_keys(&mut app, "key is dev");
    app.on_key(enter(), 199);
    assert_eq!(
        app.take_commands(),
        vec![Command::Prompt {
            agent: agent_id("slice-7"),
            text: "key is dev".into(),
            attachments: Vec::new()
        }]
    );
    assert_eq!(
        app.open_agent(),
        Some(&agent_id("slice-7")),
        "typing to an agent shows it"
    );
}

#[test]
fn i_from_the_inbox_targets_the_coordinator() {
    let mut app = app_with(world());
    type_keys(&mut app, " n");
    press(&mut app, "i");
    type_keys(&mut app, "x");
    app.on_key(enter(), 50);
    assert_eq!(
        app.take_commands(),
        vec![Command::Prompt {
            agent: agent_id("coordinator"),
            text: "x".into(),
            attachments: Vec::new()
        }]
    );
}

#[test]
fn i_in_the_main_region_targets_the_agent_it_shows() {
    let mut app = app_with(world());
    select(&mut app, "slice-4");
    app.on_key(enter(), 0);
    app.take_commands();
    press(&mut app, "i");
    type_keys(&mut app, "go");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::Prompt {
            agent: agent_id("slice-4"),
            text: "go".into(),
            attachments: Vec::new()
        }]
    );
}

#[test]
fn the_chat_input_takes_enter_ctrl_j_alt_enter_ctrl_w_and_ctrl_u() {
    let mut app = app_with(world());
    press(&mut app, "i");
    type_keys(&mut app, "first line");
    app.on_key(ctrl('j'), 100);
    type_keys(&mut app, "second");
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), 101);
    type_keys(&mut app, "third of four");
    app.on_key(ctrl('w'), 102);
    app.on_key(ctrl('w'), 103);
    app.on_key(ctrl('u'), 104);
    type_keys(&mut app, "third");
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("› first line"), "{screen}");
    assert!(screen.contains("INSERT · coordinator"));
    app.on_key(enter(), 200);
    assert_eq!(
        app.take_commands(),
        vec![Command::Prompt {
            agent: agent_id("coordinator"),
            text: "first line\nsecond\nthird".into(),
            attachments: Vec::new()
        }]
    );
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn esc_keeps_the_draft_for_the_next_time() {
    let mut app = app_with(world());
    press(&mut app, "i");
    type_keys(&mut app, "half a thought");
    app.on_key(esc(), 50);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.take_commands().is_empty());
    press(&mut app, "i");
    assert!(render(&mut app, 100, 30).contains("› half a thought"));
}

#[test]
fn an_empty_message_sends_nothing() {
    let mut app = app_with(world());
    press(&mut app, "i");
    app.on_key(enter(), 1);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn a_long_chat_message_wraps_and_the_input_grows_to_a_limit() {
    let mut app = app_with(world());
    press(&mut app, "i");
    for _ in 0..30 {
        type_keys(&mut app, "word word word ");
    }
    let screen = render(&mut app, 100, 30);
    assert!(screen.lines().filter(|l| l.contains("word word")).count() <= 6);
    assert!(screen.contains("word"));
}

#[test]
fn the_chat_input_wraps_at_word_boundaries() {
    let mut app = app_with(world());
    press(&mut app, "i");
    let words = "please rebase the payments slice onto main, rerun the integration \
                 suite, and tell me whether the flaky checkout-redirect test \
                 still fails";
    type_keys(&mut app, words);
    let screen = render(&mut app, 80, 24);
    // The input rows, cut at the conversation pane's right border.
    let input: Vec<&str> = screen
        .lines()
        .filter_map(|l| l.strip_prefix('│')?.split('│').next())
        .skip_while(|l| !l.contains("› please"))
        .take_while(|l| !l.trim().is_empty())
        .collect();
    assert!(input.len() > 1, "{screen}");
    // Every row ends on a whole word: the words read back unbroken.
    let read: Vec<&str> = input
        .iter()
        .flat_map(|l| l.trim().trim_start_matches('›').split_whitespace())
        .collect();
    assert_eq!(
        read,
        words.split_whitespace().collect::<Vec<_>>(),
        "{screen}"
    );
    insta::assert_snapshot!(screen);
}

// --- streaming and history ---

#[test]
fn a_streamed_reply_grows_in_place_with_a_cursor() {
    let mut snap = world();
    snap.conversations
        .insert(agent_id("coordinator"), rich_conversation(NOW_MS));
    snap.conversations
        .get_mut(&agent_id("coordinator"))
        .expect("tail")
        .entries
        .push(entry(7, NOW_MS, EntryKind::Assistant, "Streaming"));
    let mut app = app_with(snap);
    let before = render(&mut app, 120, 50);
    assert!(before.contains("Streaming▌"), "{before}");
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::AssistantDelta {
            agent: agent_id("coordinator"),
            entry: EntryId(7),
            text: " now **bold**".into(),
        },
    });
    assert!(render(&mut app, 120, 50).contains("Streaming now bold▌"));
}

#[test]
fn reaching_the_top_with_older_history_asks_for_it_once() {
    let mut snap = world();
    let mut tail = rich_conversation(NOW_MS);
    tail.has_more = true;
    snap.conversations.insert(agent_id("coordinator"), tail);
    snap.agents[0].status = AgentStatus::Idle;
    let mut app = app_with(snap);
    app.on_key(key('h'), 0);
    render(&mut app, 120, 20);
    type_keys(&mut app, "gg");
    assert!(render(&mut app, 120, 20).contains("↑ older history"));
    let asked = app.take_commands();
    assert_eq!(
        asked,
        vec![Command::LoadConversation {
            agent: agent_id("coordinator"),
            before: Some(EntryId(1))
        }]
    );
    render(&mut app, 120, 20);
    type_keys(&mut app, "gg");
    assert!(app.take_commands().is_empty(), "asked only once");
}

#[test]
fn older_history_arriving_above_keeps_the_lines_being_read_in_place() {
    let mut snap = world();
    let mut tail = rich_conversation(NOW_MS);
    tail.has_more = true;
    snap.conversations.insert(agent_id("coordinator"), tail);
    snap.agents[0].status = AgentStatus::Idle;
    let mut app = app_with(snap);
    app.on_key(key('h'), 0);
    render(&mut app, 120, 14);
    type_keys(&mut app, "gg");
    app.take_commands();
    // Scroll down a little and remember what the top row says.
    for _ in 0..4 {
        app.on_key(key('j'), 0);
    }
    let before = render(&mut app, 120, 14);
    // The first row of the pane that has text on it.
    let row = |screen: &str| {
        screen
            .lines()
            .skip(1)
            .map(|line| {
                line.chars()
                    .skip(44)
                    .collect::<String>()
                    .trim_end_matches([' ', '│'])
                    .trim()
                    .to_string()
            })
            .find(|text| !text.is_empty())
            .unwrap_or_default()
    };
    let reading = row(&before);
    assert!(!reading.is_empty());
    // Ten older entries arrive.
    let older: Vec<_> = (0..10)
        .map(|i| {
            entry(
                100 + i,
                NOW_MS - 60 * 60_000,
                EntryKind::Assistant,
                &format!("ancient {i}"),
            )
        })
        .collect();
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::ConversationPage {
            agent: agent_id("coordinator"),
            entries: older,
            has_more: false,
        },
    });
    assert_eq!(row(&render(&mut app, 120, 14)), reading);
}

#[test]
fn i_from_the_tree_moves_focus_to_the_pane_that_holds_the_input() {
    let mut app = app_with(world());
    press(&mut app, "i");
    assert_eq!(app.focus(), RegionId::Main);
}

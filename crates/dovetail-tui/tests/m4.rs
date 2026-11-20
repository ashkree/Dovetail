//! M4: command mode, the picker and switcher, quick capture, task detail, the feeds, search,
//! the permission-mode cycle and context fill. Written across `App` and its buffers, so each
//! test covers a focus, mode or overlay change end to end.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::*;
use dovetail_tui::Mode;
use dovetail_tui::region::RegionId;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use support::*;

fn tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn leader(app: &mut dovetail_tui::App, keys: &str) {
    app.on_key(key(' '), 0);
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 1 + i as u64);
    }
}

fn cell_fg(app: &mut dovetail_tui::App, needle: &str) -> ratatui::style::Color {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(at) = row.find(needle) {
            let x = row[..at].chars().count() as u16;
            return buffer[(x, y)].fg;
        }
    }
    panic!("{needle} not on screen");
}

// --- command mode ---

#[test]
fn spawn_with_a_name_and_brief_starts_a_slice_on_the_focused_task() {
    let mut app = app_with(world());
    select(&mut app, "get_decision 500");
    type_keys(
        &mut app,
        ":spawn eval-fix reproduce the 500 with the key set",
    );
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::NewSlice {
            repo: None,
            task: Some(TaskId(2)),
            name: "eval-fix".into(),
            brief: "reproduce the 500 with the key set".into()
        }]
    );
}

#[test]
fn spawn_with_no_task_in_focus_makes_a_slice_with_no_task() {
    let mut app = app_with(world());
    type_keys(&mut app, ":spawn scratch");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::NewSlice {
            repo: None,
            task: None,
            name: "scratch".into(),
            brief: String::new()
        }]
    );
}

#[test]
fn leader_s_n_asks_for_a_name_then_a_brief() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    leader(&mut app, "sn");
    assert_eq!(app.mode(), Mode::Insert);
    assert!(render(&mut app, 120, 40).contains("slice name ›"));
    type_keys(&mut app, "keyflow");
    app.on_key(enter(), 50);
    assert!(render(&mut app, 120, 40).contains("brief for keyflow ›"));
    type_keys(&mut app, "sketch it");
    app.on_key(enter(), 100);
    assert_eq!(
        app.take_commands(),
        vec![Command::NewSlice {
            repo: None,
            task: Some(TaskId(3)),
            name: "keyflow".into(),
            brief: "sketch it".into()
        }]
    );
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn an_empty_slice_name_starts_nothing() {
    let mut app = app_with(world());
    leader(&mut app, "sn");
    app.on_key(enter(), 50);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn interrupt_works_by_key_by_command_and_by_name() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    leader(&mut app, "sx");
    assert_eq!(
        app.take_commands(),
        vec![Command::Interrupt {
            agent: agent_id("slice-7")
        }]
    );
    type_keys(&mut app, ":interrupt slice-4");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::Interrupt {
            agent: agent_id("slice-4")
        }]
    );
    type_keys(&mut app, ":interrupt nobody");
    app.on_key(enter(), 199);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.note(), Some("no agent named nobody"));
}

#[test]
fn interrupting_a_subagent_asks_before_stopping_its_session() {
    let mut app = app_with(world());
    select(&mut app, "rebase");
    leader(&mut app, "sx");
    assert!(
        app.take_commands().is_empty(),
        "nothing is sent before you answer"
    );
    let screen = render(&mut app, 120, 40);
    // rebase is done; explore callers is the one other running subagent.
    assert!(
        screen.contains("Interrupting rebase stops slice-4's turn and its 1 other running subagent. Go ahead? (y/n)"),
        "{screen}"
    );
    app.on_key(key('n'), 10);
    assert!(app.take_commands().is_empty());
    leader(&mut app, "sx");
    app.on_key(key('y'), 20);
    assert_eq!(
        app.take_commands(),
        vec![Command::Interrupt {
            agent: agent_id("slice-4")
        }]
    );
}

#[test]
fn colon_aliases_reach_the_same_actions_as_the_keys() {
    let mut app = app_with(world());
    type_keys(&mut app, ":inbox");
    app.on_key(enter(), 9);
    assert_eq!(app.focus(), RegionId::SideTop);
    type_keys(&mut app, ":activity");
    app.on_key(enter(), 19);
    assert_eq!(app.focus(), RegionId::Main);
    assert!(render(&mut app, 120, 30).contains("pushed to #1231"));
    type_keys(&mut app, ":coordinator");
    app.on_key(enter(), 29);
    assert!(
        render(&mut app, 120, 30).contains("coordinator · ○ idle")
            || render(&mut app, 120, 30).contains("coordinator ·")
    );
}

// --- permission mode ---

#[test]
fn leader_s_m_cycles_the_mode_of_the_agent_under_the_cursor() {
    let mut app = app_with(world());
    select(&mut app, "slice-4");
    leader(&mut app, "sm");
    assert_eq!(
        app.take_commands(),
        vec![Command::CyclePermissionMode {
            agent: agent_id("slice-4")
        }]
    );
    assert_eq!(app.note(), Some("slice-4: permission mode → manual"));
}

#[test]
fn the_mode_cycle_runs_auto_accept_edits_manual_and_around() {
    for (agent, expect) in [("coordinator", "acceptEdits"), ("slice-7", "auto")] {
        let mut app = app_with(world());
        select(&mut app, agent);
        leader(&mut app, "sm");
        let note = app.note().expect("a note").to_string();
        assert!(note.ends_with(expect), "{agent}: {note}");
    }
}

#[test]
fn on_a_subagent_the_mode_applies_to_its_session_with_a_note() {
    let mut app = app_with(world());
    select(&mut app, "rebase");
    leader(&mut app, "sm");
    assert_eq!(
        app.take_commands(),
        vec![Command::CyclePermissionMode {
            agent: agent_id("slice-4")
        }]
    );
    assert!(
        app.note()
            .expect("note")
            .contains("subagents share their session's")
    );
}

#[test]
fn the_badge_follows_the_mode_the_core_reports() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains(" edits "));
    assert!(screen.contains(" manual "));
    let mut agent = app
        .snapshot()
        .agents
        .iter()
        .find(|a| a.id == agent_id("slice-4"))
        .cloned()
        .expect("slice-4");
    agent.permission_mode = PermissionMode::Auto;
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::AgentUpserted(agent),
    });
    assert!(!render(&mut app, 120, 40).contains(" edits "));
}

// --- context fill ---

#[test]
fn context_fill_shows_in_the_status_bar_and_the_tree_with_amber_and_red() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 40);
    let bar = screen.lines().last().expect("status bar");
    assert!(bar.contains("ctx 62%"), "{bar}");
    let theme = dovetail_tui::Theme::default();
    assert_eq!(
        cell_fg(&mut app, "ctx 62%"),
        theme.needs_you,
        "62 % is amber"
    );
    assert_eq!(cell_fg(&mut app, "71%"), theme.needs_you);
    assert_eq!(cell_fg(&mut app, "85%"), theme.error, "above 80 % is red");
}

#[test]
fn subagents_show_no_context_fill() {
    let mut app = app_with(world());
    let screen = render(&mut app, 120, 40);
    let line = screen.lines().find(|l| l.contains("rebase")).expect("row");
    assert!(!line.contains('%'), "{line}");
}

// --- quick capture ---

#[test]
fn leader_t_opens_capture_and_enter_saves_with_its_tokens() {
    let mut app = app_with(world());
    leader(&mut app, "t");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "fix eval key !p1 @now #1231");
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("P1 · Now · PR #1231"), "{screen}");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::CreateTask(NewTask {
            title: "fix eval key".into(),
            p_level: Some(PLevel::P1),
            bucket: Some(Bucket::Now),
            notes: String::new(),
            follow_up_of: None,
            pr: Some(1231),
            related: vec![],
            effort: Default::default(),
            criterion: None,
        })]
    );
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(
        app.focus(),
        RegionId::SideBottom,
        "saving returns to where you were"
    );
}

#[test]
fn capture_without_a_bucket_says_unsorted() {
    let mut app = app_with(world());
    leader(&mut app, "t");
    type_keys(&mut app, "just a thought");
    assert!(render(&mut app, 120, 40).contains("no P-level · Unsorted"));
}

#[test]
fn the_link_to_the_focused_task_shows_and_ctrl_l_unlinks() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    leader(&mut app, "t");
    type_keys(&mut app, "follow up");
    assert!(render(&mut app, 120, 40).contains("linked to: get_decision 500"));
    app.on_key(ctrl('l'), 50);
    assert!(render(&mut app, 120, 40).contains("not linked to get_decision 500"));
    app.on_key(ctrl('l'), 51);
    app.on_key(enter(), 52);
    match app.take_commands().as_slice() {
        [Command::CreateTask(task)] => {
            assert_eq!(task.related, vec![TaskId(2)]);
            assert_eq!(
                task.follow_up_of, None,
                "the link relates, it does not make a follow-up"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn unlinked_capture_makes_a_task_with_no_relation() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    leader(&mut app, "t");
    type_keys(&mut app, "standalone");
    app.on_key(ctrl('l'), 50);
    app.on_key(enter(), 52);
    match app.take_commands().as_slice() {
        [Command::CreateTask(task)] => assert!(task.related.is_empty()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn capture_from_the_open_conversation_links_to_its_agents_task() {
    let mut app = app_with(world());
    select(&mut app, "slice-4");
    app.on_key(enter(), 0);
    assert_eq!(app.focus(), RegionId::Main);
    leader(&mut app, "t");
    assert!(render(&mut app, 120, 40).contains("linked to: business surface"));
}

#[test]
fn an_empty_capture_saves_nothing_and_says_why() {
    let mut app = app_with(world());
    leader(&mut app, "t");
    type_keys(&mut app, "!p1 @now");
    app.on_key(enter(), 50);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.note(), Some("a task needs a title"));
}

#[test]
fn esc_cancels_capture() {
    let mut app = app_with(world());
    leader(&mut app, "t");
    type_keys(&mut app, "never mind");
    app.on_key(esc(), 50);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.take_commands().is_empty());
    assert!(!render(&mut app, 120, 40).contains("new task"));
}

#[test]
fn tab_saves_and_opens_the_detail_once_the_core_announces_the_task() {
    let mut app = app_with(world());
    leader(&mut app, "t");
    type_keys(&mut app, "brand new idea !p2");
    app.on_key(tab(), 50);
    assert_eq!(app.take_commands().len(), 1);
    // Nothing opens until the task exists.
    assert!(!render(&mut app, 120, 40).contains("task · #"));
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::TaskUpserted(Task {
            id: TaskId(40),
            title: "brand new idea".into(),
            status: TaskStatus::Todo,
            p_level: Some(PLevel::P2),
            bucket: None,
            order: 40,
            deps: vec![],
            notes: String::new(),
            pr: None,
            follow_up_of: None,
            coordinator_created: false,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }),
    });
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("task · #40"), "{screen}");
    assert_eq!(app.focus(), RegionId::Main);
}

#[test]
fn the_tree_a_key_opens_the_same_capture() {
    let mut app = app_with(world());
    press(&mut app, "a");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "write the docs");
    app.on_key(enter(), 99);
    match app.take_commands().as_slice() {
        [Command::CreateTask(task)] => assert_eq!(task.title, "write the docs"),
        other => panic!("{other:?}"),
    }
}

// --- picker and switcher ---

#[test]
fn leader_s_s_opens_the_switcher_with_needs_you_agents_first_and_a_preview() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    assert_eq!(app.mode(), Mode::Insert);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("Switch"));
    assert!(screen.contains("Preview"));
    // The preview shows the highlighted agent's header.
    assert!(screen.contains("slice ·"), "{screen}");
    // With an empty query the first row is an agent that needs you, not the coordinator.
    app.on_key(enter(), 50);
    let opened = app.open_agent().cloned().expect("an agent opened");
    assert!(
        opened == agent_id("slice-7") || opened == agent_id("slice-4"),
        "{opened:?}"
    );
}

fn switcher_rows(screen: &str) -> Vec<String> {
    screen
        .lines()
        .filter(|l| l.contains('▌') || l.contains("└"))
        .map(str::to_string)
        .collect()
}

#[test]
fn the_switcher_indents_subagents_under_their_parent() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    let screen = render(&mut app, 140, 40);
    let lines: Vec<&str> = screen.lines().collect();
    let parent = lines
        .iter()
        .position(|l| l.contains("slice-4"))
        .expect("parent");
    let child = lines
        .iter()
        .position(|l| l.contains("└ rebase"))
        .expect("child");
    assert!(child > parent);
    let _ = switcher_rows(&screen);
}

#[test]
fn typing_filters_the_switcher_and_enter_opens_the_agent() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    type_keys(&mut app, "s7");
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("1/"), "{screen}");
    app.on_key(enter(), 50);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-7")));
    assert_eq!(app.focus(), RegionId::Main);
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn esc_closes_the_switcher_and_changes_nothing() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    app.on_key(esc(), 50);
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.focus(), RegionId::SideBottom);
    assert!(!render(&mut app, 120, 40).contains("Preview"));
}

#[test]
fn ctrl_j_and_ctrl_k_move_through_the_switcher() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    app.on_key(ctrl('j'), 50);
    app.on_key(enter(), 51);
    let second = app.open_agent().cloned().expect("an agent opened");
    let mut again = app_with(world());
    leader(&mut again, "ss");
    again.on_key(enter(), 51);
    let first = again.open_agent().cloned().expect("an agent opened");
    assert_ne!(first, second);
}

#[test]
fn the_finder_lists_tasks_agents_and_inbox_items() {
    let mut app = app_with(world());
    leader(&mut app, " ");
    assert_eq!(app.mode(), Mode::Insert);
    let screen = render(&mut app, 140, 50);
    assert!(screen.contains("Find"));
    for needle in [
        "Tasks",
        "Agents",
        "Inbox",
        "business surface",
        "slice-7 Bash",
    ] {
        assert!(screen.contains(needle), "{needle}\n{screen}");
    }
}

#[test]
fn choosing_a_task_in_the_finder_opens_its_detail() {
    let mut app = app_with(world());
    leader(&mut app, " ");
    type_keys(&mut app, "idea");
    app.on_key(enter(), 50);
    assert!(render(&mut app, 120, 30).contains("task · #3"));
}

#[test]
fn choosing_an_inbox_item_in_the_finder_jumps_to_it() {
    let mut app = app_with(world());
    leader(&mut app, " ");
    type_keys(&mut app, "Finished");
    app.on_key(enter(), 50);
    assert_eq!(app.focus(), RegionId::SideTop);
    // The finished turn is the selected item now: `x` dismisses it.
    assert_eq!(
        press(&mut app, "x"),
        vec![Command::DismissInbox { item: InboxId(3) }]
    );
}

#[test]
fn the_switcher_previews_a_background_subagent_with_its_progress() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    type_keys(&mut app, "explore");
    let screen = render(&mut app, 140, 40);
    assert!(screen.contains("Find callers of get_decision"), "{screen}");
    assert!(screen.contains("12 tool uses · last Grep"));
}

// --- task detail ---

#[test]
fn enter_on_a_task_opens_its_detail_and_esc_goes_back() {
    let mut app = app_with(world());
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    assert_eq!(app.focus(), RegionId::Main);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("task · #1"));
    assert!(screen.contains("P1 · Now · in progress · PR #1231"));
    assert!(screen.contains("Notes"));
    assert!(screen.contains("slice-4"), "its agents are listed");
    app.on_key(esc(), 9);
    assert!(render(&mut app, 120, 40).contains("coordinator ·"));
    assert!(!render(&mut app, 120, 40).contains("task · #1"));
}

#[test]
fn the_detail_cycles_status_bucket_and_p_level_and_stubs_notes() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    app.on_key(enter(), 0);
    assert_eq!(
        press(&mut app, "s"),
        vec![Command::UpdateTask {
            id: TaskId(3),
            patch: vec![TaskChange::Status(TaskStatus::InProgress)]
        }]
    );
    assert_eq!(
        press(&mut app, "b"),
        vec![Command::UpdateTask {
            id: TaskId(3),
            patch: vec![TaskChange::Bucket(Some(Bucket::Later))]
        }]
    );
    assert_eq!(
        press(&mut app, "p"),
        vec![Command::UpdateTask {
            id: TaskId(3),
            patch: vec![TaskChange::PLevel(Some(PLevel::P3))]
        }]
    );
    press(&mut app, "e");
    assert_eq!(
        app.note(),
        Some("editing notes in $EDITOR arrives with storage (M5)")
    );
}

#[test]
fn a_task_with_notes_shows_them_and_one_without_says_so() {
    let mut snap = world();
    snap.tasks[0].notes = "Eval lives in apps/backend.\nNever log the key.".into();
    let mut app = app_with(snap);
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("Eval lives in apps/backend."));
    app.on_key(esc(), 1);
    app.on_key(key('l'), 2);
    select(&mut app, "idea: eval key flow");
    app.on_key(enter(), 3);
    assert!(render(&mut app, 120, 40).contains("(no notes)"));
}

// --- feeds ---

#[test]
fn leader_a_shows_the_activity_feed_newest_first_and_the_same_key_goes_back() {
    let mut app = app_with(world());
    leader(&mut app, "a");
    assert_eq!(app.focus(), RegionId::Main);
    let screen = render(&mut app, 120, 30);
    let newest = screen.find("morning review applied").expect("newest");
    let oldest = screen.find("pushed to #1231").expect("oldest");
    assert!(newest < oldest);
    leader(&mut app, "a");
    assert!(!render(&mut app, 120, 30).contains("morning review applied"));
}

#[test]
fn enter_on_an_activity_row_opens_its_agent() {
    let mut app = app_with(world());
    leader(&mut app, "a");
    app.on_key(key('j'), 50);
    app.on_key(enter(), 51);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-4")));
    assert!(render(&mut app, 120, 30).contains("┌ slice-4 "));
}

#[test]
fn an_activity_row_with_no_agent_says_so() {
    let mut app = app_with(world());
    leader(&mut app, "a");
    app.on_key(enter(), 51);
    assert_eq!(app.note(), Some("no agent on this row"));
}

#[test]
fn leader_m_shows_the_message_timeline_with_names_not_ids() {
    let mut app = app_with(world());
    leader(&mut app, "m");
    let screen = render(&mut app, 120, 30);
    assert!(screen.contains("coordinator → slice-4"));
    assert!(screen.contains("slice-4 → coordinator"));
    app.on_key(esc(), 9);
    assert!(!render(&mut app, 120, 30).contains("messages between agents"));
}

#[test]
fn the_feeds_search_too() {
    let mut app = app_with(world());
    leader(&mut app, "a");
    type_keys(&mut app, "/CI");
    app.on_key(enter(), 60);
    assert_eq!(app.note(), Some("/CI  1/1"));
}

// --- search ---

#[test]
fn slash_n_and_shift_n_step_through_the_tree() {
    let mut app = app_with(world());
    type_keys(&mut app, "/slice");
    app.on_key(enter(), 50);
    let note = app.note().expect("a note").to_string();
    assert!(note.starts_with("/slice  1/"), "{note}");
    app.take_commands();
    let first = row_text(&mut app);
    app.on_key(key('n'), 60);
    let second = row_text(&mut app);
    assert_ne!(first, second);
    app.on_key(key('N'), 61);
    assert_eq!(row_text(&mut app), first);
}

#[test]
fn search_wraps_and_reports_no_match() {
    let mut app = app_with(world());
    type_keys(&mut app, "/slice-7");
    app.on_key(enter(), 50);
    assert!(row_text(&mut app).contains("slice-7"));
    app.on_key(key('n'), 60);
    assert!(
        row_text(&mut app).contains("slice-7"),
        "one match wraps onto itself"
    );
    type_keys(&mut app, "/zzzz");
    app.on_key(enter(), 70);
    assert_eq!(app.note(), Some("no match for zzzz"));
}

#[test]
fn n_without_a_search_says_to_press_slash() {
    let mut app = app_with(world());
    app.on_key(key('n'), 0);
    assert_eq!(app.note(), Some("no search yet: press / first"));
}

#[test]
fn search_in_the_inbox_moves_its_cursor() {
    let mut app = app_with(world());
    type_keys(&mut app, " n");
    type_keys(&mut app, "/Handler");
    app.on_key(enter(), 60);
    // The finished turn's preview says "Handler wired".
    assert_eq!(
        press(&mut app, "x"),
        vec![Command::DismissInbox { item: InboxId(3) }]
    );
}

#[test]
fn search_in_the_conversation_scrolls_to_the_match() {
    let mut snap = world();
    let mut entries = Vec::new();
    for i in 0..30u64 {
        entries.push(entry(
            i * 2 + 1,
            NOW_MS - 900_000,
            EntryKind::User,
            &format!("question {i}"),
        ));
        entries.push(entry(
            i * 2 + 2,
            NOW_MS - 900_000,
            EntryKind::Assistant,
            &format!("answer {i}"),
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
    render(&mut app, 120, 20);
    type_keys(&mut app, "gg");
    render(&mut app, 120, 20);
    type_keys(&mut app, "/answer 17");
    app.on_key(enter(), 90);
    assert!(render(&mut app, 120, 20).contains("answer 17"));
}

// --- the overlays take the keys ---

#[test]
fn keys_typed_in_an_overlay_do_not_reach_the_layout() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    type_keys(&mut app, "ddj:q");
    assert!(app.take_commands().is_empty());
    assert!(!app.should_quit());
}

#[test]
fn leader_tab_and_leader_c_still_work_after_an_overlay_closes() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    app.on_key(esc(), 50);
    leader(&mut app, "c");
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
}

// --- take-over (M14) ---

#[test]
fn leader_s_t_lends_the_terminal_for_the_session_under_the_cursor() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    leader(&mut app, "st");
    assert!(
        app.take_commands().is_empty(),
        "TakeOver waits for the hand-off"
    );
    assert_eq!(
        app.take_handoff(),
        Some(Command::TakeOver {
            agent: agent_id("slice-7")
        })
    );
    assert!(app.terminal_away());
}

#[test]
fn taking_over_a_session_mid_turn_asks_first_and_a_subagent_means_its_session() {
    let mut app = app_with(world());
    // `rebase` runs inside slice-4, which is mid-turn.
    select(&mut app, "rebase");
    leader(&mut app, "st");
    assert_eq!(app.take_handoff(), None, "asked first");
    let screen = render(&mut app, 120, 40);
    assert!(
        screen.contains("slice-4 (the session this subagent runs in) is mid-turn"),
        "{screen}"
    );
    app.on_key(key('y'), 20);
    assert_eq!(
        app.take_handoff(),
        Some(Command::TakeOver {
            agent: agent_id("slice-4")
        })
    );
}

#[test]
fn a_session_already_taken_over_says_so_and_shows_a_native_badge() {
    let mut snap = world();
    for agent in &mut snap.agents {
        if agent.name == "slice-7" {
            agent.lock = SessionLock::TakenOver;
        }
    }
    let mut app = app_with(snap);
    select(&mut app, "slice-7");
    // The side pane is narrow: the badge may be cut short there.
    assert!(
        row_text(&mut app).contains(" nati"),
        "{}",
        row_text(&mut app)
    );
    leader(&mut app, "st");
    assert_eq!(app.take_handoff(), None);
    assert_eq!(
        app.note(),
        Some("slice-7 is already taken over in a native terminal")
    );
}

#[test]
fn the_project_switcher_says_how_to_make_a_project_when_there_is_none() {
    let mut app = app_with(world());
    leader(&mut app, "pp");
    assert!(app.note().is_some_and(|n| n.contains(":project new")));
}

// --- snapshots of the overlays and new buffers ---

#[test]
fn snapshot_switcher_at_120() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    insta::assert_snapshot!(render(&mut app, 120, 34));
}

#[test]
fn snapshot_switcher_filtered_at_100() {
    let mut app = app_with(world());
    leader(&mut app, "ss");
    type_keys(&mut app, "slice");
    insta::assert_snapshot!(render(&mut app, 100, 30));
}

#[test]
fn snapshot_finder_at_120() {
    let mut app = app_with(world());
    leader(&mut app, " ");
    insta::assert_snapshot!(render(&mut app, 120, 38));
}

#[test]
fn snapshot_capture_at_100() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    leader(&mut app, "t");
    type_keys(&mut app, "check the eval key flow !p1 @next #1240");
    insta::assert_snapshot!(render(&mut app, 100, 28));
}

#[test]
fn snapshot_task_detail_at_120() {
    let mut snap = world();
    snap.tasks[0].notes = "Eval lives in apps/backend.\nNever log the key.".into();
    snap.tasks[0].deps = vec![TaskId(2)];
    let mut app = app_with(snap);
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    insta::assert_snapshot!(render(&mut app, 120, 32));
}

#[test]
fn snapshot_activity_at_100() {
    let mut app = app_with(world());
    leader(&mut app, "a");
    insta::assert_snapshot!(render(&mut app, 100, 24));
}

#[test]
fn snapshot_messages_at_100() {
    let mut app = app_with(world());
    leader(&mut app, "m");
    insta::assert_snapshot!(render(&mut app, 100, 24));
}

#[test]
fn the_switcher_opens_and_draws_at_once_even_with_a_hundred_agents() {
    let mut snap = world();
    for i in 0..100 {
        let mut a = snap.agents[1].clone();
        a.id = agent_id(&format!("extra-{i}"));
        a.name = format!("extra-{i}");
        a.last_activity_ms -= i * 1000;
        snap.agents.push(a);
    }
    let mut app = app_with(snap);
    let started = std::time::Instant::now();
    leader(&mut app, "ss");
    let screen = render(&mut app, 140, 40);
    let took = started.elapsed();
    assert!(screen.contains("Switch"));
    // Generous for a debug build on a busy machine; the real figure is a few milliseconds.
    assert!(took < std::time::Duration::from_millis(500), "{took:?}");
}

#[test]
fn a_failed_save_does_not_leave_tab_waiting_for_a_task_of_that_title() {
    let mut app = app_with(world());
    leader(&mut app, "t");
    type_keys(&mut app, "doomed");
    app.on_key(tab(), 50);
    app.take_commands();
    let mut seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::CommandFailed {
            message: "disk full".into(),
        },
    });
    seq += 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::TaskUpserted(Task {
            id: TaskId(41),
            title: "doomed".into(),
            status: TaskStatus::Todo,
            p_level: None,
            bucket: None,
            order: 41,
            deps: vec![],
            notes: String::new(),
            pr: None,
            follow_up_of: None,
            coordinator_created: false,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }),
    });
    assert!(!render(&mut app, 120, 30).contains("task · #41"));
}

#[test]
fn sm_and_sx_from_the_inbox_act_on_the_selected_items_agent() {
    let mut app = app_with(world());
    type_keys(&mut app, " n");
    leader(&mut app, "sx");
    assert_eq!(
        app.take_commands(),
        vec![Command::Interrupt {
            agent: agent_id("slice-7")
        }]
    );
    leader(&mut app, "sm");
    assert_eq!(
        app.take_commands(),
        vec![Command::CyclePermissionMode {
            agent: agent_id("slice-7")
        }]
    );
}

// --- related tasks and the decision log (decided 2026-11-03, ADR 0003) ---

fn with_decisions() -> Snapshot {
    let mut snap = world();
    snap.tasks[0].related = vec![TaskId(2)];
    snap.tasks[1].related = vec![TaskId(1)];
    snap.decisions = vec![
        Decision {
            id: DecisionId(1),
            task: TaskId(1),
            at_ms: NOW_MS - 3_600_000,
            by: DecisionAuthor::Agent(agent_id("coordinator")),
            text: "Sort every bucket by P-level".into(),
            why: None,
            alternatives: vec![],
            supersedes: None,
            evidence: Vec::new(),
        },
        Decision {
            id: DecisionId(2),
            task: TaskId(1),
            at_ms: NOW_MS - 60_000,
            by: DecisionAuthor::User,
            text: "Now keeps your own order".into(),
            why: Some("J and K reorder Now.".into()),
            alternatives: vec![Alternative {
                option: "P-level everywhere".into(),
                why_not: Some("J and K would do nothing".into()),
            }],
            supersedes: Some(DecisionId(1)),
            evidence: Vec::new(),
        },
    ];
    snap
}

#[test]
fn a_related_task_shows_a_count_chip_in_the_tree_and_its_title_in_the_detail() {
    let mut app = app_with(with_decisions());
    let screen = render(&mut app, 120, 40);
    let row = screen
        .lines()
        .find(|l| l.contains("business surface"))
        .expect("row");
    assert!(row.contains("↔ 1"), "{row}");
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    let detail = render(&mut app, 120, 60);
    assert!(detail.contains("Related"));
    assert!(detail.contains("↔ get_decision 500"), "{detail}");
}

#[test]
fn the_detail_lists_decisions_newest_first_with_why_alternatives_and_supersession() {
    let mut app = app_with(with_decisions());
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    let screen = render(&mut app, 120, 70);
    let newer = screen.find("Now keeps your own order").expect("newer");
    let older = screen.find("Sort every bucket by P-level").expect("older");
    assert!(newer < older, "newest first");
    assert!(screen.contains("you · #2"));
    assert!(screen.contains("coordinator · #1"));
    assert!(screen.contains("J and K reorder Now."));
    assert!(screen.contains("considered: P-level everywhere — J and K would do nothing"));
    assert!(screen.contains("superseded by #2"));
}

#[test]
fn a_superseded_decision_is_struck_through() {
    let mut app = app_with(with_decisions());
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 70)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let buffer = terminal.backend().buffer();
    let mut found = false;
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(at) = row.find("Sort every bucket") {
            let x = row[..at].chars().count() as u16;
            assert!(
                buffer[(x, y)]
                    .modifier
                    .contains(ratatui::style::Modifier::CROSSED_OUT)
            );
            found = true;
        }
    }
    assert!(found);
}

#[test]
fn a_in_the_detail_logs_a_decision_on_that_task() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    app.on_key(enter(), 0);
    press(&mut app, "a");
    assert!(render(&mut app, 120, 40).contains("decision ›"));
    type_keys(&mut app, "Ship it behind a flag");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::LogDecision(NewDecision {
            task: TaskId(3),
            text: "Ship it behind a flag".into(),
            why: None,
            alternatives: vec![],
            supersedes: None,
        })]
    );
}

#[test]
fn colon_decide_logs_on_the_task_in_focus_or_says_there_is_none() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    type_keys(&mut app, ":decide Key comes from the vault");
    app.on_key(enter(), 99);
    match app.take_commands().as_slice() {
        [Command::LogDecision(d)] => {
            assert_eq!(d.task, TaskId(2));
            assert_eq!(d.text, "Key comes from the vault");
        }
        other => panic!("{other:?}"),
    }
    // The coordinator row has no task.
    app.on_key(key('g'), 200);
    app.on_key(key('g'), 201);
    type_keys(&mut app, ":decide nothing");
    app.on_key(enter(), 299);
    assert!(app.take_commands().is_empty());
    assert_eq!(app.note(), Some("no task in focus to decide on"));
}

#[test]
fn capital_r_in_the_detail_relates_through_the_picker_and_unrelates_a_linked_one() {
    let mut app = app_with(with_decisions());
    select(&mut app, "business surface");
    app.on_key(enter(), 0);
    press(&mut app, "R");
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("Relate to"));
    assert!(screen.contains("related · ⏎ to unlink"));
    assert!(
        !screen
            .lines()
            .any(|l| l.contains("▌") && l.contains("business surface")),
        "itself is not offered"
    );
    type_keys(&mut app, "idea");
    app.on_key(enter(), 50);
    assert_eq!(
        app.take_commands(),
        vec![Command::UpdateTask {
            id: TaskId(1),
            patch: vec![TaskChange::Relate(TaskId(3))]
        }]
    );
    press(&mut app, "R");
    type_keys(&mut app, "get_decision");
    app.on_key(enter(), 90);
    assert_eq!(
        app.take_commands(),
        vec![Command::UpdateTask {
            id: TaskId(1),
            patch: vec![TaskChange::Unrelate(TaskId(2))]
        }]
    );
}

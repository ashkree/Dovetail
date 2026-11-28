//! Keys in, commands out: the mode machine and the tree and inbox keys, driven through `App`
//! the way the event loop drives it.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    Command, Event, ServerMsg, Snapshot,
    activity::inbox::{InboxCommand, InboxEvent, InboxId, InboxItem, InboxKind, PermissionAnswer},
    activity::status::{StatusEvent, Totals},
    agents::agent::AgentStatus,
    agents::conversation::ConversationCommand,
    app::{AppCommand, AppEvent},
    tracking::task::{Bucket, NewTask, PLevel, TaskChange, TaskCommand, TaskId, TaskStatus},
};
use dovetail_tui::Mode;
use dovetail_tui::keymap::{SEQUENCE_TIMEOUT_MS, WHICH_KEY_DELAY_MS};
use dovetail_tui::region::RegionId;
use support::*;

// --- mode machine ---

#[test]
fn colon_enters_command_mode_and_esc_returns_to_normal() {
    let mut app = app_with(world());
    assert_eq!(app.mode(), Mode::Normal);
    app.on_key(key(':'), 0);
    assert_eq!(app.mode(), Mode::Command);
    app.on_key(esc(), 1);
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn colon_q_enter_quits() {
    let mut app = app_with(world());
    // The b.png world has a working session, so :q asks first; use an idle world here.
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    app.on_server(ServerMsg::Snapshot(snap));
    type_keys(&mut app, ":q");
    assert!(!app.should_quit());
    app.on_key(enter(), 5);
    assert!(app.should_quit());
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn quitting_with_a_turn_running_asks_first() {
    let mut app = app_with(world());
    type_keys(&mut app, ":q");
    app.on_key(enter(), 5);
    assert!(!app.should_quit(), "a session is working");
    assert!(render(&mut app, 100, 30).contains("A turn is running. Quit anyway? (y/n)"));
    app.on_key(key('n'), 6);
    assert!(!app.should_quit());
    type_keys(&mut app, ":q");
    app.on_key(enter(), 8);
    app.on_key(key('y'), 9);
    assert!(app.should_quit());
}

#[test]
fn colon_q_bang_quits_without_asking() {
    let mut app = app_with(world());
    type_keys(&mut app, ":q!");
    app.on_key(enter(), 5);
    assert!(app.should_quit());
}

#[test]
fn an_unknown_command_shows_a_note_and_leaves_command_mode() {
    let mut app = app_with(world());
    type_keys(&mut app, ":nope");
    app.on_key(enter(), 5);
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.note(), Some("unknown command :nope"));
}

// --- focus (plan B2) ---

#[test]
fn focus_moves_with_h_l_and_ctrl_j_k() {
    let mut app = app_with(world());
    // Inbox on top, tree below (ADR 0008); focus starts on the tree.
    assert_eq!(app.focus(), RegionId::SideBottom);
    app.on_key(ctrl('k'), 0);
    assert_eq!(app.focus(), RegionId::SideTop);
    app.on_key(key('h'), 1);
    assert_eq!(app.focus(), RegionId::Main);
    app.on_key(key('l'), 2);
    assert_eq!(app.focus(), RegionId::SideTop);
    app.on_key(ctrl('j'), 3);
    assert_eq!(app.focus(), RegionId::SideBottom);
}

#[test]
fn leader_n_jumps_to_the_inbox() {
    let mut app = app_with(world());
    type_keys(&mut app, " n");
    assert_eq!(app.focus(), RegionId::SideTop);
}

#[test]
fn j_and_k_never_leave_a_list() {
    let mut app = app_with(world());
    for _ in 0..80 {
        app.on_key(key('j'), 0);
    }
    assert_eq!(app.focus(), RegionId::SideBottom);
    for _ in 0..80 {
        app.on_key(key('k'), 0);
    }
    assert_eq!(app.focus(), RegionId::SideBottom);
}

#[test]
fn a_half_typed_leader_opens_the_popup_then_waits_and_esc_drops_it() {
    let mut app = app_with(world());
    app.on_key(key(' '), 100);
    // The first deadline is the popup's, before the sequence gives up (task #19).
    assert_eq!(app.next_deadline_ms(), Some(100 + WHICH_KEY_DELAY_MS));
    const { assert!(WHICH_KEY_DELAY_MS < SEQUENCE_TIMEOUT_MS) };
    app.on_timeout(100 + WHICH_KEY_DELAY_MS);
    // With the list on screen the sequence waits for the user: no deadline.
    assert_eq!(app.next_deadline_ms(), None);
    app.on_key(esc(), 5_000);
    assert!(app.which_key().is_empty());
    // `n` alone is not bound, so nothing moves.
    app.on_key(key('n'), 6_000);
    assert_eq!(app.focus(), RegionId::SideBottom);
}

#[test]
fn an_idle_app_has_no_deadline_and_is_clean_after_a_draw() {
    let mut app = app_with(world());
    assert_eq!(app.next_deadline_ms(), None);
    render(&mut app, 100, 30);
    app.take_dirty();
    assert!(!app.is_dirty());
}

// --- tree keys ---

#[test]
fn enter_on_an_agent_opens_its_conversation_and_focuses_it() {
    let mut app = app_with(world());
    select(&mut app, "slice-7");
    app.on_key(enter(), 0);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-7")));
    assert_eq!(app.focus(), RegionId::Main);
}

#[test]
fn s_cycles_the_status_of_the_task_under_the_cursor() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    assert_eq!(
        press(&mut app, "s"),
        vec![Command::Task(TaskCommand::Update {
            id: TaskId(3),
            patch: vec![TaskChange::Status(TaskStatus::InProgress)]
        })]
    );
    // On an agent row the keys act on that agent's task.
    select(&mut app, "slice-7");
    assert_eq!(
        press(&mut app, "s"),
        vec![Command::Task(TaskCommand::Update {
            id: TaskId(2),
            patch: vec![TaskChange::Status(TaskStatus::Done)]
        })]
    );
}

#[test]
fn b_and_p_cycle_bucket_and_p_level() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    assert_eq!(
        press(&mut app, "b"),
        vec![Command::Task(TaskCommand::Update {
            id: TaskId(3),
            patch: vec![TaskChange::Bucket(Some(Bucket::Later))]
        })]
    );
    assert_eq!(
        press(&mut app, "p"),
        vec![Command::Task(TaskCommand::Update {
            id: TaskId(3),
            patch: vec![TaskChange::PLevel(Some(PLevel::P3))]
        })]
    );
}

#[test]
fn dd_archives_and_a_single_d_does_not() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    assert_eq!(press(&mut app, "d"), vec![]);
    assert_eq!(
        press(&mut app, "d"),
        vec![Command::Task(TaskCommand::Archive { id: TaskId(3) })]
    );
}

#[test]
fn j_and_k_reorder_within_now_only() {
    let mut app = app_with(world());
    select(&mut app, "business surface");
    assert_eq!(
        press(&mut app, "J"),
        vec![Command::Task(TaskCommand::Reorder {
            id: TaskId(1),
            delta: 1
        })]
    );
    select(&mut app, "idea: eval key flow");
    assert_eq!(press(&mut app, "K"), vec![]);
    assert_eq!(app.note(), Some("J and K reorder tasks within Now"));
}

#[test]
fn o_folds_a_group_and_hides_its_tasks() {
    let mut app = app_with(world());
    select(&mut app, "Next (1)");
    press(&mut app, "o");
    assert!(!render(&mut app, 120, 40).contains("idea: eval key flow"));
    press(&mut app, "o");
    assert!(render(&mut app, 120, 40).contains("idea: eval key flow"));
}

#[test]
fn a_adds_a_task_by_title() {
    let mut app = app_with(world());
    press(&mut app, "a");
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "write the docs");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::Task(TaskCommand::Create(NewTask {
            title: "write the docs".into(),
            p_level: None,
            bucket: None,
            notes: String::new(),
            follow_up_of: None,
            pr: None,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }))]
    );
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn e_edits_the_title_starting_from_the_old_one() {
    let mut app = app_with(world());
    select(&mut app, "get_decision 500");
    press(&mut app, "e");
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("title › get_decision 500"));
    type_keys(&mut app, "!");
    app.on_key(enter(), 99);
    assert_eq!(
        app.take_commands(),
        vec![Command::Task(TaskCommand::Update {
            id: TaskId(2),
            patch: vec![TaskChange::Title("get_decision 500!".into())]
        })]
    );
}

#[test]
fn landing_on_a_coordinator_created_task_marks_it_seen() {
    let mut snap = world();
    snap.tasks[2].coordinator_created = true;
    let mut app = app_with(snap);
    let mut commands = Vec::new();
    app.on_key(key('g'), 0);
    app.on_key(key('g'), 1);
    app.take_commands();
    for _ in 0..40 {
        app.on_key(key('j'), 2);
        commands.extend(app.take_commands());
    }
    assert!(commands.contains(&Command::Task(TaskCommand::Update {
        id: TaskId(3),
        patch: vec![TaskChange::Seen]
    })));
}

// --- inbox keys ---

fn in_inbox(app: &mut dovetail_tui::App) {
    type_keys(app, " n");
    assert_eq!(app.focus(), RegionId::SideTop);
}

#[test]
fn y_allows_the_selected_permission_request() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    assert_eq!(
        press(&mut app, "y"),
        vec![Command::Inbox(InboxCommand::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::Allow
        })]
    );
}

#[test]
fn shift_a_allows_always_and_d_denies() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    app.on_key(key('A'), 0);
    app.on_key(key('d'), 1);
    assert_eq!(
        app.take_commands(),
        vec![
            Command::Inbox(InboxCommand::AnswerPermission {
                item: InboxId(1),
                answer: PermissionAnswer::AllowAlways
            }),
            Command::Inbox(InboxCommand::AnswerPermission {
                item: InboxId(1),
                answer: PermissionAnswer::Deny { message: None }
            }),
        ]
    );
}

#[test]
fn always_allow_without_a_suggestion_is_disabled_with_a_note() {
    let mut snap = world();
    if let InboxKind::Permission { suggestions, .. } = &mut snap.inbox[0].kind {
        suggestions.clear();
    }
    let mut app = app_with(snap);
    in_inbox(&mut app);
    assert_eq!(press(&mut app, "A"), vec![]);
    assert_eq!(app.note(), Some("no rule was offered to always allow"));
}

#[test]
fn shift_d_opens_a_reason_prompt_that_denies_with_the_text() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    app.on_key(key('D'), 0);
    assert_eq!(app.mode(), Mode::Insert);
    type_keys(&mut app, "not on main");
    app.on_key(enter(), 50);
    assert_eq!(
        app.take_commands(),
        vec![Command::Inbox(InboxCommand::AnswerPermission {
            item: InboxId(1),
            answer: PermissionAnswer::Deny {
                message: Some("not on main".into())
            }
        })]
    );
}

#[test]
fn r_replies_to_a_question_and_x_dismisses_a_finished_turn() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    app.on_key(key('j'), 0);
    app.on_key(key('r'), 1);
    type_keys(&mut app, "/api");
    app.on_key(enter(), 50);
    assert_eq!(
        app.take_commands(),
        vec![Command::Inbox(InboxCommand::AnswerQuestion {
            item: InboxId(2),
            text: "/api".into()
        })]
    );
    app.on_key(key('j'), 60);
    assert_eq!(
        press(&mut app, "x"),
        vec![Command::Inbox(InboxCommand::Dismiss { item: InboxId(3) })]
    );
}

#[test]
fn r_on_a_finished_turn_sends_the_agent_a_normal_prompt() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    app.on_key(key('G'), 0);
    app.on_key(key('r'), 1);
    type_keys(&mut app, "go on");
    app.on_key(enter(), 50);
    assert_eq!(
        app.take_commands(),
        // Opening slice-4 to type to it also clears its finished turn (ADR 0001).
        vec![
            Command::Inbox(InboxCommand::Dismiss { item: InboxId(3) }),
            Command::Conversation(ConversationCommand::Prompt {
                agent: agent_id("slice-4"),
                text: "go on".into(),
                attachments: Vec::new(),
                effort: None,
            })
        ]
    );
}

#[test]
fn x_does_not_dismiss_a_permission_request() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    assert_eq!(press(&mut app, "x"), vec![]);
    assert!(app.note().is_some());
}

#[test]
fn opening_an_agent_from_the_inbox_clears_its_finished_turn() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    app.on_key(key('G'), 0);
    app.on_key(enter(), 1);
    assert_eq!(app.open_agent(), Some(&agent_id("slice-4")));
    assert_eq!(
        app.take_commands(),
        vec![Command::Inbox(InboxCommand::Dismiss { item: InboxId(3) })]
    );
}

#[test]
fn the_inbox_orders_permissions_then_questions_newest_first_then_the_rest() {
    let mut snap = world();
    snap.inbox.push(InboxItem {
        id: InboxId(4),
        agent: agent_id("slice-7"),
        at_ms: NOW_MS - 20 * 60_000,
        kind: InboxKind::Question {
            text: "Which key?".into(),
        },
    });
    let ids: Vec<u64> = dovetail_tui::buffers::inbox_order(&snap)
        .iter()
        .map(|i| i.id.0)
        .collect();
    assert_eq!(ids, vec![1, 2, 4, 3]);
}

#[test]
fn answering_moves_the_selection_to_the_neighbour() {
    let mut app = app_with(world());
    in_inbox(&mut app);
    app.on_key(key('y'), 0);
    app.take_commands();
    // The core resolves item 1.
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::Inbox(InboxEvent::Resolved { id: InboxId(1) }),
    });
    // Selection stays on the same row index: the question now.
    assert_eq!(
        press(&mut app, "r"),
        vec![],
        "r opens a prompt, it sends nothing yet"
    );
    type_keys(&mut app, "ok");
    app.on_key(enter(), 90);
    assert_eq!(
        app.take_commands(),
        vec![Command::Inbox(InboxCommand::AnswerQuestion {
            item: InboxId(2),
            text: "ok".into()
        })]
    );
}

// --- the stream ---

#[test]
fn a_gap_asks_for_one_resync_and_the_snapshot_clears_it() {
    let mut app = app_with(world());
    let seq = app.snapshot().seq;
    let skip = |n| ServerMsg::Event {
        seq: seq + n,
        event: Event::Status(StatusEvent::TotalsChanged(Totals::default())),
    };
    app.on_server(skip(5));
    app.on_server(skip(6));
    assert_eq!(app.take_commands(), vec![Command::App(AppCommand::Resync)]);
    app.on_server(ServerMsg::Snapshot(world()));
    app.on_server(skip(5));
    assert_eq!(app.take_commands(), vec![Command::App(AppCommand::Resync)]);
}

#[test]
fn a_failed_command_shows_in_the_status_bar() {
    let mut app = app_with(world());
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::App(AppEvent::CommandFailed {
            message: "no agent x".into(),
        }),
    });
    assert_eq!(app.note(), Some("no agent x"));
    assert!(render(&mut app, 100, 30).contains("no agent x"));
    // The note goes away on the next key.
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), 0);
    assert_eq!(app.note(), None);
}

#[test]
fn the_core_dropping_leaves_the_ui_up_with_a_note() {
    let mut app = app_with(world());
    app.on_core_closed();
    assert!(app.note().is_some());
    assert!(!app.should_quit());
}

#[test]
fn an_empty_snapshot_draws_without_panicking() {
    let mut app = dovetail_tui::App::new(jiff::tz::TimeZone::UTC);
    render(&mut app, 100, 30);
    render(&mut app, 20, 6);
    render(&mut app, 1, 1);
}

/// Live check 2026-11-06: while the user read the coordinator, each finished turn was dismissed
/// as it arrived (and again on every later event, giving "no inbox item N"), so in auto mode
/// Needs you stayed empty. Focusing the conversation clears its item once; a turn that
/// finishes while it already has focus stays (Decision 7a).
#[test]
fn a_turn_that_finishes_while_its_conversation_has_focus_stays_in_needs_you() {
    let mut app = app_with(world());
    app.on_key(key(' '), 0);
    app.on_key(key('c'), 1);
    assert_eq!(app.open_agent(), Some(&agent_id("coordinator")));
    app.take_commands();
    let mut seq = app.snapshot().seq;
    let mut event = |app: &mut dovetail_tui::App, event: Event| {
        seq += 1;
        app.on_server(ServerMsg::Event { seq, event });
    };
    event(
        &mut app,
        Event::Inbox(InboxEvent::Upserted(InboxItem {
            id: InboxId(10),
            agent: agent_id("coordinator"),
            at_ms: NOW_MS,
            kind: InboxKind::FinishedTurn {
                preview: "done".into(),
            },
        })),
    );
    event(
        &mut app,
        Event::Status(StatusEvent::TotalsChanged(Totals::default())),
    );
    assert_eq!(app.take_commands(), vec![], "no dismiss while reading");
    assert!(app.snapshot().inbox.iter().any(|i| i.id == InboxId(10)));

    // Leaving and coming back opens it: one dismiss, not one per event.
    app.on_key(key('l'), 2);
    app.on_key(key('h'), 3);
    event(
        &mut app,
        Event::Status(StatusEvent::TotalsChanged(Totals::default())),
    );
    assert_eq!(
        app.take_commands(),
        vec![Command::Inbox(InboxCommand::Dismiss { item: InboxId(10) })]
    );
}

// --- Ctrl-C and signals (task #13, decision A) ---

fn idle_world() -> Snapshot {
    let mut snap = world();
    for a in &mut snap.agents {
        a.status = AgentStatus::Idle;
    }
    snap
}

#[test]
fn double_ctrl_c_with_a_turn_running_asks_first() {
    let mut app = app_with(world());
    app.on_key(ctrl('c'), 0);
    assert_eq!(app.note(), Some("Press Ctrl-C again to exit"));
    app.on_key(ctrl('c'), 100);
    assert!(!app.should_quit(), "a session is working");
    assert!(render(&mut app, 100, 30).contains("A turn is running. Quit anyway? (y/n)"));
    app.on_key(key('n'), 200);
    assert!(!app.should_quit());

    app.on_key(ctrl('c'), 300);
    app.on_key(ctrl('c'), 400);
    app.on_key(key('y'), 500);
    assert!(app.should_quit());
}

#[test]
fn double_ctrl_c_with_nothing_running_quits_but_not_after_the_hint_expires() {
    use dovetail_tui::app::EXIT_CONFIRM_MS;
    let mut app = app_with(idle_world());
    app.on_key(ctrl('c'), 0);
    app.on_timeout(EXIT_CONFIRM_MS + 1);
    assert_eq!(app.note(), None);
    app.on_key(ctrl('c'), EXIT_CONFIRM_MS + 2);
    assert!(!app.should_quit(), "the first press had expired");
    // Another key in between disarms it.
    app.on_key(key('i'), EXIT_CONFIRM_MS + 3);
    assert_eq!(app.mode(), Mode::Insert);
    app.on_key(ctrl('c'), EXIT_CONFIRM_MS + 4);
    assert!(!app.should_quit());
    // Insert mode takes it too.
    app.on_key(ctrl('c'), EXIT_CONFIRM_MS + 5);
    assert!(app.should_quit());
}

#[test]
fn sigint_asks_like_q_and_sigterm_quits() {
    use dovetail_tui::app::Signal;
    let mut app = app_with(world());
    app.on_signal(Signal::Interrupt);
    assert!(!app.should_quit());
    assert!(render(&mut app, 100, 30).contains("A turn is running. Quit anyway? (y/n)"));
    // A second SIGINT while the question is open answers yes.
    app.on_signal(Signal::Interrupt);
    assert!(app.should_quit());

    let mut app = app_with(world());
    app.on_signal(Signal::Terminate);
    assert!(app.should_quit());

    let mut app = app_with(idle_world());
    app.on_signal(Signal::Interrupt);
    assert!(app.should_quit());
}

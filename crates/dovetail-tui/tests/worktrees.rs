//! M12c: the worktree view (`<leader>g`), driven by `MockCore` data. Opening it asks the core
//! for a refresh and says "refreshing…" until every worktree has answered; `Enter` opens the
//! slice, `e` lends the terminal to the editor, `c` copies the path, `X` removes the worktree
//! through the archive path, which asks first when work is unsaved.
//!
//! The mock's slices, in the view's (name) order: `inbox-keys` (clean, behind main, no PR),
//! `picker` (untracked files, failed setup, PR lookup failed) and `tree-view` (changes, one
//! unpushed commit, PR #1231 open).

mod support;

use dovetail_api::*;
use dovetail_mock::{MockConfig, MockCore};
use dovetail_tui::App;
use dovetail_tui::app::Signal;
use dovetail_tui::region::RegionId;
use support::*;

/// The mock core and an app on its snapshot (no worktree data yet, as on a fresh start).
fn start() -> (MockCore, App) {
    let core = MockCore::new(MockConfig::default());
    let app = app_with(core.state().clone());
    (core, app)
}

fn leader(app: &mut App, keys: &str) {
    app.on_key(key(' '), 0);
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 1 + i as u64);
    }
}

/// Hands the app every message the core has ready, as the event loop does.
fn deliver(core: &mut MockCore, app: &mut App) {
    for command in app.take_commands() {
        core.handle(command);
    }
    while core.peek_delay() == 0 {
        let Some(scheduled) = core.next_message() else {
            break;
        };
        app.on_server(scheduled.msg);
    }
}

/// The view open and refreshed, the cursor on its first row (`inbox-keys`).
fn view() -> (MockCore, App) {
    let (mut core, mut app) = start();
    leader(&mut app, "g");
    deliver(&mut core, &mut app);
    (core, app)
}

/// Moves the cursor onto `name`'s row.
fn on(app: &mut App, name: &str) {
    let order = ["inbox-keys", "picker", "tree-view"];
    let at = order.iter().position(|n| *n == name).expect("a mock slice");
    app.on_key(key('g'), 0);
    app.on_key(key('g'), 1);
    for _ in 0..at {
        app.on_key(key('j'), 2);
    }
}

fn line_with(screen: &str, needle: &str) -> String {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not on screen:\n{screen}"))
        .to_string()
}

#[test]
fn leader_g_opens_the_view_focused_asks_for_a_refresh_and_again_goes_back() {
    let (_core, mut app) = start();
    leader(&mut app, "g");
    assert_eq!(app.take_commands(), vec![Command::RefreshWorktrees]);
    assert_eq!(app.focus(), RegionId::Main);
    assert!(render(&mut app, 160, 40).contains("worktrees · Dovetail · 3"));
    leader(&mut app, "g");
    let back = app.take_commands();
    assert!(!back.contains(&Command::RefreshWorktrees), "{back:?}");
    assert!(!render(&mut app, 160, 40).contains("worktrees ·"));
}

#[test]
fn the_worktrees_command_opens_it_and_esc_goes_back() {
    let (_core, mut app) = start();
    app.on_key(key(':'), 0);
    type_keys(&mut app, "worktrees");
    app.on_key(enter(), 50);
    assert_eq!(app.take_commands(), vec![Command::RefreshWorktrees]);
    assert!(render(&mut app, 160, 40).contains("worktrees ·"));
    app.on_key(esc(), 60);
    assert!(!render(&mut app, 160, 40).contains("worktrees ·"));
}

#[test]
fn it_says_refreshing_with_dashes_until_every_worktree_has_answered() {
    let (mut core, mut app) = start();
    leader(&mut app, "g");
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("refreshing…"), "{screen}");
    let row = line_with(&screen, "tree-view");
    assert!(row.contains("dovetail/tree-view"), "{row}");
    assert_eq!(row.matches('—').count(), 5, "{row}");

    // Two of three answer: still refreshing.
    for command in app.take_commands() {
        core.handle(command);
    }
    for _ in 0..2 {
        let msg = core.next_message().expect("an answer").msg;
        app.on_server(msg);
    }
    assert!(render(&mut app, 160, 40).contains("refreshing…"));
    let msg = core.next_message().expect("the last answer").msg;
    app.on_server(msg);
    let screen = render(&mut app, 160, 40);
    assert!(!screen.contains("refreshing…"), "{screen}");
    let row = line_with(&screen, "tree-view");
    assert!(row.contains("#1231 open"), "{row}");
}

#[test]
fn a_part_the_core_could_not_tell_shows_a_dash() {
    let (_core, mut app) = view();
    let screen = render(&mut app, 160, 40);
    // picker: gh failed, so its PR is unknown; it has no remote base either, so no origin.
    let row = line_with(&screen, "picker");
    assert_eq!(row.matches('—').count(), 1, "{row}");
    assert!(!row.contains("origin"), "{row}");
    assert!(line_with(&screen, "inbox-keys  dovetail/").contains("origin ↓2"));
}

#[test]
fn enter_opens_the_slice_conversation() {
    let (_core, mut app) = view();
    on(&mut app, "picker");
    app.on_key(enter(), 10);
    assert_eq!(app.open_agent(), Some(&agent_id("picker")));
    assert_eq!(app.focus(), RegionId::Main);
    assert!(!render(&mut app, 160, 40).contains("worktrees ·"));
}

#[test]
fn x_on_unsaved_work_asks_first_and_yes_removes_it() {
    let (mut core, mut app) = view();
    on(&mut app, "tree-view");
    app.on_key(key('X'), 10);
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::CloseSlice {
            agent: agent_id("tree-view")
        }]
    );
    assert_eq!(app.note(), None, "nothing is said until the core answers");
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    assert_eq!(
        app.note(),
        Some("tree-view has unsaved work: waiting for your answer in Needs you")
    );
    let ask = app
        .snapshot()
        .inbox
        .iter()
        .find(|i| {
            i.agent == agent_id("tree-view")
                && matches!(&i.kind, InboxKind::Permission { tool, .. }
                    if tool == REMOVE_WORKTREE_PERMISSION)
        })
        .expect("a removal question in Needs you")
        .id;
    assert!(
        app.snapshot()
            .agents
            .iter()
            .any(|a| a.id == agent_id("tree-view")),
        "nothing removed before the answer"
    );
    core.handle(Command::AnswerPermission {
        item: ask,
        answer: PermissionAnswer::Allow,
    });
    deliver(&mut core, &mut app);
    assert!(
        !app.snapshot()
            .agents
            .iter()
            .any(|a| a.id == agent_id("tree-view"))
    );
    assert!(render(&mut app, 160, 40).contains("worktrees · Dovetail · 2"));
}

#[test]
fn x_on_a_clean_pushed_worktree_removes_it_at_once() {
    let (mut core, mut app) = view();
    // A new slice: clean, and nothing to push.
    core.handle(Command::NewSlice {
        task: None,
        name: "fresh".into(),
        brief: "try it".into(),
    });
    for _ in 0..200 {
        let ready = app
            .snapshot()
            .agents
            .iter()
            .any(|a| a.name == "fresh" && a.status != AgentStatus::SettingUp);
        if ready {
            break;
        }
        let msg = core.next_message().expect("runs").msg;
        app.on_server(msg);
    }
    // `fresh` sorts first.
    app.on_key(key('g'), 0);
    app.on_key(key('g'), 1);
    app.on_key(key('X'), 10);
    assert_eq!(app.note(), None, "nothing is said until the core answers");
    deliver(&mut core, &mut app);
    assert_eq!(app.note(), Some("removed fresh"));
    assert!(app.snapshot().inbox.iter().all(|i| !matches!(
        &i.kind,
        InboxKind::Permission { tool, .. } if tool == REMOVE_WORKTREE_PERMISSION
    )));
    assert!(!app.snapshot().agents.iter().any(|a| a.name == "fresh"));
}

#[test]
fn e_hands_off_the_terminal_and_takes_it_back_on_the_core_answer() {
    let (mut core, mut app) = view();
    on(&mut app, "tree-view");
    app.on_key(key('e'), 10);
    assert!(
        app.take_commands().is_empty(),
        "OpenEditor waits for the hand-off"
    );
    let path = app
        .snapshot()
        .agents
        .iter()
        .find(|a| a.id == agent_id("tree-view"))
        .and_then(|a| a.worktree.as_ref())
        .map(|w| w.path.clone())
        .expect("a worktree");
    let handoff = app.take_handoff().expect("a hand-off");
    assert_eq!(handoff, Command::OpenEditor { path });
    assert!(app.terminal_away());
    assert_eq!(app.take_handoff(), None, "one hand-off per key");

    // The mock opens no editor and says so; the terminal comes back either way.
    core.handle(handoff);
    deliver(&mut core, &mut app);
    assert!(!app.terminal_away());
    assert_eq!(app.note(), Some("the mock core opens no editor"));
}

#[test]
fn ctrl_c_in_the_editor_is_the_editor_s_but_sigterm_still_quits() {
    let (_core, mut app) = view();
    // tree-view is mid-turn, so a SIGINT would normally ask "Quit anyway?".
    on(&mut app, "tree-view");
    app.on_key(key('e'), 10);
    app.take_handoff().expect("a hand-off");
    app.on_signal(Signal::Interrupt);
    app.on_signal(Signal::Interrupt);
    assert!(!app.should_quit());
    assert!(app.terminal_away());
    assert!(!render(&mut app, 160, 40).contains("Quit anyway"));
    app.on_signal(Signal::Terminate);
    assert!(app.should_quit());
}

#[test]
fn c_copies_the_path() {
    let (_core, mut app) = view();
    on(&mut app, "picker");
    app.on_key(key('c'), 10);
    let path = "/home/you/.local/share/dovetail/projects/dovetail/worktrees/picker";
    assert_eq!(
        app.take_commands(),
        vec![Command::CopyToClipboard { text: path.into() }]
    );
    assert_eq!(app.note(), Some(format!("copied {path}").as_str()));
}

#[test]
fn snapshot_worktrees_refreshing_at_120() {
    let (_core, mut app) = start();
    leader(&mut app, "g");
    insta::assert_snapshot!(render(&mut app, 120, 34));
}

#[test]
fn snapshot_worktrees_at_120() {
    let (_core, mut app) = view();
    on(&mut app, "tree-view");
    insta::assert_snapshot!(render(&mut app, 120, 34));
}

#[test]
fn snapshot_worktrees_at_200() {
    let (_core, mut app) = view();
    on(&mut app, "picker");
    insta::assert_snapshot!(render(&mut app, 200, 40));
}

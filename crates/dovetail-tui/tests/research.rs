//! `:round` (R1): opens and closes research rounds through the core.

mod support;

use dovetail_api::{
    Command, Event, ServerMsg,
    inbox::{InboxId, InboxItem, InboxKind},
    task::TaskId,
};
use dovetail_api::{
    inbox::InboxCommand,
    research::{
        FindingId, FindingSummary, ResearchCommand, ResearchContent, ResearchDocument,
        ResearchEvent, ResearchListing, ResearchSummary, ResearchWhat, RoundSummary, SourceId,
        SourceKind, SourceSummary,
    },
    task::TaskCommand,
    terminal::{TerminalCommand, TerminalEvent},
};
use dovetail_tui::App;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use support::*;

const REPORT: &str = "# Per seat or flat?\n\n**Recommendation:** per seat [F1][S1]. Confidence: \
medium.\n\n## Evidence\n\n- Churn is lower on seat plans [F1, S2].\n- Flat plans leave money \
behind [3].\n\nA long paragraph so that the reading column has something to wrap at narrow \
widths: pricing research is mostly reading, and reading wants calm lines of text that do not run \
across a whole wide terminal.\n";

fn summary(task: i64, report: bool) -> ResearchSummary {
    ResearchSummary {
        task: TaskId(task),
        dir: "/data/research/1-pricing".into(),
        rounds: vec![
            RoundSummary {
                id: 1,
                queued: false,
                n: 1,
                brief: "per seat or flat?".into(),
                dir: "rounds/01-2026-11-10".into(),
                opened_ms: NOW_MS - 86_400_000,
                closed_ms: Some(NOW_MS - 3_600_000),
                findings: 2,
            },
            RoundSummary {
                id: 2,
                queued: false,
                n: 2,
                brief: "what does churn look like?".into(),
                dir: "rounds/02-2026-11-11".into(),
                opened_ms: NOW_MS - 600_000,
                closed_ms: None,
                findings: 0,
            },
        ],
        findings: 3,
        quick_findings: 1,
        sources: 2,
        report_ms: report.then_some(NOW_MS - 1_800_000),
        kind: None,
    }
}

fn listing() -> ResearchListing {
    let finding = |id, round, n, label: &str| FindingSummary {
        id: FindingId(id),
        round,
        n,
        agent: agent_id("rebase"),
        label: label.into(),
        path: format!("/data/research/1-pricing/findings/{id}-{label}.md").into(),
        created_ms: NOW_MS - 7_200_000,
        sources: vec![SourceId(1)],
    };
    ResearchListing {
        findings: vec![
            finding(1, Some(1), 1, "seat-plans"),
            finding(2, Some(1), 2, "flat-plans"),
            finding(3, None, 1, "churn-note"),
        ],
        sources: vec![
            SourceSummary {
                id: SourceId(1),
                kind: SourceKind::Web,
                locator: "https://example.com/pricing".into(),
                tool: "WebFetch".into(),
                path: Some("/data/research/1-pricing/sources/1.md".into()),
                is_error: false,
                fetched_ms: NOW_MS - 7_200_000,
            },
            SourceSummary {
                id: SourceId(2),
                kind: SourceKind::Search,
                locator: "seat pricing churn".into(),
                tool: "WebSearch".into(),
                path: None,
                is_error: false,
                fetched_ms: NOW_MS - 7_200_000,
            },
        ],
    }
}

fn tab() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    )
}

fn back_tab() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::BackTab,
        crossterm::event::KeyModifiers::SHIFT,
    )
}

fn deliver(app: &mut App, event: Event) {
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event { seq, event });
}

fn loaded(app: &mut App, what: ResearchWhat, path: Option<&str>, markdown: &str) {
    deliver(
        app,
        Event::Research(ResearchEvent::Loaded {
            task: TaskId(1),
            what,
            content: ResearchContent::Document(ResearchDocument {
                path: path.map(Into::into),
                markdown: markdown.into(),
                images: Vec::new(),
            }),
        }),
    );
}

/// The app with task 1's detail open and its listing answered.
fn detail(report: bool) -> App {
    let mut snap = world();
    snap.research.push(summary(1, report));
    let title = snap
        .tasks
        .iter()
        .find(|t| t.id == TaskId(1))
        .unwrap()
        .title
        .clone();
    let mut app = app_with(snap);
    select(&mut app, &title);
    app.on_key(enter(), 0);
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Listing,
        })]
    );
    deliver(
        &mut app,
        Event::Research(ResearchEvent::Loaded {
            task: TaskId(1),
            what: ResearchWhat::Listing,
            content: ResearchContent::Listing(listing()),
        }),
    );
    // The rows are known once drawn; the real loop draws after every message.
    render(&mut app, 120, 60);
    app
}

/// Presses `Enter` on the first research row (the report) and answers with the report.
fn open_report(app: &mut App) {
    app.on_key(tab(), 1);
    app.on_key(enter(), 2);
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Report,
        })]
    );
    loaded(
        app,
        ResearchWhat::Report,
        Some("/data/research/1-pricing/report.md"),
        REPORT,
    );
}

#[test]
fn a_document_in_the_task_details_reading_view_paints_no_background() {
    let mut app = detail(true);
    let probe = |app: &mut App| {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal.draw(|f| app.draw(f)).expect("draw");
        // Inside the reading column, away from the borders.
        let buffer = terminal.backend().buffer().clone();
        buffer[(60, 12)].bg
    };
    assert_eq!(probe(&mut app), ratatui::style::Color::Reset);
    open_report(&mut app);
    // Nothing paints a solid background (decided 2026-11-11 later).
    assert_eq!(probe(&mut app), ratatui::style::Color::Reset);
}

#[test]
fn the_task_detail_lists_report_rounds_findings_and_sources() {
    let mut app = detail(true);
    let screen = render(&mut app, 120, 60);
    for needle in [
        "Research",
        "2 rounds · 3 findings · 2 sources",
        "Report",
        "Round 1 · closed · 2 findings",
        "Round 2 · open · 0 findings",
        "F1   01 seat-plans",
        "Quick findings",
        "F3   01 churn-note",
        "Sources (2)",
        "S1   web",
        "S2   search",
    ] {
        assert!(screen.contains(needle), "{needle:?} missing:\n{screen}");
    }
}

#[test]
fn a_task_with_no_report_says_so_and_enter_has_nothing_to_open_there() {
    let mut app = detail(false);
    let screen = render(&mut app, 120, 60);
    assert!(screen.contains("no report yet"), "{screen}");
    // The first selectable row is round 1's brief, not a report.
    app.on_key(tab(), 1);
    app.on_key(enter(), 1);
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Brief { round: 1 },
        })]
    );
}

#[test]
fn a_task_without_research_shows_no_research_section() {
    let mut app = app_with(world());
    app.on_key(enter(), 0);
    let screen = render(&mut app, 120, 60);
    assert!(!screen.contains("Research"), "{screen}");
}

#[test]
fn every_row_opens_its_document_in_the_reading_view() {
    // Rows in order: report, brief 1, F1, F2, brief 2, F3, S1, S2.
    let expected = [
        ResearchWhat::Report,
        ResearchWhat::Brief { round: 1 },
        ResearchWhat::Finding(FindingId(1)),
        ResearchWhat::Finding(FindingId(2)),
        ResearchWhat::Brief { round: 2 },
        ResearchWhat::Finding(FindingId(3)),
        ResearchWhat::Source(SourceId(1)),
        ResearchWhat::Source(SourceId(2)),
    ];
    for (n, what) in expected.into_iter().enumerate() {
        let mut app = detail(true);
        for _ in 0..=n {
            app.on_key(tab(), 1);
        }
        app.on_key(enter(), 2);
        assert_eq!(
            app.take_commands(),
            vec![Command::Research(ResearchCommand::Load {
                task: TaskId(1),
                what: what.clone(),
            })],
            "row {n}"
        );
        loaded(
            &mut app,
            what,
            None,
            &format!("# Document {n}\n\nbody {n}\n"),
        );
        let screen = render(&mut app, 120, 30);
        assert!(
            screen.contains(&format!("Document {n}")),
            "row {n}:\n{screen}"
        );
        assert!(screen.contains("e edit"), "row {n}:\n{screen}");
    }
}

#[test]
fn the_reading_view_colours_citation_markers_and_nothing_else() {
    let mut app = detail(true);
    open_report(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let cite = dovetail_tui::Theme::default().cite;
    let mut coloured = String::new();
    let mut plain_brackets = 0;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.fg == cite {
                coloured.push_str(cell.symbol());
            }
        }
    }
    // The chosen colour is not used for anything but markers in the reading view's text.
    for needle in ["[F1]", "[S1]", "[F1, S2]", "[3]"] {
        assert!(
            coloured.contains(needle),
            "{needle} not coloured: {coloured:?}"
        );
        plain_brackets += 1;
    }
    assert_eq!(plain_brackets, 4);
    assert!(!coloured.contains("Recommendation"), "{coloured:?}");
}

#[test]
fn esc_goes_from_the_document_to_the_detail_and_then_out() {
    let mut app = detail(true);
    open_report(&mut app);
    assert!(render(&mut app, 120, 30).contains("Per seat or flat?"));
    app.on_key(esc(), 3);
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("Round 1 · closed"), "{screen}");
    assert!(!screen.contains("Per seat or flat?"), "{screen}");
    app.on_key(esc(), 4);
    let screen = render(&mut app, 120, 40);
    assert!(!screen.contains("task · #1"), "{screen}");
}

#[test]
fn e_hands_the_documents_file_to_the_editor_and_a_listed_source_has_none() {
    let mut app = detail(true);
    open_report(&mut app);
    app.on_key(key('e'), 3);
    assert_eq!(
        app.take_commands(),
        vec![Command::Terminal(TerminalCommand::OpenEditor {
            path: "/data/research/1-pricing/report.md".into()
        })]
    );
    deliver(&mut app, Event::Terminal(TerminalEvent::Lend));
    assert_eq!(
        app.take_handoff(),
        Some(Command::Terminal(TerminalCommand::Released))
    );
    deliver(
        &mut app,
        Event::Terminal(TerminalEvent::Returned { error: None }),
    );
    // A source that was listed but not cached has no file.
    app.on_key(esc(), 4);
    for _ in 0..7 {
        app.on_key(tab(), 5);
    }
    app.on_key(enter(), 6);
    app.take_commands();
    loaded(
        &mut app,
        ResearchWhat::Source(SourceId(2)),
        None,
        "Search: x\n",
    );
    app.on_key(key('e'), 7);
    assert_eq!(app.take_handoff(), None);
    assert!(app.note().unwrap_or_default().contains("no file"));
}

#[test]
fn gg_and_capital_g_move_through_a_long_document() {
    let mut app = detail(true);
    let long: String = (0..80).map(|n| format!("line {n}\n\n")).collect();
    app.on_key(tab(), 1);
    app.on_key(enter(), 2);
    app.take_commands();
    loaded(&mut app, ResearchWhat::Report, None, &long);
    assert!(render(&mut app, 100, 30).contains("line 0"));
    press(&mut app, "G");
    let screen = render(&mut app, 100, 30);
    assert!(
        screen.contains("line 79") && !screen.contains("line 0\n"),
        "{screen}"
    );
    press(&mut app, "gg");
    assert!(render(&mut app, 100, 30).contains("line 0"));
}

#[test]
fn colon_research_lists_the_tasks_that_have_research_and_enter_opens_one() {
    let mut snap = world();
    snap.research.push(summary(1, true));
    let mut app = app_with(snap);
    assert_eq!(run(&mut app, "research"), vec![]);
    let screen = render(&mut app, 200, 40);
    assert!(screen.contains("2 rounds · 3 findings"), "{screen}");
    app.on_key(enter(), 100);
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Listing,
        })]
    );
    assert!(render(&mut app, 120, 40).contains("task · #1"));
}

#[test]
fn colon_research_with_none_says_how_to_start() {
    let mut app = app_with(world());
    assert_eq!(run(&mut app, "research"), vec![]);
    assert!(app.note().unwrap_or_default().contains(":round"));
}

#[test]
fn snapshot_task_detail_with_research_at_80() {
    let mut app = detail(true);
    insta::assert_snapshot!(render(&mut app, 80, 50));
}

#[test]
fn snapshot_task_detail_with_research_at_120() {
    let mut app = detail(true);
    insta::assert_snapshot!(render(&mut app, 120, 50));
}

#[test]
fn snapshot_task_detail_with_research_at_200() {
    let mut app = detail(true);
    insta::assert_snapshot!(render(&mut app, 200, 50));
}

#[test]
fn snapshot_reading_a_report_at_80() {
    let mut app = detail(true);
    open_report(&mut app);
    insta::assert_snapshot!(render(&mut app, 80, 30));
}

#[test]
fn snapshot_reading_a_report_at_120() {
    let mut app = detail(true);
    open_report(&mut app);
    insta::assert_snapshot!(render(&mut app, 120, 30));
}

#[test]
fn snapshot_reading_a_report_at_200() {
    let mut app = detail(true);
    open_report(&mut app);
    insta::assert_snapshot!(render(&mut app, 200, 30));
}

fn run(app: &mut dovetail_tui::App, line: &str) -> Vec<Command> {
    type_keys(app, &format!(":{line}"));
    app.on_key(enter(), 40);
    app.take_commands()
}

#[test]
fn colon_round_opens_a_round_with_its_brief() {
    let mut app = app_with(world());
    assert_eq!(
        run(&mut app, "round #3 per seat or flat?"),
        vec![Command::Research(ResearchCommand::StartRound {
            task: TaskId(3),
            brief: "per seat or flat?".into(),
        })]
    );
    assert_eq!(run(&mut app, "round 3"), vec![]);
    assert!(app.note().unwrap_or_default().starts_with("usage: :round"));
}

#[test]
fn colon_round_close_closes_the_open_round() {
    let mut snap = world();
    assert_eq!(run(&mut app_with(snap.clone()), "round close"), vec![]);
    snap.research.push(ResearchSummary {
        task: TaskId(3),
        dir: "/r/3-x".into(),
        rounds: vec![RoundSummary {
            id: 1,
            queued: false,
            n: 1,
            brief: "b".into(),
            dir: "rounds/01-2026-11-11".into(),
            opened_ms: 0,
            closed_ms: None,
            findings: 0,
        }],
        findings: 0,
        quick_findings: 0,
        sources: 0,
        report_ms: None,
        kind: None,
    });
    let mut app = app_with(snap);
    assert_eq!(
        run(&mut app, "round close"),
        vec![Command::Research(ResearchCommand::CloseRound {
            task: TaskId(3)
        })]
    );
}

// --- follow-ups of 2026-11-11 --------------------------------------------------------------

#[test]
fn j_and_k_scroll_line_by_line_and_do_not_pick_rows() {
    let mut app = detail(true);
    // Short terminal: the detail scrolls.
    let top = render(&mut app, 100, 14);
    assert!(top.contains("task · #1"), "{top}");
    for _ in 0..6 {
        app.on_key(key('j'), 1);
    }
    let scrolled = render(&mut app, 100, 14);
    assert_ne!(top, scrolled);
    // `j` picked no row, so `Enter` has nothing to open.
    assert_eq!(app.on_key(enter(), 2), ());
    assert_eq!(app.take_commands(), vec![]);
    assert!(
        app.note().unwrap_or_default().contains("Tab picks"),
        "{:?}",
        app.note()
    );
    for _ in 0..6 {
        app.on_key(key('k'), 3);
    }
    assert_eq!(render(&mut app, 100, 14), top);
}

#[test]
fn tab_and_shift_tab_jump_between_research_rows_and_enter_opens_the_selected_one() {
    let mut app = detail(true);
    app.on_key(tab(), 1);
    app.on_key(tab(), 2);
    app.on_key(enter(), 3);
    // Report, then round 1's brief.
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Brief { round: 1 },
        })]
    );
    let mut app = detail(true);
    // Shift-Tab from nothing lands on the last row, the second source.
    app.on_key(back_tab(), 1);
    app.on_key(enter(), 2);
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Source(SourceId(2)),
        })]
    );
    // And it wraps round.
    let mut app = detail(true);
    app.on_key(back_tab(), 1);
    app.on_key(tab(), 2);
    app.on_key(enter(), 3);
    assert_eq!(
        app.take_commands(),
        vec![Command::Research(ResearchCommand::Load {
            task: TaskId(1),
            what: ResearchWhat::Report,
        })]
    );
}

#[test]
fn e_on_a_selected_research_row_opens_that_file_in_the_editor() {
    let mut app = detail(true);
    // Report, brief 1, F1.
    for _ in 0..3 {
        app.on_key(tab(), 1);
    }
    app.on_key(key('e'), 2);
    assert_eq!(
        app.take_commands(),
        vec![Command::Terminal(TerminalCommand::OpenEditor {
            path: "/data/research/1-pricing/findings/1-seat-plans.md".into()
        })]
    );
    deliver(&mut app, Event::Terminal(TerminalEvent::Lend));
    assert_eq!(
        app.take_handoff(),
        Some(Command::Terminal(TerminalCommand::Released))
    );
    deliver(
        &mut app,
        Event::Terminal(TerminalEvent::Returned { error: None }),
    );
}

#[test]
fn e_with_no_row_selected_is_still_the_notes_action() {
    let mut app = detail(true);
    app.on_key(key('e'), 1);
    assert_eq!(app.take_handoff(), None);
    assert!(
        app.note().unwrap_or_default().contains("notes"),
        "{:?}",
        app.note()
    );
}

fn queued_round(id: i64, n: u32) -> RoundSummary {
    RoundSummary {
        id,
        queued: true,
        n,
        brief: "queued criteria".into(),
        dir: format!("rounds/{n:02}-2026-11-11").into(),
        opened_ms: NOW_MS,
        closed_ms: None,
        findings: 0,
    }
}

#[test]
fn a_queued_round_shows_its_place_in_line() {
    let mut snap = world();
    let mut a = summary(1, true);
    a.rounds.push(queued_round(5, 3));
    snap.research.push(a);
    let mut b = summary(3, false);
    b.rounds.clear();
    b.rounds.push(queued_round(4, 1));
    snap.research.push(b);
    let mut app = app_with(snap);
    // Task 3's round was requested first: 2nd in line (the open round is 1st); task 1's is 3rd.
    select(&mut app, "idea: eval key flow");
    app.on_key(enter(), 0);
    let screen = render(&mut app, 120, 60);
    assert!(
        screen.contains("Round 1 · queued (2nd in line)"),
        "{screen}"
    );
    assert!(
        !screen.contains("1 round ·"),
        "queued is not a round yet:\n{screen}"
    );
}

#[test]
fn archiving_a_task_with_an_open_round_asks_first() {
    let mut snap = world();
    snap.research.push(ResearchSummary {
        task: TaskId(3),
        dir: "/r/3-x".into(),
        rounds: vec![RoundSummary {
            id: 1,
            queued: false,
            n: 1,
            brief: "b".into(),
            dir: "rounds/01-2026-11-11".into(),
            opened_ms: 0,
            closed_ms: None,
            findings: 0,
        }],
        findings: 0,
        quick_findings: 0,
        sources: 0,
        report_ms: None,
        kind: None,
    });
    let mut app = app_with(snap);
    select(&mut app, "idea: eval key flow");
    press(&mut app, "d");
    assert_eq!(press(&mut app, "d"), vec![]);
    let screen = render(&mut app, 120, 40);
    assert!(
        screen.contains("Task #3 has an open research round. Close it and archive? y/n"),
        "{screen}"
    );
    // `n` keeps the task.
    app.on_key(key('n'), 50);
    assert_eq!(app.take_commands(), vec![]);
    press(&mut app, "d");
    press(&mut app, "d");
    app.on_key(key('y'), 51);
    assert_eq!(
        app.take_commands(),
        vec![Command::Task(TaskCommand::Archive { id: TaskId(3) })]
    );
}

#[test]
fn colon_round_drop_drops_a_queued_round_and_only_a_queued_one() {
    let mut snap = world();
    snap.research.push(ResearchSummary {
        task: TaskId(3),
        dir: "/r/3-x".into(),
        rounds: vec![queued_round(9, 2)],
        findings: 0,
        quick_findings: 0,
        sources: 0,
        report_ms: None,
        kind: None,
    });
    let mut app = app_with(snap);
    assert_eq!(
        run(&mut app, "round drop 3"),
        vec![Command::Research(ResearchCommand::DropRound {
            task: TaskId(3),
            round: None
        })]
    );
    assert_eq!(
        run(&mut app, "round drop #3 2"),
        vec![Command::Research(ResearchCommand::DropRound {
            task: TaskId(3),
            round: Some(2)
        })]
    );
    assert_eq!(run(&mut app, "round drop 3 7"), vec![]);
    assert!(app.note().unwrap_or_default().contains("no such queued"));
    assert_eq!(run(&mut app, "round drop 4"), vec![]);
}

#[test]
fn colon_research_kind_sets_the_kind_of_the_selected_task() {
    let mut app = app_with(world());
    select(&mut app, "idea: eval key flow");
    assert_eq!(
        run(&mut app, "research kind decision"),
        vec![Command::Research(ResearchCommand::SetKind {
            task: TaskId(3),
            kind: dovetail_api::research::ResearchKind::Decision,
        })]
    );
    assert_eq!(run(&mut app, "research kind soon"), vec![]);
    assert!(
        app.note()
            .unwrap_or_default()
            .contains("usage: :research kind")
    );
}

fn unfiled_item(id: u64) -> InboxItem {
    InboxItem {
        id: InboxId(id),
        agent: agent_id("coordinator"),
        at_ms: NOW_MS,
        kind: InboxKind::UnfiledFinding {
            finding: 7,
            label: "pricing-pages".into(),
            sources: 3,
        },
    }
}

#[test]
fn an_unfiled_finding_is_filed_from_the_inbox_through_a_task_picker() {
    let mut snap = world();
    snap.inbox.clear();
    snap.inbox.push(unfiled_item(40));
    let mut app = app_with(snap);
    press(&mut app, " n");
    let screen = render(&mut app, 140, 40);
    assert!(screen.contains("Unfiled finding: 'pricing"), "{screen}");
    // `y` opens the picker over tasks; Enter chooses the first row.
    assert_eq!(press(&mut app, "y"), vec![]);
    let screen = render(&mut app, 140, 40);
    assert!(screen.contains("File finding under"), "{screen}");
    app.on_key(enter(), 60);
    let commands = app.take_commands();
    assert!(
        matches!(
            commands.as_slice(),
            [Command::Research(ResearchCommand::FileUnfiled {
                item: InboxId(40),
                ..
            })]
        ),
        "{commands:?}"
    );
}

#[test]
fn enter_on_an_unfiled_finding_also_picks_a_task_and_d_drops_it() {
    let mut snap = world();
    snap.inbox.clear();
    snap.inbox.push(unfiled_item(40));
    let mut app = app_with(snap);
    press(&mut app, " n");
    app.on_key(enter(), 1);
    assert!(render(&mut app, 140, 40).contains("File finding under"));
    app.on_key(esc(), 2);
    assert_eq!(
        press(&mut app, "d"),
        vec![Command::Inbox(InboxCommand::Dismiss { item: InboxId(40) })]
    );
}

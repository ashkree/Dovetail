//! `:round` (R1): opens and closes research rounds through the core.

mod support;

use dovetail_api::research::{
    FindingId, FindingSummary, ResearchContent, ResearchDocument, ResearchListing, ResearchSummary,
    ResearchWhat, RoundSummary, SourceId, SourceKind, SourceSummary,
};
use dovetail_api::*;
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
                n: 1,
                brief: "per seat or flat?".into(),
                dir: "rounds/01-2026-11-10".into(),
                opened_ms: NOW_MS - 86_400_000,
                closed_ms: Some(NOW_MS - 3_600_000),
                findings: 2,
            },
            RoundSummary {
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

fn deliver(app: &mut App, event: Event) {
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event { seq, event });
}

fn loaded(app: &mut App, what: ResearchWhat, path: Option<&str>, markdown: &str) {
    deliver(
        app,
        Event::ResearchLoaded {
            task: TaskId(1),
            what,
            content: ResearchContent::Document(ResearchDocument {
                path: path.map(Into::into),
                markdown: markdown.into(),
            }),
        },
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
        vec![Command::LoadResearch {
            task: TaskId(1),
            what: ResearchWhat::Listing,
        }]
    );
    deliver(
        &mut app,
        Event::ResearchLoaded {
            task: TaskId(1),
            what: ResearchWhat::Listing,
            content: ResearchContent::Listing(listing()),
        },
    );
    // The rows are known once drawn; the real loop draws after every message.
    render(&mut app, 120, 60);
    app
}

/// Presses `Enter` on the first research row (the report) and answers with the report.
fn open_report(app: &mut App) {
    app.on_key(enter(), 2);
    assert_eq!(
        app.take_commands(),
        vec![Command::LoadResearch {
            task: TaskId(1),
            what: ResearchWhat::Report,
        }]
    );
    loaded(
        app,
        ResearchWhat::Report,
        Some("/data/research/1-pricing/report.md"),
        REPORT,
    );
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
    app.on_key(enter(), 1);
    assert_eq!(
        app.take_commands(),
        vec![Command::LoadResearch {
            task: TaskId(1),
            what: ResearchWhat::Brief { round: 1 },
        }]
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
        for _ in 0..n {
            app.on_key(key('j'), 1);
        }
        app.on_key(enter(), 2);
        assert_eq!(
            app.take_commands(),
            vec![Command::LoadResearch {
                task: TaskId(1),
                what: what.clone(),
            }],
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
    let cite = dovetail_tui::Theme::dusk().cite;
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
        app.take_handoff(),
        Some(Command::OpenEditor {
            path: "/data/research/1-pricing/report.md".into()
        })
    );
    deliver(&mut app, Event::TerminalReturned { error: None });
    // A source that was listed but not cached has no file.
    app.on_key(esc(), 4);
    for _ in 0..7 {
        app.on_key(key('j'), 5);
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
        vec![Command::LoadResearch {
            task: TaskId(1),
            what: ResearchWhat::Listing,
        }]
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
        vec![Command::StartRound {
            task: TaskId(3),
            brief: "per seat or flat?".into(),
        }]
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
    });
    let mut app = app_with(snap);
    assert_eq!(
        run(&mut app, "round close"),
        vec![Command::CloseRound { task: TaskId(3) }]
    );
}

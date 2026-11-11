//! Reading mode (M21, ADR 0018, decided 2026-11-11), driven by `MockCore`'s seeded research
//! (task #3, a report citing findings and sources; a decision with evidence; two repo docs):
//! the second layout preset, the library, the reading column's citation keys, the core-launched
//! browser, Ask, and leaving.

mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::research::{DocImage, ResearchDocument, ResearchWhat};
use dovetail_api::*;
use dovetail_mock::{MockConfig, MockCore};
use dovetail_tui::App;
use dovetail_tui::canvas::view::CellPx;
use dovetail_tui::mode::Mode;
use dovetail_tui::region::RegionId;
use support::*;

fn start() -> (MockCore, App) {
    let core = MockCore::new(MockConfig::default());
    let app = app_with(core.state().clone());
    (core, app)
}

/// Hands the core the app's commands and the app the core's answers.
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

/// Runs the mock until `done`, handing over commands as they come.
fn run_until(core: &mut MockCore, app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..3_000 {
        if done(app) {
            return;
        }
        for command in app.take_commands() {
            core.handle(command);
        }
        if let Some(scheduled) = core.next_message() {
            app.on_server(scheduled.msg);
        }
    }
    panic!("never happened: {what}");
}

fn keys(app: &mut App, keys: &str) {
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 5 * i as u64);
    }
}

fn ctrl_o() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)
}

/// Reading mode on, with its lists delivered.
fn reading() -> (MockCore, App) {
    let (mut core, mut app) = start();
    keys(&mut app, " r");
    let commands = app.take_commands();
    assert!(commands.contains(&Command::ListDocs), "{commands:?}");
    assert!(
        commands.contains(&Command::LoadResearch {
            task: TaskId(3),
            what: ResearchWhat::Listing
        }),
        "{commands:?}"
    );
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    (core, app)
}

/// Moves the library cursor to the row containing `needle`.
fn pick(app: &mut App, needle: &str) {
    keys(app, "gg");
    for _ in 0..60 {
        if focused_row(app).contains(needle) {
            return;
        }
        keys(app, "j");
    }
    panic!(
        "no library row containing {needle:?}:\n{}",
        render(app, 160, 40)
    );
}

/// The library row with the selection highlight.
fn focused_row(app: &mut App) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let buffer = terminal.backend().buffer();
    let library = dovetail_tui::region::reading_layout(buffer.area, false).library;
    for y in library.y + 1..library.y + library.height {
        if buffer[(library.x + 2, y)].bg != ratatui::style::Color::Reset {
            return (library.x..library.x + library.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
        }
    }
    String::new()
}

/// The report of task #3, open in the reading column.
fn report() -> (MockCore, App) {
    let (mut core, mut app) = reading();
    pick(&mut app, "#3");
    app.on_key(enter(), 0);
    assert_eq!(
        app.take_commands(),
        [Command::LoadResearch {
            task: TaskId(3),
            what: ResearchWhat::Report
        }]
    );
    core.handle(Command::LoadResearch {
        task: TaskId(3),
        what: ResearchWhat::Report,
    });
    deliver(&mut core, &mut app);
    assert_eq!(app.focus(), RegionId::Reader);
    // The column is laid out when drawn, as it is before any key in the real loop.
    render(&mut app, 160, 40);
    (core, app)
}

#[test]
fn leader_r_toggles_reading_mode_and_layout_b_comes_back_as_it_was() {
    let (_, mut app) = start();
    keys(&mut app, "k");
    let before = render(&mut app, 160, 40);
    let focus = app.focus();
    keys(&mut app, " r");
    assert!(app.reading_mode());
    assert_eq!(app.focus(), RegionId::Library);
    let screen = render(&mut app, 160, 40);
    assert!(
        screen.contains("library") && screen.contains("reading"),
        "{screen}"
    );
    assert!(screen.contains(" READING "), "{screen}");
    keys(&mut app, " r");
    assert!(!app.reading_mode());
    assert_eq!(app.focus(), focus);
    assert_eq!(render(&mut app, 160, 40), before);
    // Esc in Normal mode leaves it too, from any of its columns.
    keys(&mut app, " r");
    keys(&mut app, "l");
    app.on_key(esc(), 0);
    assert!(!app.reading_mode());
    assert_eq!(render(&mut app, 160, 40), before);
}

#[test]
fn the_library_lists_projects_research_tangents_decisions_and_docs() {
    let (_, mut app) = reading();
    pick(&mut app, "Decisions");
    keys(&mut app, "o");
    pick(&mut app, "Docs");
    keys(&mut app, "o");
    pick(&mut app, "round 1");
    keys(&mut app, "o");
    pick(&mut app, "sources");
    keys(&mut app, "o");
    let screen = render(&mut app, 220, 60);
    for needle in [
        "Dovetail · open",
        "◇ Market sizing",
        "#3",
        "● report",
        "round 1",
        "brief",
        "F1 01 nucleo",
        "quick findings (1)",
        "S1 web https://docs.rs/nuc",
        "Decisions (3)",
        "Use nucleo for the picker",
        "Docs · ADRs (2)",
        "Reading mode: a second",
        // The Tangents section (ADR 0017, M22), from the mock's two seeded tangents.
        "↗ Tangents (2)",
        "How does kitty's graphics prot",
        "What hydration should a first",
    ] {
        assert!(screen.contains(needle), "{needle:?} missing:\n{screen}");
    }
    // Tangents, the built-in project, is pinned last among the projects with its own marker.
    let market = screen.find("◇ Market sizing").expect("research project");
    let tangents = screen.find("↗ Tangents ·").expect("Tangents project");
    let notes = screen.find("notes-site").expect("code project");
    assert!(notes < tangents && market < tangents, "{screen}");
}

#[test]
fn a_tangent_opens_its_answer_and_its_citations_resolve_against_its_sources() {
    let (mut core, mut app) = start();
    keys(&mut app, " r");
    let commands = app.take_commands();
    for tangent in [1, 2] {
        assert!(
            commands.contains(&Command::LoadTangent {
                tangent: TaskId(tangent),
                what: ResearchWhat::Listing,
            }),
            "{commands:?}"
        );
    }
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    pick(&mut app, "How does kitty");
    app.on_key(enter(), 0);
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::LoadTangent {
            tangent: TaskId(1),
            what: ResearchWhat::Finding(research::FindingId(1)),
        }]
    );
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    assert_eq!(app.focus(), RegionId::Reader);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("kitty sends images"), "{screen}");
    // `]s` selects [S1]; `gx` opens the web page; `gd` reads the cached copy. The tangent's
    // own S1, not the open project's S1 (docs.rs/nucleo).
    keys(&mut app, "]s");
    keys(&mut app, "gx");
    deliver(&mut core, &mut app);
    assert_eq!(
        core.opened(),
        ["https://sw.kovidgoyal.net/kitty/graphics-protocol/"]
    );
    keys(&mut app, "gd");
    let commands = app.take_commands();
    assert_eq!(
        commands,
        vec![Command::LoadTangent {
            tangent: TaskId(1),
            what: ResearchWhat::Source(research::SourceId(1)),
        }]
    );
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("Terminal graphics protocol"), "{screen}");
}

#[test]
fn i_in_the_library_does_nothing() {
    let (_, mut app) = reading();
    keys(&mut app, "i");
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.reading_mode());
    assert!(app.take_commands().is_empty());
}

#[test]
fn citations_step_with_s_keys_and_gd_opens_the_finding_and_ctrl_o_comes_back() {
    let (mut core, mut app) = report();
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("Which fuzzy matcher"), "{screen}");
    keys(&mut app, "]s");
    assert!(
        render(&mut app, 160, 40).contains("[F1] 1/"),
        "status shows the citation"
    );
    keys(&mut app, "gd");
    let commands = app.take_commands();
    assert_eq!(
        commands,
        [Command::LoadResearch {
            task: TaskId(3),
            what: ResearchWhat::Finding(dovetail_api::research::FindingId(1))
        }]
    );
    for command in commands {
        core.handle(command);
    }
    deliver(&mut core, &mut app);
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("reading · F1 · nucleo"), "{screen}");
    assert!(screen.contains("port of fzf"), "{screen}");
    app.on_key(ctrl_o(), 0);
    assert!(render(&mut app, 160, 40).contains("reading · #3 report"));
    // `[s` from the first goes nowhere before it; `]s` twice reaches [S1].
    keys(&mut app, "]s[s");
    assert!(render(&mut app, 160, 40).contains("[F1] 1/"));
    keys(&mut app, "]s");
    assert!(render(&mut app, 160, 40).contains("[S1] 2/"));
}

#[test]
fn gx_opens_a_web_source_through_the_core_and_explains_the_rest() {
    let (mut core, mut app) = report();
    keys(&mut app, "]s]s");
    keys(&mut app, "gx");
    let commands = app.take_commands();
    assert_eq!(
        commands,
        [Command::OpenExternal {
            url: "https://docs.rs/nucleo".into()
        }]
    );
    assert!(
        app.note()
            .is_some_and(|n| n.contains("opening https://docs.rs/nucleo"))
    );
    for command in commands {
        core.handle(command);
    }
    assert_eq!(core.opened(), ["https://docs.rs/nucleo"]);
    // A finding is read, not browsed.
    keys(&mut app, "gg]s");
    keys(&mut app, "gx");
    assert!(app.take_commands().is_empty());
    assert!(app.note().is_some_and(|n| n.contains("F1 is a finding")));
}

#[test]
fn an_html_source_reads_as_markdown_and_gx_opens_the_real_page() {
    let (mut core, mut app) = report();
    keys(&mut app, "]s]s");
    keys(&mut app, "gd");
    deliver(&mut core, &mut app);
    let screen = render(&mut app, 160, 40);
    assert!(
        screen.contains("reading · S1 · https://docs.rs/nucleo"),
        "{screen}"
    );
    assert!(screen.contains("Crate nucleo"), "{screen}");
    assert!(screen.contains("the threaded one"), "{screen}");
    for raw in ["<html", "<p>", "track()", "body{}"] {
        assert!(!screen.contains(raw), "{raw} shown:\n{screen}");
    }
    // The page cites nothing: gx opens the page itself.
    keys(&mut app, "gx");
    assert_eq!(
        app.take_commands(),
        [Command::OpenExternal {
            url: "https://docs.rs/nucleo".into()
        }]
    );
}

#[test]
fn ask_answers_with_citations_that_resolve_and_esc_twice_leaves() {
    let (mut core, mut app) = report();
    keys(&mut app, "i");
    assert_eq!(app.take_commands(), [Command::StartAsk { task: TaskId(3) }]);
    core.handle(Command::StartAsk { task: TaskId(3) });
    deliver(&mut core, &mut app);
    assert_eq!(app.mode(), Mode::Insert);
    assert_eq!(app.focus(), RegionId::Ask);
    type_keys(&mut app, "why nucleo?");
    // Esc while typing only returns to Normal mode, keeping the draft.
    app.on_key(esc(), 0);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.reading_mode(), "the first Esc only leaves Insert mode");
    keys(&mut app, "i");
    assert_eq!(app.mode(), Mode::Insert);
    app.on_key(enter(), 0);
    let ask = agent_id("ask-3");
    assert_eq!(
        app.take_commands(),
        [Command::Prompt {
            agent: ask.clone(),
            text: "why nucleo?".into(),
            attachments: vec![]
        }]
    );
    core.handle(Command::Prompt {
        agent: ask.clone(),
        text: "why nucleo?".into(),
        attachments: vec![],
    });
    run_until(&mut core, &mut app, "the answer", |app| {
        app.snapshot()
            .agents
            .iter()
            .any(|a| a.id == ask && a.status == AgentStatus::Idle)
            && app.snapshot().conversations.get(&ask).is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.text.contains("round 2 is checking"))
            })
    });
    // No finished-turn item for an Ask answer: it is in the column.
    assert!(!app.snapshot().inbox.iter().any(|i| i.agent == ask));
    let screen = render(&mut app, 220, 50);
    assert!(
        screen.contains("[F1]") && screen.contains("[S2]"),
        "{screen}"
    );
    // Sending closed the chat bar: Normal mode, still in Ask.
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.focus(), RegionId::Ask);
    keys(&mut app, "]s");
    assert!(
        app.note().is_some_and(|n| n.starts_with("[F1] 1/4")),
        "{:?}",
        app.note()
    );
    keys(&mut app, "]s");
    keys(&mut app, "gd");
    assert_eq!(
        app.take_commands(),
        [Command::LoadResearch {
            task: TaskId(3),
            what: ResearchWhat::Source(dovetail_api::research::SourceId(1))
        }]
    );
    app.on_key(esc(), 0);
    assert!(!app.reading_mode());
    // The Ask agent shows in neither the tree nor Needs you.
    assert!(!render(&mut app, 160, 40).contains("ask-3"));
    // Leaving keeps the session: back in, the answer is still there.
    keys(&mut app, " r");
    assert!(render(&mut app, 220, 50).contains("round 2 is checking"));
}

#[test]
fn a_decision_reads_with_its_evidence_and_gd_follows_it() {
    let (_, mut app) = reading();
    pick(&mut app, "Decisions");
    keys(&mut app, "o");
    pick(&mut app, "Use nucleo");
    app.on_key(enter(), 0);
    assert!(app.take_commands().is_empty(), "built from the snapshot");
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("reading · decision #3"), "{screen}");
    assert!(screen.contains("[F1] [S2]"), "{screen}");
    render(&mut app, 160, 40);
    keys(&mut app, "]sgd");
    assert_eq!(
        app.take_commands(),
        [Command::LoadResearch {
            task: TaskId(3),
            what: ResearchWhat::Finding(dovetail_api::research::FindingId(1))
        }]
    );
}

#[test]
fn a_doc_opens_read_only_and_its_image_is_text_without_kitty_graphics() {
    let (mut core, mut app) = reading();
    pick(&mut app, "Docs");
    keys(&mut app, "o");
    pick(&mut app, "Reading mode");
    app.on_key(enter(), 0);
    deliver(&mut core, &mut app);
    let screen = render(&mut app, 160, 40);
    assert!(
        screen.contains("reading · adr/0018-reading-mode-layout.md"),
        "{screen}"
    );
    assert!(
        screen.contains("▣ image: The three columns · reading-mode.png"),
        "{screen}"
    );
    assert!(app.reading_figures().is_empty());
}

#[test]
fn with_kitty_graphics_a_measured_image_is_placed_by_path() {
    let (_, mut app) = reading();
    app.set_graphics(Some(CellPx { w: 10, h: 20 }));
    pick(&mut app, "Docs");
    keys(&mut app, "o");
    pick(&mut app, "Reading mode");
    app.on_key(enter(), 0);
    app.take_commands();
    let path = std::path::PathBuf::from("adr/0018-reading-mode-layout.md");
    let seq = app.snapshot().seq + 1;
    app.on_server(ServerMsg::Event {
        seq,
        event: Event::DocLoaded {
            path,
            document: ResearchDocument {
                path: None,
                markdown: "# Figure\n\n![The three columns](cols.png)\n\nAfter.\n".into(),
                images: vec![DocImage {
                    src: "cols.png".into(),
                    path: "/repo/docs/adr/cols.png".into(),
                    width: 400,
                    height: 200,
                }],
            },
        },
    });
    let screen = render(&mut app, 220, 50);
    assert!(screen.contains("▣ The three columns"), "{screen}");
    let figures = app.reading_figures();
    assert_eq!(figures.len(), 1, "{figures:?}");
    // 400 px over 10 px cells is 40 columns; 40 * 10 * 200 / 400 / 20 is 10 rows.
    assert_eq!((figures[0].cols, figures[0].rows), (40, 10));
    assert_eq!(
        figures[0].path,
        std::path::Path::new("/repo/docs/adr/cols.png")
    );
    // Not while a picker floats over it, nor out of reading mode.
    app.on_key(esc(), 0);
    assert!(app.reading_figures().is_empty());
}

#[test]
fn reading_mode_draws_only_when_something_changed() {
    let (_, mut app) = report();
    render(&mut app, 160, 40);
    app.take_dirty();
    assert!(!app.is_dirty());
    assert_eq!(app.next_deadline_ms(), None, "no timer while idle");
    keys(&mut app, "j");
    assert!(app.is_dirty());
}

fn snapshot_at(width: u16, height: u16) -> String {
    let (_, mut app) = report();
    keys(&mut app, "]s");
    render(&mut app, width, height)
}

#[test]
fn snapshot_reading_a_report_at_100() {
    insta::assert_snapshot!(snapshot_at(100, 30));
}

#[test]
fn snapshot_reading_a_report_at_160() {
    insta::assert_snapshot!(snapshot_at(160, 40));
}

#[test]
fn snapshot_reading_a_report_at_220() {
    insta::assert_snapshot!(snapshot_at(220, 50));
}

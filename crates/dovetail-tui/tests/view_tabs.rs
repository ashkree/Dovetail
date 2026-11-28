//! Task #20: the view-tab row under the header.

mod support;

use dovetail_api::research::ResearchSummary;
use dovetail_api::tracking::task::TaskId;
use dovetail_tui::App;
use dovetail_tui::layout::tabs::{TABS, key_label};
use support::*;

fn leader(app: &mut App, keys: &str) {
    app.on_key(key(' '), 0);
    for (i, c) in keys.chars().enumerate() {
        app.on_key(key(c), 1 + i as u64);
    }
}

fn world_with_research() -> dovetail_api::Snapshot {
    let mut snap = world();
    snap.research.push(ResearchSummary {
        task: TaskId(1),
        dir: "/data/research/1-pricing".into(),
        rounds: vec![],
        findings: 0,
        quick_findings: 0,
        sources: 0,
        report_ms: None,
        kind: None,
    });
    snap
}

/// The tabs inside the screens box (screen row 1) of an app after `setup`, borders removed.
fn tab_row(width: u16, setup: impl FnOnce(&mut App)) -> String {
    let mut app = app_with(world_with_research());
    setup(&mut app);
    render(&mut app, width, 30)
        .lines()
        .nth(1)
        .unwrap_or_default()
        .trim_matches('│')
        .trim_end()
        .to_string()
}

const FULL: &str = " ␣c session · ␣b board · ␣g worktrees · ␣M maps · ␣R research · ␣r reading";

#[test]
fn tab_row_at_120_columns() {
    assert_eq!(tab_row(120, |_| {}), FULL);
}

#[test]
fn tab_row_at_80_columns_still_fits_whole() {
    assert_eq!(tab_row(80, |_| {}), FULL);
}

#[test]
fn tab_row_below_80_columns_shows_keys_only_and_never_overflows() {
    assert_eq!(tab_row(60, |_| {}), " ␣c ␣b ␣g ␣M ␣R ␣r");
    let narrow = tab_row(10, |_| {});
    assert!(narrow.chars().count() <= 10, "{narrow:?}");
}

#[test]
fn the_tab_keys_come_from_the_keymap() {
    let keys: Vec<_> = TABS.iter().map(|t| key_label(t.action).unwrap()).collect();
    assert_eq!(keys, ["␣c", "␣b", "␣g", "␣M", "␣R", "␣r"]);
}

/// The label of the tab drawn bold and underlined, per view.
fn active_tab(width: u16, setup: impl FnOnce(&mut App)) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
    let mut app = app_with(world_with_research());
    setup(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let marked: String = (0..width)
        .map(|x| {
            let cell = &buffer[(x, 1)];
            if cell.modifier.contains(Modifier::UNDERLINED) {
                cell.symbol().to_string()
            } else {
                "\0".to_string()
            }
        })
        .collect();
    marked
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_active_tab_follows_the_view_at_80_and_120() {
    for width in [80, 120] {
        assert_eq!(active_tab(width, |_| {}), ["␣c session"], "{width}");
        assert_eq!(
            active_tab(width, |a| leader(a, "b")),
            ["␣b board"],
            "{width}"
        );
        assert_eq!(
            active_tab(width, |a| leader(a, "g")),
            ["␣g worktrees"],
            "{width}"
        );
        assert_eq!(
            active_tab(width, |a| leader(a, "M")),
            ["␣M maps"],
            "{width}"
        );
        assert_eq!(
            active_tab(width, |a| leader(a, "R")),
            ["␣R research"],
            "{width}"
        );
    }
}

#[test]
fn the_reading_tab_is_active_in_reading_mode() {
    assert_eq!(active_tab(120, |a| leader(a, "r")), ["␣r reading"]);
}

#[test]
fn leader_r_capital_opens_the_research_list() {
    let mut app = app_with(world_with_research());
    leader(&mut app, "R");
    assert!(render(&mut app, 120, 30).contains("Research"));
}

#[test]
fn the_tab_row_is_hidden_in_focus_mode() {
    // MAP mode hides it the same way (`layout(.., with_tabs = false)`, tested in region.rs);
    // entering MAP mode needs a drawn map, which map_canvas.rs covers.
    let focused = tab_row(120, |a| leader(a, "z"));
    assert!(!focused.contains("session"), "{focused:?}");
}

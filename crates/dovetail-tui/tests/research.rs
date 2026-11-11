//! `:round` (R1): opens and closes research rounds through the core.

mod support;

use dovetail_api::research::{ResearchSummary, RoundSummary};
use dovetail_api::*;
use support::*;

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

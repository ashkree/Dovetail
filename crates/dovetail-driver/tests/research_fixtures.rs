//! The two spike S7 recordings replay through `FakeDriver` and carry what `docs/research.md`
//! §5 captures: the subagent's source tool calls (tagged with the `Agent` call's id), their
//! results, and the full report (`docs/spikes/research-capture.md`).

use dovetail_driver::{
    ContentBlock, DriverEvent, FakeDriver, SessionDriver, SessionSpec, TaskPhase,
};
use std::path::Path;

async fn replay(name: &str) -> Vec<DriverEvent> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.ndjson"));
    let fake = FakeDriver::from_file(path).expect("fixture loads");
    let mut session = fake
        .start(SessionSpec::new("s-1", "/tmp"))
        .expect("session starts");
    let mut rx = session.take_events();
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        let done = matches!(ev, DriverEvent::Exited(_));
        out.push(ev);
        if done {
            break;
        }
    }
    out
}

fn check(events: &[DriverEvent]) {
    let agent_call = events
        .iter()
        .find_map(|e| match e {
            DriverEvent::Assistant(m) => m.blocks.iter().find_map(|b| match b {
                ContentBlock::ToolUse { id, name, .. } if name == "Agent" => Some(id.clone()),
                _ => None,
            }),
            _ => None,
        })
        .expect("coordinator calls Agent");
    let mut names = Vec::new();
    for e in events {
        if let DriverEvent::Assistant(m) = e {
            for b in &m.blocks {
                if let ContentBlock::ToolUse { name, .. } = b
                    && m.origin.parent_tool_use_id.as_deref() == Some(agent_call.as_str())
                {
                    names.push(name.as_str());
                }
            }
        }
    }
    for want in ["WebFetch", "Read", "SubagentHandback"] {
        assert!(
            names.contains(&want),
            "{want} tagged to {agent_call} in {names:?}"
        );
    }
    assert!(events.iter().any(|e| matches!(
        e,
        DriverEvent::Task(t) if t.phase == TaskPhase::Notification && t.output_file.is_some()
    )));
}

#[tokio::test]
async fn foreground_round_replays() {
    check(&replay("research_round_foreground").await);
}

#[tokio::test]
async fn background_round_replays() {
    check(&replay("research_round_background").await);
}

//! Long-session memory (M16): streams a long synthetic conversation through the UI, drawing as
//! it goes, and prints the process's resident memory. Run by `scripts/rss-check.sh` in release
//! mode; ignored in the normal test run because the number only means something there.

mod support;

use dovetail_api::{
    Event, ServerMsg,
    activity::{ActivityEvent, ActivityItem, ActivityKind},
    agent::AgentStatus,
    conversation::{ConversationEvent, EntryId, EntryKind},
    snapshot::{ACTIVITY_TAIL, CONVERSATION_KEEP},
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use support::*;

/// Resident set size and its peak, in kB, from `/proc/self/status`.
fn rss_kb() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .unwrap_or(0)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// Turns streamed unless `DOVETAIL_RSS_TURNS` says otherwise.
const TURNS: u64 = 10_000;
const DELTAS: u64 = 20;
const DELTA: &str = "The slice rebased **cleanly** onto `main`; see `src/lib.rs` for the change. ";

#[test]
#[ignore = "measurement: run scripts/rss-check.sh"]
fn a_long_streaming_conversation_stays_bounded() {
    let mut snap = world();
    snap.agents[0].status = AgentStatus::Working;
    let turns = std::env::var("DOVETAIL_RSS_TURNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(TURNS);
    let mut app = app_with(snap);
    app.on_key(key('h'), 0);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
    terminal.draw(|f| app.draw(f)).expect("draw");
    let (start, _) = rss_kb();

    let mut seq = app.snapshot().seq;
    let mut send = |app: &mut dovetail_tui::App, event: Event| {
        seq += 1;
        app.on_server(ServerMsg::Event { seq, event });
    };
    let coordinator = agent_id("coordinator");
    let mut id = 1_000;
    for turn in 0..turns {
        let at = NOW_MS + turn as i64;
        for (kind, text) in [
            (EntryKind::User, "Please keep going with the next slice."),
            (EntryKind::ToolSummary, "read 3 files · 2 s"),
            (EntryKind::Assistant, ""),
        ] {
            id += 1;
            send(
                &mut app,
                Event::Conversation(ConversationEvent::Appended {
                    agent: coordinator.clone(),
                    entry: entry(id, at, kind, text),
                }),
            );
        }
        for _ in 0..DELTAS {
            send(
                &mut app,
                Event::Conversation(ConversationEvent::AssistantDelta {
                    agent: coordinator.clone(),
                    entry: EntryId(id),
                    text: DELTA.into(),
                }),
            );
        }
        // The real loop coalesces streaming to about 30 fps: a few frames per reply.
        if app.take_dirty() {
            terminal.draw(|f| app.draw(f)).expect("draw");
        }
        send(
            &mut app,
            Event::Activity(ActivityEvent::Added(ActivityItem {
                id: turn,
                at_ms: at,
                agent: Some(coordinator.clone()),
                kind: ActivityKind::Lifecycle,
                text: "turn finished".into(),
            })),
        );
    }
    terminal.draw(|f| app.draw(f)).expect("draw");

    let (end, peak) = rss_kb();
    let held = app.snapshot().conversations[&coordinator].entries.len();
    println!(
        "turns={turns} entries_held={held} rss_start_kb={start} rss_end_kb={end} peak_kb={peak}"
    );
    assert!(held <= CONVERSATION_KEEP);
    assert!(app.snapshot().activity.len() <= ACTIVITY_TAIL);
}

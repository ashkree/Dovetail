//! Recorded agy (ACP) sessions from the `agy-headless` spike, replayed through the agy
//! driver's own mapping: what the core sees from a real agy session.

use dovetail_driver::{
    ContentBlock, ControlAnswer, DriverEvent, DriverKind, FakeDriver, ReplayOptions, Sent,
    SessionDriver, SessionSpec,
};

fn fixture(name: &str) -> FakeDriver {
    let path = format!(
        "{}/tests/fixtures/agy/{name}.ndjson",
        env!("CARGO_MANIFEST_DIR")
    );
    FakeDriver::from_file(path)
        .expect("fixture loads")
        .for_agy()
}

async fn all_events(fake: &FakeDriver, spec: SessionSpec) -> Vec<DriverEvent> {
    let mut session = fake.start(spec).expect("session starts");
    let mut events = session.take_events();
    let mut out = Vec::new();
    while let Some(ev) = events.recv().await {
        let exited = matches!(ev, DriverEvent::Exited(_));
        out.push(ev);
        if exited {
            break;
        }
    }
    out
}

#[tokio::test]
async fn a_plain_turn_streams_confirms_and_ends() {
    let fake = fixture("plain_turn");
    assert_eq!(fake.kind(), DriverKind::Agy);
    let events = all_events(&fake, SessionSpec::new("d-1", "/work")).await;

    let Some(DriverEvent::Init(init)) = events.first() else {
        panic!("{events:?}")
    };
    assert_eq!(init.session_id, "bfbc337e-0bc1-4d82-b5e8-1db97b63f1ed");
    assert_eq!(
        init.cwd.as_deref(),
        Some("/work"),
        "the session's own folder"
    );
    assert!(events.iter().any(
        |e| matches!(e, DriverEvent::Models(m) if m.iter().any(|m| m.value == "claude-opus-5-5"))
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DriverEvent::TextDelta { text, .. } if text == "pong"))
    );
    let confirmed = events.iter().find_map(|e| match e {
        DriverEvent::Assistant(m) if !m.text().is_empty() => Some(m),
        _ => None,
    });
    assert_eq!(confirmed.map(|m| m.text()), Some("pong".to_owned()));
    let result = events.iter().find_map(|e| match e {
        DriverEvent::TurnResult(r) => Some(r),
        _ => None,
    });
    let result = result.expect("a result");
    assert_eq!(result.subtype, "success");
    assert_eq!(result.context_window(), Some(1_048_576));
    assert!(matches!(events.last(), Some(DriverEvent::Exited(_))));
}

#[tokio::test]
async fn a_permission_request_waits_for_dovetail_and_the_tool_runs_after() {
    let fake = fixture("permission").with_options(ReplayOptions {
        wait_for_answers: true,
        ..Default::default()
    });
    let mut session = fake
        .start(SessionSpec::new("d-1", "/w"))
        .expect("session starts");
    let mut events = session.take_events();
    let request = loop {
        match events.recv().await.expect("a request comes") {
            DriverEvent::PermissionRequest(req) => break req,
            DriverEvent::ToolResult(r) => panic!("ran before the answer: {r:?}"),
            _ => {}
        }
    };
    assert_eq!(request.tool_name, "run_command");
    assert_eq!(
        request.input["CommandLine"], "touch b.txt && rm b.txt",
        "{:?}",
        request.input
    );
    session
        .answer_control(request.id.clone(), ControlAnswer::allow())
        .expect("answer");
    let mut ran = false;
    while let Some(ev) = events.recv().await {
        ran |= matches!(ev, DriverEvent::ToolResult(ref r) if !r.is_error);
        if matches!(ev, DriverEvent::Exited(_)) {
            break;
        }
    }
    assert!(ran, "the tool result follows the answer");
    assert!(matches!(&fake.sent()[..], [Sent::Answer { id, .. }] if *id == request.id));
}

#[tokio::test]
async fn a_cancelled_turn_reads_as_interrupted_and_the_next_one_runs() {
    let events = all_events(&fixture("cancel"), SessionSpec::new("d-1", "/w")).await;
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DriverEvent::TurnResult(r) => Some((r.subtype.as_str(), r.result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0].0, "error_during_execution");
    assert_eq!(results[1], ("success", Some("after".to_owned())));
}

#[tokio::test]
async fn dovetail_tools_are_tool_calls_and_allowed_ones_are_not_asked_about() {
    let mut spec = SessionSpec::new("d-1", "/w");
    spec.agy.allowed_tools = vec!["mcp_dovetail_core_report_status".into()];
    let events = all_events(&fixture("mcp_tool"), spec).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DriverEvent::PermissionRequest(_))),
        "the allow list answers for Dovetail's own tools"
    );
    let call = events.iter().find_map(|e| match e {
        DriverEvent::Assistant(m) => m.blocks.iter().find_map(|b| match b {
            ContentBlock::ToolUse { name, input, .. } => Some((name.clone(), input.clone())),
            _ => None,
        }),
        _ => None,
    });
    let (name, input) = call.expect("a tool call");
    assert_eq!(name, "mcp_dovetail_core_report_status");
    assert_eq!(input["status"], "agy probe");
    // The bridge could not reach Dovetail in the recording, so the call failed.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DriverEvent::ToolResult(r) if r.is_error))
    );
}

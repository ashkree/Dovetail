use super::*;

fn plan(mode: PermissionMode) -> Plan {
    let mut spec = SessionSpec::new("dovetail-id", "/w");
    spec.model = Some("gemini-3.8-flash".into());
    spec.effort = Some(ReasoningEffort::Low);
    spec.permission_mode = mode;
    spec.agy.allowed_tools = vec!["mcp_dovetail_core_report_status".into()];
    Plan::of(&spec, None)
}

fn answer(line: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": line["id"], "result": result})
}

fn opened() -> Value {
    json!({
        "sessionId": "agy-1",
        "models": {
            "currentModelId": "gemini-4-neon-high",
            "availableModels": [
                {"modelId": "gemini-4-neon-high", "name": "Gemini 4 Neon (High)"},
                {"modelId": "gemini-3.8-flash-low", "name": "Gemini 3.8 Flash (Low)"},
                {"modelId": "gemini-3.8-flash-medium", "name": "Gemini 3.8 Flash (Medium)"},
            ],
        },
        "modes": {"currentModeId": "default"},
    })
}

/// Runs start-up to the point where prompts go out; returns the conversation and the events.
fn started(mode: PermissionMode) -> (Conversation, Vec<DriverEvent>) {
    let mut c = Conversation::new(plan(mode));
    let init = c.start();
    assert_eq!(init["method"], "initialize");
    let step = c.received(&answer(&init, json!({"agentInfo": {"version": "1.5.14"}})));
    let [open] = &step.lines[..] else {
        panic!("{step:?}")
    };
    assert_eq!(open["method"], "session/new");
    let step = c.received(&answer(open, opened()));
    let mut events = step.events;
    // The spec's model, then its mode.
    let [model, mode_line] = &step.lines[..] else {
        panic!("{:?}", step.lines)
    };
    assert_eq!(model["method"], "session/set_model");
    assert_eq!(model["params"]["modelId"], "gemini-3.8-flash-low");
    assert_eq!(mode_line["params"]["modeId"], "auto");
    assert!(c.received(&answer(model, json!({}))).events.is_empty());
    events.extend(c.received(&answer(mode_line, json!({}))).events);
    (c, events)
}

#[test]
fn start_up_opens_the_conversation_and_applies_the_spec() {
    let (_, events) = started(PermissionMode::Auto);
    let [
        DriverEvent::Init(init),
        DriverEvent::Models(models),
        DriverEvent::ModelSettings {
            switch,
            model,
            effort,
        },
    ] = &events[..]
    else {
        panic!("{events:?}")
    };
    assert_eq!(
        init.session_id, "agy-1",
        "agy picks the id; the core records it"
    );
    assert_eq!(init.version.as_deref(), Some("1.5.14"));
    assert_eq!(init.permission_mode.as_deref(), Some("default"));
    assert_eq!(models.len(), 2);
    assert_eq!(*switch, None);
    assert_eq!(model.as_deref(), Some("gemini-3.8-flash"));
    assert_eq!(*effort, Some(ReasoningEffort::Low));
}

#[test]
fn prompts_wait_for_start_up_and_go_one_at_a_time() {
    let mut c = Conversation::new(plan(PermissionMode::Manual));
    let init = c.start();
    assert!(c.send(&UserInput::text("first")).lines.is_empty());
    let open = c.received(&answer(&init, json!({}))).lines.remove(0);
    let mut result = opened();
    result["modes"]["currentModeId"] = json!("default");
    let step = c.received(&answer(&open, result));
    let model = &step.lines[0];
    let step = c.received(&answer(model, json!({})));
    let [prompt] = &step.lines[..] else {
        panic!("{step:?}")
    };
    assert_eq!(prompt["method"], "session/prompt");
    assert_eq!(
        prompt["params"],
        json!({"sessionId": "agy-1", "prompt": [{"type": "text", "text": "first"}]})
    );
    assert!(
        c.send(&UserInput::text("second")).lines.is_empty(),
        "a turn runs"
    );
    let cancel = c.interrupt();
    assert_eq!(cancel.lines[0]["method"], "session/cancel");
    let step = c.received(&answer(prompt, json!({"stopReason": "cancelled"})));
    assert!(matches!(
        step.events.last(),
        Some(DriverEvent::TurnResult(r)) if r.subtype == "error_during_execution"
    ));
    assert_eq!(step.lines[0]["params"]["prompt"][0]["text"], "second");
}

#[test]
fn a_model_switch_is_confirmed_by_its_answer_or_refused() {
    let (mut c, _) = started(PermissionMode::Auto);
    let (switch, step) = c.set_model(None, Some(ReasoningEffort::Medium));
    assert_eq!(
        step.lines[0]["params"]["modelId"],
        "gemini-3.8-flash-medium"
    );
    let events = c.received(&answer(&step.lines[0], json!({}))).events;
    assert_eq!(
        events,
        [DriverEvent::ModelSettings {
            switch: Some(switch),
            model: Some("gemini-3.8-flash".into()),
            effort: Some(ReasoningEffort::Medium),
        }]
    );
    let (switch, step) = c.set_model(Some("gemini-3.8-flash"), Some(ReasoningEffort::Max));
    assert!(
        step.lines.is_empty(),
        "nothing is sent for a choice agy lacks"
    );
    assert!(
        matches!(&step.events[..], [DriverEvent::ModelSwitchFailed { switch: s, .. }] if *s == switch)
    );
    let (switch, step) = c.set_model(Some("gemini-4-neon"), None);
    let refused = json!({"jsonrpc": "2.0", "id": step.lines[0]["id"], "error": {"code": -32602, "message": "no"}});
    assert_eq!(
        c.received(&refused).events,
        [DriverEvent::ModelSwitchFailed {
            switch,
            error: "no".into()
        }]
    );
}

fn permission(id: &str, tool: &str) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "session/request_permission",
        "params": {"toolCall": {"toolCallId": "c", "name": tool, "rawInput": {}},
                   "options": [{"optionId": "allow", "kind": "allow_once"},
                               {"optionId": "deny", "kind": "reject_once"}]},
    })
}

#[test]
fn permissions_are_held_answered_or_settled_by_the_allow_list() {
    let (mut c, _) = started(PermissionMode::Auto);
    let step = c.received(&permission("1", "run_command"));
    let [DriverEvent::PermissionRequest(req)] = &step.events[..] else {
        panic!("{step:?}")
    };
    let step = c.received(&permission("2", "mcp_dovetail_core_report_status"));
    assert!(
        step.events.is_empty(),
        "Dovetail's own tools need no answer"
    );
    assert_eq!(step.lines[0]["result"]["outcome"]["optionId"], "allow");
    let step = c
        .answer(&req.id, &ControlAnswer::deny("no"))
        .expect("pending");
    assert_eq!(step.lines[0]["id"], "1");
    assert_eq!(step.lines[0]["result"]["outcome"]["optionId"], "deny");
    assert!(matches!(
        c.answer(&req.id, &ControlAnswer::allow()),
        Err(DriverError::UnknownControl(_))
    ));
}

#[test]
fn a_dont_ask_session_denies_without_asking() {
    let mut c = Conversation::new(plan(PermissionMode::DontAsk));
    let step = c.received(&permission("1", "run_command"));
    assert!(step.events.is_empty());
    assert_eq!(step.lines[0]["result"]["outcome"]["optionId"], "deny");
}

#[test]
fn a_refused_open_ends_as_a_failed_turn_and_closes() {
    let mut c = Conversation::new(plan(PermissionMode::Auto));
    let init = c.start();
    let open = c.received(&answer(&init, json!({}))).lines.remove(0);
    let refused = json!({"jsonrpc": "2.0", "id": open["id"], "error": {"code": -32603, "message": "not logged in"}});
    let step = c.received(&refused);
    assert!(step.close);
    assert!(matches!(
        step.events.last(),
        Some(DriverEvent::TurnResult(r)) if r.is_error && r.result.as_deref() == Some("not logged in")
    ));
}

#[test]
fn agent_requests_we_do_not_serve_are_refused_and_unknown_lines_kept() {
    let mut c = Conversation::new(plan(PermissionMode::Auto));
    let step = c
        .received(&json!({"jsonrpc": "2.0", "id": 9, "method": "fs/read_text_file", "params": {}}));
    assert_eq!(step.lines[0]["error"]["code"], -32601);
    let step = c.received(&json!({"jsonrpc": "2.0", "id": 99, "result": {}}));
    assert!(matches!(&step.events[..], [DriverEvent::Unknown(_)]));
}

#[test]
fn images_go_before_the_text() {
    let input = UserInput {
        text: "look".into(),
        images: vec![crate::ImageInput {
            media_type: "image/png".into(),
            data: vec![1, 2, 3],
        }],
    };
    assert_eq!(
        prompt_blocks(&input),
        json!([{"type": "image", "mimeType": "image/png", "data": "AQID"},
               {"type": "text", "text": "look"}])
    );
}

//! Tool and socket wire tests.

use dovetail_api::effort::CriterionId;
use dovetail_api::inbox::InboxKind;
use dovetail_api::model::ReasoningEffort;
use dovetail_api::task::{PLevel, TaskStatus};
use dovetail_tools::coordination::{ClaimCriterion, CoordinationTool, Report, ReportState};
use dovetail_tools::project::{ProjectTool, StartSlice};
use dovetail_tools::report::{AskUser, ReportTool};
use dovetail_tools::socket::{SocketRequest, SocketRequestBody};
use dovetail_tools::task::{BucketFilter, TaskTool};
use dovetail_tools::{Role, TOOLS, Tool, ToolError, deprecated_alias, deprecation_note, tool_spec};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let text = serde_json::to_string(value).expect("serialises");
    let back: T = serde_json::from_str(&text).expect("deserialises");
    assert_eq!(&back, value, "changed in transit: {text}");
}

/// ADR 0016: each renamed tool still parses by its old name, as the new tool, and `SocketRequest`
/// deserializes the nested `Tool` shape.
#[test]
fn old_tool_names_parse_as_the_new_tools() {
    let renamed: Vec<_> = TOOLS.iter().filter(|t| t.old_name.is_some()).collect();
    assert_eq!(renamed.len(), 12);
    for spec in renamed {
        let old = spec.old_name.unwrap_or_default();
        assert!(
            spec.name.starts_with(spec.suite.as_str()),
            "{} lacks its prefix",
            spec.name
        );
        assert_eq!(tool_spec(old), Some(spec));
        assert_eq!(deprecated_alias(old), Some(spec));
        assert_eq!(deprecated_alias(spec.name), None);
        assert!(deprecation_note(spec).contains(&format!("call `{}` instead", spec.name)));
    }
    let new = Tool::parse("tasks_list", json!({})).expect("new name");
    assert_eq!(Tool::parse("list_tasks", json!({})), Ok(new.clone()));
    // Nested `Tool` on the socket wire (`{"task": {"list": {}}}`), no `called_as`.
    let line = json!({"id": 1, "body": {"kind": "tool_call", "agent": "a", "tool_use_id": null,
        "call": {"task": {"list": {}}}}});
    let request: SocketRequest = serde_json::from_value(line).expect("nested wire call parses");
    assert!(matches!(
        request.body,
        SocketRequestBody::ToolCall { call, called_as: None, .. } if call == new
    ));
}

#[test]
fn an_old_blocking_ask_user_and_inbox_payload_still_parse() {
    // `core_ask_user` never blocks (ADR 0007): an old caller's `blocking` is ignored, not refused.
    let tool = Tool::parse(
        "core_ask_user",
        json!({"question": "Which port?", "blocking": true}),
    );
    assert_eq!(
        tool,
        Ok(Tool::Report(ReportTool::AskUser(AskUser {
            question: "Which port?".into()
        })))
    );
    // Inbox rows stored before the change carry the field too.
    let kind: InboxKind =
        serde_json::from_value(json!({"question": {"text": "Which port?", "blocking": true}}))
            .expect("old payload parses");
    assert_eq!(
        kind,
        InboxKind::Question {
            text: "Which port?".into()
        }
    );
}

#[test]
fn update_task_tells_an_absent_priority_from_a_null_one_and_survives_the_socket() {
    let parse = |args| match Tool::parse("tasks_update", args) {
        Ok(Tool::Task(TaskTool::Update(u))) => u,
        other => panic!("{other:?}"),
    };
    let absent = parse(json!({"id": 1, "status": "done"}));
    assert_eq!(absent.p_level, None);
    let cleared = parse(json!({"id": 1, "p_level": null}));
    assert_eq!(cleared.p_level, Some(None));
    let set = parse(json!({"id": 1, "p_level": "P1", "bucket": "unsorted"}));
    assert_eq!(set.p_level, Some(Some(PLevel::P1)));
    assert_eq!(set.bucket, Some(BucketFilter::Unsorted));
    assert_eq!(set.new_bucket(), Some(None));
    for u in [absent, cleared, set] {
        roundtrip(&Tool::Task(TaskTool::Update(u)));
    }

    let refused = |args| {
        Tool::parse("tasks_update", args)
            .expect("parses")
            .validate(Role::Coordinator)
    };
    assert_eq!(refused(json!({"id": 1, "bucket": "now"})), Ok(()));
    assert_eq!(
        refused(json!({"id": 1, "status": "archived"})),
        Err(ToolError::UpdateToArchived)
    );
    assert_eq!(refused(json!({"id": 1})), Err(ToolError::NothingToUpdate));
    assert_eq!(
        refused(json!({"id": 1, "title": " "})),
        Err(ToolError::Empty("title"))
    );
}

/// ADR 0022: a slice's report parses from MCP arguments, survives the socket, and is refused
/// from any role but a slice.
#[test]
fn a_report_parses_roundtrips_and_is_for_slices_only() {
    let parsed = Tool::parse(
        "coordination_report",
        json!({"state": "done", "summary": "PR open, tests pass", "pr": 41}),
    )
    .unwrap();
    let report = Tool::Coordination(CoordinationTool::Report(Report {
        state: ReportState::Done,
        summary: "PR open, tests pass".into(),
        pr: Some(41),
    }));
    assert_eq!(parsed, report);
    roundtrip(&report);
    assert_eq!(report.validate(Role::Slice), Ok(()));
    assert_eq!(
        report.validate(Role::Coordinator),
        Err(ToolError::ReportFromNonSlice)
    );
    let no_pr = Tool::parse(
        "coordination_report",
        json!({"state": "blocked", "summary": "need the schema"}),
    )
    .unwrap();
    assert_eq!(no_pr.validate(Role::Slice), Ok(()));
    let empty = Tool::parse(
        "coordination_report",
        json!({"state": "progress", "summary": "  "}),
    )
    .unwrap();
    assert_eq!(
        empty.validate(Role::Slice),
        Err(ToolError::Empty("summary"))
    );
}

/// Decided 2026-11-16: `coordination_claim_criterion` parses, survives the socket, and is for
/// slices and the coordinator only, with evidence.
#[test]
fn a_criterion_claim_is_for_slices_and_the_coordinator_with_evidence() {
    let parsed = Tool::parse(
        "coordination_claim_criterion",
        json!({"criterion": 3, "evidence": "PR #41, test guest_pays"}),
    )
    .unwrap();
    let claim = Tool::Coordination(CoordinationTool::ClaimCriterion(ClaimCriterion {
        criterion: CriterionId(3),
        evidence: "PR #41, test guest_pays".into(),
    }));
    assert_eq!(parsed, claim);
    roundtrip(&claim);
    assert_eq!(claim.validate(Role::Slice), Ok(()));
    assert_eq!(claim.validate(Role::Coordinator), Ok(()));
    for role in [Role::SideChat, Role::Ask] {
        assert_eq!(claim.validate(role), Err(ToolError::ClaimFromOtherRole));
    }
    let empty = Tool::parse(
        "coordination_claim_criterion",
        json!({"criterion": 3, "evidence": " "}),
    )
    .unwrap();
    assert_eq!(
        empty.validate(Role::Slice),
        Err(ToolError::Empty("evidence"))
    );
    let zero = Tool::parse(
        "coordination_claim_criterion",
        json!({"criterion": 0, "evidence": "x"}),
    )
    .unwrap();
    assert_eq!(zero.validate(Role::Slice), Err(ToolError::NotACriterionId));
}

/// ADR 0024: `review` is a status with its own name on the wire, and an agent's `tasks_update`
/// cannot set any status.
#[test]
fn review_is_a_status_and_agents_cannot_set_status() {
    assert_eq!(
        serde_json::to_value(TaskStatus::Review).unwrap(),
        json!("review")
    );
    roundtrip(&TaskStatus::Review);
    for (status, refusal) in [
        ("done", ToolError::StatusIsSetByDovetail),
        ("review", ToolError::StatusIsSetByDovetail),
        ("archived", ToolError::UpdateToArchived),
    ] {
        let call = Tool::parse("tasks_update", json!({"id": 1, "status": status})).unwrap();
        assert_eq!(call.validate(Role::Coordinator), Err(refusal), "{status}");
    }
    let other = Tool::parse("tasks_update", json!({"id": 1, "title": "renamed"})).unwrap();
    assert_eq!(other.validate(Role::Coordinator), Ok(()));
}

#[test]
fn start_slice_takes_an_optional_model_and_an_optional_effort() {
    let base = json!({"task": 3, "name": "x", "brief": "go"});
    // Left out: nothing changes for old callers.
    let plain = Tool::parse("project_start_slice", base.clone()).expect("parses");
    assert!(matches!(
        &plain,
        Tool::Project(ProjectTool::StartSlice(StartSlice {
            model: None,
            effort: None,
            ..
        }))
    ));
    assert_eq!(plain.validate(Role::Coordinator), Ok(()));
    // Given: an alias and a level.
    let mut args = base.clone();
    args["model"] = json!("haiku");
    args["effort"] = json!("low");
    let chosen = Tool::parse("project_start_slice", args).expect("parses");
    match &chosen {
        Tool::Project(ProjectTool::StartSlice(s)) => {
            assert_eq!(s.model.as_deref(), Some("haiku"));
            assert_eq!(s.effort, Some(ReasoningEffort::Low));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(chosen.validate(Role::Coordinator), Ok(()));
    // An unknown level is refused when parsed; an effort alone is fine (the model defaults).
    let mut bad = base.clone();
    bad["model"] = json!("haiku");
    bad["effort"] = json!("bogus");
    assert!(Tool::parse("project_start_slice", bad).is_err());
    let mut alone = base;
    alone["effort"] = json!("low");
    let alone = Tool::parse("project_start_slice", alone).expect("parses");
    assert_eq!(alone.validate(Role::Coordinator), Ok(()));
}

//! `session/request_permission`: agy asks before a tool call; Dovetail answers with one of the
//! options it offered (`allow_once`, `reject_once`, and the `_always` kinds when offered).

use super::rpc;
use crate::ControlAnswer;
use crate::event::{ControlId, PermissionRequest};
use serde_json::{Value, json};

/// A request waiting for its answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pending {
    /// The agent's JSON-RPC id, echoed in the answer.
    rpc_id: Value,
    /// `(kind, optionId)` of each option offered.
    options: Vec<(String, String)>,
}

/// The id the core answers by: the agent's JSON-RPC id as text (agy sends strings).
pub(crate) fn control_id(rpc_id: &Value) -> ControlId {
    ControlId(
        rpc_id
            .as_str()
            .map_or_else(|| rpc_id.to_string(), str::to_owned),
    )
}

/// The tool a request is for, as agy names it (`run_command`, `mcp_dovetail_core_ask_user`).
pub(crate) fn tool_name(msg: &Value) -> String {
    msg.pointer("/params/toolCall/name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The event for a request, and what answering it needs.
pub(crate) fn request_of(msg: &Value) -> (PermissionRequest, Pending) {
    let rpc_id = msg.get("id").cloned().unwrap_or(Value::Null);
    let call = msg.pointer("/params/toolCall").cloned().unwrap_or_default();
    let text = |key: &str| call.get(key).and_then(Value::as_str).map(str::to_owned);
    let options = msg
        .pointer("/params/options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            let kind = o.get("kind")?.as_str()?;
            let id = o.get("optionId")?.as_str()?;
            Some((kind.to_owned(), id.to_owned()))
        })
        .collect();
    let request = PermissionRequest {
        id: control_id(&rpc_id),
        tool_name: tool_name(msg),
        display_name: text("title"),
        input: call
            .get("rawInput")
            .or_else(|| call.get("args"))
            .cloned()
            .unwrap_or_default(),
        tool_use_id: text("toolCallId"),
        description: text("title"),
        decision_reason: None,
        decision_reason_type: None,
        suggestions: Vec::new(),
        agent_id: None,
        raw: msg.clone(),
    };
    (request, Pending { rpc_id, options })
}

impl Pending {
    /// The answer line for `answer`. "Always allow" picks `allow_always` when agy offered it,
    /// else allows once. A denial that also interrupts says so (the caller cancels the turn).
    pub(crate) fn answer(&self, answer: &ControlAnswer) -> (Value, bool) {
        let (kinds, interrupt): (&[&str], bool) = match answer {
            ControlAnswer::Allow {
                updated_permissions,
                ..
            } if !updated_permissions.is_empty() => (&["allow_always", "allow_once"], false),
            ControlAnswer::Allow { .. } => (&["allow_once", "allow_always"], false),
            ControlAnswer::Deny { interrupt, .. } => {
                (&["reject_once", "reject_always"], *interrupt)
            }
        };
        let chosen = kinds
            .iter()
            .find_map(|k| self.options.iter().find(|(kind, _)| kind == k));
        let outcome = match chosen {
            Some((_, option)) => json!({"outcome": "selected", "optionId": option}),
            // Nothing fits: cancelling is the protocol's way to refuse.
            None => json!({"outcome": "cancelled"}),
        };
        (
            rpc::result(&self.rpc_id, json!({"outcome": outcome})),
            interrupt,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({
            "jsonrpc": "2.0", "id": "1", "method": "session/request_permission",
            "params": {
                "sessionId": "s",
                "toolCall": {
                    "toolCallId": "call_1", "name": "run_command", "title": "touch b.txt",
                    "rawInput": {"CommandLine": "touch b.txt"}, "status": "pending",
                },
                "options": [
                    {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                    {"optionId": "deny", "name": "Deny", "kind": "reject_once"},
                ],
            },
        })
    }

    #[test]
    fn a_request_reads_its_tool_and_input() {
        let (req, _) = request_of(&request());
        assert_eq!(req.id, ControlId("1".into()));
        assert_eq!(req.tool_name, "run_command");
        assert_eq!(req.input, json!({"CommandLine": "touch b.txt"}));
        assert_eq!(req.tool_use_id.as_deref(), Some("call_1"));
        assert_eq!(req.description.as_deref(), Some("touch b.txt"));
    }

    #[test]
    fn answers_pick_an_offered_option() {
        let (_, pending) = request_of(&request());
        let (line, interrupt) = pending.answer(&ControlAnswer::allow());
        assert_eq!(
            line,
            json!({"jsonrpc": "2.0", "id": "1", "result": {"outcome": {"outcome": "selected", "optionId": "allow"}}})
        );
        assert!(!interrupt);
        // No allow_always offered: "always" allows once.
        let (line, _) = pending.answer(&ControlAnswer::allow_with_rules(vec![json!({})]));
        assert_eq!(line["result"]["outcome"]["optionId"], "allow");
        let (line, interrupt) = pending.answer(&ControlAnswer::Deny {
            message: "no".into(),
            interrupt: true,
        });
        assert_eq!(line["result"]["outcome"]["optionId"], "deny");
        assert!(interrupt);
    }

    #[test]
    fn without_a_fitting_option_the_request_is_cancelled() {
        let mut msg = request();
        msg["params"]["options"] = json!([]);
        let (_, pending) = request_of(&msg);
        let (line, _) = pending.answer(&ControlAnswer::allow());
        assert_eq!(line["result"]["outcome"], json!({"outcome": "cancelled"}));
    }
}

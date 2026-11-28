//! JSON-RPC 2.0 bookkeeping: our request ids and what each one was for, so an answer finds its
//! request whatever order answers come in.

use crate::event::SwitchId;
use dovetail_api::agents::model::ReasoningEffort;
use serde_json::{Value, json};
use std::collections::HashMap;

/// What one of our requests was for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    /// `initialize`.
    Initialize,
    /// `session/new` or `session/resume`.
    Open,
    /// The `session/set_model` that applies the spec's model at start.
    StartModel {
        /// The family and effort asked for.
        choice: (String, Option<ReasoningEffort>),
    },
    /// A `session/set_model` the core asked for.
    SetModel {
        /// The switch it answers.
        switch: SwitchId,
        /// The family and effort asked for.
        choice: (String, Option<ReasoningEffort>),
    },
    /// The `session/set_mode` that applies the spec's mode at start.
    StartMode,
    /// A later `session/set_mode`.
    SetMode,
    /// `session/prompt`: answered when the turn ends.
    Prompt,
}

/// A request's key: its id as JSON text, so a number and a string never collide.
fn key(id: &Value) -> String {
    id.to_string()
}

/// Our requests in flight.
#[derive(Debug, Default)]
pub(crate) struct Rpc {
    next: u64,
    pending: HashMap<String, Call>,
}

impl Rpc {
    /// A request line for `method`, remembered as `call`.
    pub(crate) fn request(&mut self, method: &str, params: Value, call: Call) -> Value {
        self.next += 1;
        let id = json!(self.next);
        self.pending.insert(key(&id), call);
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    /// The call an answer belongs to, once.
    pub(crate) fn take(&mut self, id: &Value) -> Option<Call> {
        self.pending.remove(&key(id))
    }

    /// A replayed fixture's request line, remembered by its method so its recorded answer maps
    /// as the live one would. Model switches replay as start-up ones: no fixture records a
    /// switch id.
    pub(crate) fn note_sent(&mut self, msg: &Value) {
        let (Some(id), Some(method)) = (msg.get("id"), msg.get("method").and_then(Value::as_str))
        else {
            return;
        };
        let call = match method {
            "initialize" => Call::Initialize,
            "session/new" | "session/resume" | "session/load" => Call::Open,
            "session/set_model" => Call::StartModel {
                choice: (String::new(), None),
            },
            "session/set_mode" => Call::StartMode,
            "session/prompt" => Call::Prompt,
            _ => return,
        };
        self.pending.insert(key(id), call);
    }
}

/// A notification line (no id, no answer).
pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// Our answer to one of the agent's requests.
pub(crate) fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Our refusal of an agent request we do not serve (we advertise no file system or terminal).
pub(crate) fn method_not_found(id: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not supported by Dovetail"}})
}

/// The message of an error answer.
pub(crate) fn error_text(msg: &Value) -> String {
    let error = msg.get("error");
    error
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.map(Value::to_string).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_find_their_request_once() {
        let mut rpc = Rpc::default();
        let first = rpc.request("initialize", json!({}), Call::Initialize);
        let second = rpc.request("session/prompt", json!({}), Call::Prompt);
        assert_eq!(first["id"], 1);
        assert_eq!(rpc.take(&second["id"]), Some(Call::Prompt));
        assert_eq!(rpc.take(&second["id"]), None);
        assert_eq!(rpc.take(&json!("1")), None, "a string id is another id");
        assert_eq!(rpc.take(&json!(1)), Some(Call::Initialize));
    }

    #[test]
    fn replayed_requests_are_remembered_by_method() {
        let mut rpc = Rpc::default();
        rpc.note_sent(&json!({"id": 7, "method": "session/new", "params": {}}));
        rpc.note_sent(&json!({"method": "session/cancel", "params": {}}));
        assert_eq!(rpc.take(&json!(7)), Some(Call::Open));
    }

    #[test]
    fn errors_read_their_message() {
        let msg = json!({"id": 1, "error": {"code": -32602, "message": "Unknown config option"}});
        assert_eq!(error_text(&msg), "Unknown config option");
    }
}

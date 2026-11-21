//! Live model and effort switches (task #39; `docs/spikes/live-model-effort.md`).
//!
//! A switch is a chain of control requests, each sent only after the previous one is answered,
//! since Claude Code answers control requests out of order: `set_model` when the model changes,
//! then `apply_flag_settings {effortLevel}` when an effort was picked, then `get_settings`. The
//! `applied` model and effort `get_settings` reports are the confirmed state:
//! `apply_flag_settings` accepts an unknown effort and silently ignores it. Every answer carries
//! its switch's [`SwitchId`], so the core never takes one switch's answer for another's.
//!
//! Apart from switches, the session's settings are read once at start, after the `initialize`
//! answer ([`Switches::after_initialize`]); that read reports with no switch id, and a failure of
//! it is only logged.

use std::collections::HashMap;

use dovetail_api::ReasoningEffort;
use serde_json::{Value, json};

use crate::event::{DriverEvent, ModelInfo, SwitchId};

/// Request id of the `list_models` request sent at start.
pub const LIST_MODELS_ID: &str = "dt_list_models";

/// What an answered request leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// `set_model` was sent; on success the effort (if any) goes next.
    Model {
        switch: SwitchId,
        effort: Option<ReasoningEffort>,
    },
    /// `apply_flag_settings` was sent; `get_settings` goes next.
    Effort { switch: SwitchId },
    /// `get_settings` was sent; its answer is the confirmed state. `None`: the start-up read.
    Settings { switch: Option<SwitchId> },
}

/// The switches in flight on one session, keyed by request id.
#[derive(Debug, Default)]
pub(crate) struct Switches {
    next_request: u64,
    next_switch: u64,
    steps: HashMap<String, Step>,
    /// The start-up read has been sent.
    started: bool,
}

/// What to do with a control response.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Outcome {
    /// The next request line to write, if the chain goes on.
    pub(crate) send: Option<Value>,
    /// The event to emit, if the chain ended.
    pub(crate) event: Option<DriverEvent>,
}

fn request(id: &str, request: Value) -> Value {
    json!({"type": "control_request", "request_id": id, "request": request})
}

impl Switches {
    fn id(&mut self, kind: &str, step: Step) -> String {
        let id = format!("dt_{kind}_{}", self.next_request);
        self.next_request += 1;
        self.steps.insert(id.clone(), step);
        id
    }

    /// The first line of a switch to `model` (`None`: keep the model) at `effort`, and the
    /// switch's id.
    pub(crate) fn begin(
        &mut self,
        model: Option<&str>,
        effort: Option<ReasoningEffort>,
    ) -> (SwitchId, Value) {
        let switch = SwitchId(self.next_switch);
        self.next_switch += 1;
        let line = match (model, effort) {
            (Some(model), _) => {
                let id = self.id("model", Step::Model { switch, effort });
                request(&id, json!({"subtype": "set_model", "model": model}))
            }
            (None, Some(effort)) => self.effort(switch, effort),
            (None, None) => self.read_settings(Some(switch)),
        };
        (switch, line)
    }

    /// The start-up requests, once, when the `initialize` answer has arrived: the settings read
    /// (launch model and effort) and `list_models` (each model's effort levels).
    pub(crate) fn after_initialize(&mut self) -> Vec<Value> {
        if std::mem::replace(&mut self.started, true) {
            return Vec::new();
        }
        vec![self.read_settings(None), list_models()]
    }

    fn read_settings(&mut self, switch: Option<SwitchId>) -> Value {
        let id = self.id("settings", Step::Settings { switch });
        request(&id, json!({"subtype": "get_settings"}))
    }

    fn effort(&mut self, switch: SwitchId, effort: ReasoningEffort) -> Value {
        let id = self.id("effort", Step::Effort { switch });
        request(
            &id,
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": effort.as_str()}}),
        )
    }

    /// Handle the answer to request `id`; `None` when the request is not one of ours here.
    /// `body` is the `response` payload, or the error text on failure.
    pub(crate) fn answer(&mut self, id: &str, success: bool, body: &Value) -> Option<Outcome> {
        let step = self.steps.remove(id)?;
        let next = |send| Outcome {
            send: Some(send),
            event: None,
        };
        let end = |event| Outcome {
            send: None,
            event: Some(event),
        };
        Some(match step {
            Step::Model { switch, .. } if !success => end(DriverEvent::ModelSwitchFailed {
                switch,
                error: error_text(body),
            }),
            Step::Model {
                switch,
                effort: Some(effort),
            } => next(self.effort(switch, effort)),
            // `apply_flag_settings` always says success; `get_settings` tells the truth.
            Step::Model {
                switch,
                effort: None,
            }
            | Step::Effort { switch } => next(self.read_settings(Some(switch))),
            Step::Settings { switch: None } if !success => {
                tracing::warn!(
                    "reading the session's settings at start failed: {}",
                    error_text(body)
                );
                Outcome::default()
            }
            Step::Settings {
                switch: Some(switch),
            } if !success => end(DriverEvent::ModelSwitchFailed {
                switch,
                error: format!("could not read the settings back: {}", error_text(body)),
            }),
            Step::Settings { switch } => end(settings_of(switch, body)),
        })
    }
}

fn error_text(body: &Value) -> String {
    match body {
        Value::String(s) => s.clone(),
        Value::Null => "no reason given".into(),
        other => other.to_string(),
    }
}

/// `get_settings`' `applied` model and effort.
fn settings_of(switch: Option<SwitchId>, body: &Value) -> DriverEvent {
    let applied = body.get("applied");
    let text = |key: &str| {
        applied
            .and_then(|a| a.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    DriverEvent::ModelSettings {
        switch,
        model: text("model"),
        effort: text("effort").as_deref().and_then(ReasoningEffort::parse),
    }
}

/// The `list_models` request sent at start.
pub(crate) fn list_models() -> Value {
    request(LIST_MODELS_ID, json!({"subtype": "list_models"}))
}

/// The models in a `list_models` answer's payload.
pub(crate) fn models_of(body: &Value) -> Vec<ModelInfo> {
    let Some(list) = body.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|m| {
            let text = |key: &str| m.get(key).and_then(Value::as_str).map(str::to_owned);
            Some(ModelInfo {
                value: text("value")?,
                resolved: text("resolvedModel"),
                display_name: text("displayName"),
                efforts: m
                    .get("supportedEffortLevels")
                    .and_then(Value::as_array)
                    .map(|levels| {
                        levels
                            .iter()
                            .filter_map(Value::as_str)
                            .filter_map(ReasoningEffort::parse)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_of(line: &Value) -> String {
        line["request_id"].as_str().unwrap_or_default().to_owned()
    }

    fn answer(s: &mut Switches, line: &Value, success: bool, body: Value) -> Outcome {
        s.answer(&id_of(line), success, &body).unwrap_or_default()
    }

    #[test]
    fn a_switch_chains_model_effort_and_settings_on_each_answer() {
        let mut s = Switches::default();
        let (switch, first) = s.begin(Some("sonnet"), Some(ReasoningEffort::High));
        assert_eq!(
            first["request"],
            json!({"subtype": "set_model", "model": "sonnet"})
        );
        assert_eq!(first["type"], "control_request");

        let next = answer(&mut s, &first, true, json!({"model": "claude-sonnet-5-5"}));
        assert_eq!(next.event, None);
        let effort = next.send.unwrap_or_default();
        assert_eq!(
            effort["request"],
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": "high"}})
        );

        let settings = answer(&mut s, &effort, true, Value::Null)
            .send
            .unwrap_or_default();
        assert_eq!(settings["request"], json!({"subtype": "get_settings"}));

        let applied = json!({"applied": {"model": "claude-sonnet-5-5", "effort": "high"}});
        let done = answer(&mut s, &settings, true, applied);
        assert_eq!(done.send, None);
        assert_eq!(
            done.event,
            Some(DriverEvent::ModelSettings {
                switch: Some(switch),
                model: Some("claude-sonnet-5-5".into()),
                effort: Some(ReasoningEffort::High),
            })
        );
    }

    #[test]
    fn a_refused_model_ends_the_switch_without_touching_effort() {
        let mut s = Switches::default();
        let (switch, first) = s.begin(Some("bogus"), Some(ReasoningEffort::Max));
        let out = answer(&mut s, &first, false, json!("Model 'bogus' not found"));
        assert_eq!(out.send, None);
        assert_eq!(
            out.event,
            Some(DriverEvent::ModelSwitchFailed {
                switch,
                error: "Model 'bogus' not found".into()
            })
        );
    }

    #[test]
    fn an_effort_only_switch_sends_no_set_model() {
        let mut s = Switches::default();
        let (_, first) = s.begin(None, Some(ReasoningEffort::Low));
        assert_eq!(first["request"]["subtype"], "apply_flag_settings");
        let settings = answer(&mut s, &first, true, Value::Null)
            .send
            .unwrap_or_default();
        assert_eq!(settings["request"]["subtype"], "get_settings");
    }

    #[test]
    fn without_an_effort_the_model_is_followed_by_settings() {
        let mut s = Switches::default();
        let (switch, first) = s.begin(Some("claude-haiku-4-5-20251001"), None);
        let settings = answer(&mut s, &first, true, Value::Null)
            .send
            .unwrap_or_default();
        assert_eq!(settings["request"]["subtype"], "get_settings");
        let done = answer(
            &mut s,
            &settings,
            true,
            json!({"applied": {"model": "claude-haiku-4-5-20251001", "effort": null}}),
        );
        assert_eq!(
            done.event,
            Some(DriverEvent::ModelSettings {
                switch: Some(switch),
                model: Some("claude-haiku-4-5-20251001".into()),
                effort: None,
            })
        );
    }

    #[test]
    fn each_switch_gets_its_own_id() {
        let mut s = Switches::default();
        let (a, _) = s.begin(Some("opus"), None);
        let (b, _) = s.begin(Some("opus"), None);
        assert_ne!(a, b);
    }

    #[test]
    fn the_startup_read_goes_once_reports_without_a_switch_and_fails_quietly() {
        let mut s = Switches::default();
        let lines = s.after_initialize();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["request"]["subtype"], "get_settings");
        assert_eq!(lines[1]["request_id"], LIST_MODELS_ID);
        assert!(s.after_initialize().is_empty(), "only once");
        let done = answer(
            &mut s,
            &lines[0],
            true,
            json!({"applied": {"model": "claude-opus-5-5", "effort": "high"}}),
        );
        assert_eq!(
            done.event,
            Some(DriverEvent::ModelSettings {
                switch: None,
                model: Some("claude-opus-5-5".into()),
                effort: Some(ReasoningEffort::High),
            })
        );

        let mut s = Switches::default();
        let lines = s.after_initialize();
        let failed = answer(&mut s, &lines[0], false, json!("nope"));
        assert_eq!(
            failed,
            Outcome::default(),
            "no switch error for the start-up read"
        );
    }

    #[test]
    fn answers_to_other_requests_are_not_ours() {
        let mut s = Switches::default();
        assert_eq!(s.answer("dt_0", true, &Value::Null), None);
    }

    #[test]
    fn list_models_keeps_values_and_effort_levels() {
        let body = json!({"models": [
            {"value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus 5.5",
             "supportsEffort": true, "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"]},
            {"value": "claude-haiku-4-5-20251001", "displayName": "Haiku 4.5"},
            {"displayName": "no value, skipped"},
        ]});
        let models = models_of(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].value, "opus");
        assert_eq!(models[0].resolved.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(models[0].efforts, ReasoningEffort::ALL.to_vec());
        assert!(models[1].efforts.is_empty());
    }
}

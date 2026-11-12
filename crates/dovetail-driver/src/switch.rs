//! Live model and effort switches (task #39; `docs/spikes/live-model-effort.md`).
//!
//! A switch is a chain of control requests, each sent only after the previous one is answered,
//! since Claude Code answers control requests out of order: `set_model`, then
//! `apply_flag_settings {effortLevel}` when an effort was picked, then `get_settings`. The
//! `applied` model and effort `get_settings` reports are the confirmed state:
//! `apply_flag_settings` accepts an unknown effort and silently ignores it.

use std::collections::HashMap;

use dovetail_api::Effort;
use serde_json::{Value, json};

use crate::event::{DriverEvent, ModelInfo};

/// Request id of the `list_models` request sent at start.
pub const LIST_MODELS_ID: &str = "dt_list_models";

/// What an answered request leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// `set_model` was sent; on success the effort (if any) goes next.
    Model { effort: Option<Effort> },
    /// `apply_flag_settings` was sent; `get_settings` goes next.
    Effort,
    /// `get_settings` was sent; its answer is the confirmed state.
    Settings,
}

/// The switches in flight on one session, keyed by request id.
#[derive(Debug, Default)]
pub(crate) struct Switches {
    next: u64,
    steps: HashMap<String, Step>,
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
        let id = format!("dt_{kind}_{}", self.next);
        self.next += 1;
        self.steps.insert(id.clone(), step);
        id
    }

    /// The first line of a switch to `model` at `effort`.
    pub(crate) fn begin(&mut self, model: &str, effort: Option<Effort>) -> Value {
        let id = self.id("model", Step::Model { effort });
        request(&id, json!({"subtype": "set_model", "model": model}))
    }

    /// A `get_settings` request whose answer becomes [`DriverEvent::ModelSettings`].
    pub(crate) fn read_settings(&mut self) -> Value {
        let id = self.id("settings", Step::Settings);
        request(&id, json!({"subtype": "get_settings"}))
    }

    fn effort(&mut self, effort: Effort) -> Value {
        let id = self.id("effort", Step::Effort);
        request(
            &id,
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": effort.as_str()}}),
        )
    }

    /// Handle the answer to request `id`; `None` when the request is not part of a switch.
    /// `body` is the `response` payload, or the error text on failure.
    pub(crate) fn answer(&mut self, id: &str, success: bool, body: &Value) -> Option<Outcome> {
        let step = self.steps.remove(id)?;
        Some(match step {
            Step::Model { .. } if !success => Outcome {
                send: None,
                event: Some(DriverEvent::ModelSwitchFailed {
                    error: error_text(body),
                }),
            },
            Step::Model {
                effort: Some(effort),
            } => Outcome {
                send: Some(self.effort(effort)),
                event: None,
            },
            // `apply_flag_settings` always says success; `get_settings` tells the truth.
            Step::Model { effort: None } | Step::Effort => Outcome {
                send: Some(self.read_settings()),
                event: None,
            },
            Step::Settings if !success => Outcome {
                send: None,
                event: Some(DriverEvent::ModelSwitchFailed {
                    error: format!("could not read the settings back: {}", error_text(body)),
                }),
            },
            Step::Settings => Outcome {
                send: None,
                event: Some(settings_of(body)),
            },
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
fn settings_of(body: &Value) -> DriverEvent {
    let applied = body.get("applied");
    let text = |key: &str| {
        applied
            .and_then(|a| a.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    DriverEvent::ModelSettings {
        model: text("model"),
        effort: text("effort").as_deref().and_then(Effort::parse),
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
                            .filter_map(Effort::parse)
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

    #[test]
    fn a_switch_chains_model_effort_and_settings_on_each_answer() {
        let mut s = Switches::default();
        let first = s.begin("sonnet", Some(Effort::High));
        assert_eq!(
            first["request"],
            json!({"subtype": "set_model", "model": "sonnet"})
        );
        assert_eq!(first["type"], "control_request");

        let next = s
            .answer(&id_of(&first), true, &json!({"model": "claude-sonnet-5-5"}))
            .unwrap_or_default();
        assert_eq!(next.event, None);
        let effort = next.send.unwrap_or_default();
        assert_eq!(
            effort["request"],
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": "high"}})
        );

        let next = s
            .answer(&id_of(&effort), true, &Value::Null)
            .unwrap_or_default();
        let settings = next.send.unwrap_or_default();
        assert_eq!(settings["request"], json!({"subtype": "get_settings"}));

        let applied = json!({"applied": {"model": "claude-sonnet-5-5", "effort": "high"}});
        let done = s
            .answer(&id_of(&settings), true, &applied)
            .unwrap_or_default();
        assert_eq!(done.send, None);
        assert_eq!(
            done.event,
            Some(DriverEvent::ModelSettings {
                model: Some("claude-sonnet-5-5".into()),
                effort: Some(Effort::High),
            })
        );
    }

    #[test]
    fn a_refused_model_ends_the_switch_without_touching_effort() {
        let mut s = Switches::default();
        let first = s.begin("bogus", Some(Effort::Max));
        let out = s
            .answer(&id_of(&first), false, &json!("Model 'bogus' not found"))
            .unwrap_or_default();
        assert_eq!(out.send, None);
        assert_eq!(
            out.event,
            Some(DriverEvent::ModelSwitchFailed {
                error: "Model 'bogus' not found".into()
            })
        );
    }

    #[test]
    fn without_an_effort_the_model_is_followed_by_settings() {
        let mut s = Switches::default();
        let first = s.begin("claude-haiku-4-5-20251001", None);
        let out = s
            .answer(&id_of(&first), true, &Value::Null)
            .unwrap_or_default();
        let settings = out.send.unwrap_or_default();
        assert_eq!(settings["request"]["subtype"], "get_settings");
        let done = s
            .answer(
                &id_of(&settings),
                true,
                &json!({"applied": {"model": "claude-haiku-4-5-20251001", "effort": null}}),
            )
            .unwrap_or_default();
        assert_eq!(
            done.event,
            Some(DriverEvent::ModelSettings {
                model: Some("claude-haiku-4-5-20251001".into()),
                effort: None,
            })
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
        assert_eq!(models[0].efforts, Effort::ALL.to_vec());
        assert!(models[1].efforts.is_empty());
    }
}

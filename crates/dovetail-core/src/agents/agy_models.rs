//! agy's model list, so the model picker has agy rows before any agy session has run (task
//! #46). The catalog starts from the list saved last time (`registry.db`, `driver_models`),
//! or a built-in one when nothing is saved. At start one background `agy models` replaces it
//! when it answers and differs; a session's own `Models` event does the same
//! (`session::model::on_models_listed`). A failed refresh keeps what is there and only logs.

use dovetail_api::Event;
use dovetail_api::agents::model::{DriverKind, ModelEvent, ReasoningEffort};
use dovetail_driver::{ModelInfo, UsageQuery};
use serde_json::{Value, json};

use crate::service::{Core, Input};

/// The families the spike found (`docs/spikes/agy-headless.md`), with the efforts each takes:
/// what the picker shows on a first run, until `agy models` answers.
const FALLBACK: &[(&str, &str, &[ReasoningEffort])] = &[
    ("claude-opus-5-5", "Claude Opus 5.5", &LOW_TO_HIGH),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5", &LOW_TO_HIGH),
    ("gemini-3.8-flash", "Gemini 3.8 Flash", &LOW_TO_HIGH),
    ("gemini-4-neon", "Gemini 4 Neon", &LOW_TO_XHIGH),
];
const LOW_TO_HIGH: [ReasoningEffort; 3] = [
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];
const LOW_TO_XHIGH: [ReasoningEffort; 4] = [
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
];

/// The built-in list.
pub(crate) fn fallback_models() -> Vec<ModelInfo> {
    FALLBACK
        .iter()
        .map(|(value, name, efforts)| ModelInfo {
            value: (*value).to_owned(),
            resolved: None,
            display_name: Some((*name).to_owned()),
            efforts: efforts.to_vec(),
        })
        .collect()
}

/// `models` as the JSON array that is saved.
fn to_json(models: &[ModelInfo]) -> String {
    let list: Vec<Value> = models
        .iter()
        .map(|m| {
            json!({
                "value": m.value,
                "resolved": m.resolved,
                "display_name": m.display_name,
                "efforts": m.efforts.iter().map(|e| e.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Value::Array(list).to_string()
}

/// The saved array back; `None` when it does not read (a list from another version).
fn from_json(text: &str) -> Option<Vec<ModelInfo>> {
    let list: Vec<Value> = serde_json::from_str(text).ok()?;
    let text_of = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    list.iter()
        .map(|m| {
            Some(ModelInfo {
                value: text_of(m, "value")?,
                resolved: text_of(m, "resolved"),
                display_name: text_of(m, "display_name"),
                efforts: m
                    .get("efforts")?
                    .as_array()?
                    .iter()
                    .filter_map(|e| ReasoningEffort::parse(e.as_str()?))
                    .collect(),
            })
        })
        .collect()
}

impl Core {
    /// Fills agy's catalog from the saved list, else the built-in one. Before the first
    /// snapshot, so the picker has agy rows at once.
    pub(crate) async fn load_agy_models(&mut self) {
        if self.options.drivers.get(DriverKind::Agy).is_none() {
            return;
        }
        let saved = match self.db.driver_models(DriverKind::Agy.as_str()).await {
            Ok(saved) => saved.as_deref().and_then(from_json),
            Err(e) => {
                tracing::warn!("reading the saved agy models: {e}");
                None
            }
        };
        let models = saved
            .filter(|m| !m.is_empty())
            .unwrap_or_else(fallback_models);
        self.model_catalog.insert(DriverKind::Agy, models);
    }

    /// Asks agy for its models in the background; the answer arrives as an [`Input`].
    pub(crate) fn refresh_agy_models(&self) {
        let Some(driver) = self.options.drivers.get(DriverKind::Agy).cloned() else {
            return;
        };
        let query = UsageQuery {
            cwd: self.query_cwd(),
            env: Vec::new(),
        };
        let inputs = self.inputs.clone();
        tokio::spawn(async move {
            let _ = inputs.send(Input::AgyModelsRefreshed(driver.models(query).await));
        });
    }

    /// `agy models` answered (or did not): a good answer replaces the catalog and is saved;
    /// a failed one leaves the catalog as it is.
    pub(crate) fn on_agy_models_refreshed(
        &mut self,
        result: Result<Vec<ModelInfo>, dovetail_driver::ModelsError>,
    ) {
        match result {
            Ok(models) if !models.is_empty() => {
                if self.model_catalog.get(&DriverKind::Agy) == Some(&models) {
                    return;
                }
                self.model_catalog.insert(DriverKind::Agy, models.clone());
                self.save_models(DriverKind::Agy, &models);
                self.emit(Event::Model(ModelEvent::Changed(self.model_options())));
            }
            Ok(_) => tracing::warn!("agy models listed nothing; keeping the known list"),
            Err(why) => tracing::warn!("agy models failed: {why}; keeping the known list"),
        }
    }

    /// Saves agy's list for the next start (Claude Code's list is the session's own and is not
    /// kept).
    pub(crate) fn save_models(&self, driver: DriverKind, models: &[ModelInfo]) {
        if driver == DriverKind::Agy {
            let saving = self.db.save_driver_models(driver.as_str(), to_json(models));
            tokio::spawn(async move {
                if let Err(e) = saving.await {
                    tracing::warn!("saving the agy models: {e}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_list_reads_back_as_it_was_written() {
        let models = fallback_models();
        assert_eq!(from_json(&to_json(&models)), Some(models));
    }

    #[test]
    fn a_list_that_does_not_read_is_none() {
        assert_eq!(from_json("not json"), None);
        assert_eq!(from_json(r#"[{"efforts": []}]"#), None);
    }
}

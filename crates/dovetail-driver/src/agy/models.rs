//! agy's models as Dovetail's model-and-effort choice.
//!
//! agy lists one id per model and effort (`gemini-3.8-flash-low`, display name
//! `Gemini 3.8 Flash (Low)`) plus a few without an effort (`gemini-pro-orchestrator-neon`).
//! Dovetail's selector shows a model and its effort levels, so the ids are grouped into
//! families: an id whose display name ends in `(<Effort>)` and whose id ends in `-<effort>` is
//! that family at that effort; anything else is a family of its own with no effort setting.

use crate::event::ModelInfo;
use dovetail_api::ReasoningEffort;
use serde_json::Value;

/// Asks `program models` for the account's models.
pub(crate) async fn list(
    program: std::ffi::OsString,
    query: crate::UsageQuery,
) -> Result<Vec<ModelInfo>, String> {
    let text = crate::query::run_text(program, vec!["models".into()], &query)
        .await
        .map_err(|e| e.to_string())?;
    let infos = ModelTable::from_listing(&text).infos();
    if infos.is_empty() {
        return Err("agy models listed no models".into());
    }
    Ok(infos)
}

/// One family: the name the selector passes back, and the agy id for each effort.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Family {
    name: String,
    display_name: String,
    /// `(effort, agy id)`, lowest effort first; one entry with `None` for a model without
    /// effort levels.
    ids: Vec<(Option<ReasoningEffort>, String)>,
}

/// The models a session offers, from `session/new`'s `models.availableModels`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelTable {
    families: Vec<Family>,
}

/// agy's effort word in a display name (`High`, `XHigh`).
fn effort_of_label(label: &str) -> Option<ReasoningEffort> {
    ReasoningEffort::parse(&label.to_ascii_lowercase())
}

/// Splits one listed model into (family, display name, effort).
fn split(id: &str, name: &str) -> (String, String, Option<ReasoningEffort>) {
    if let Some(open) = name.rfind(" (")
        && let Some(label) = name[open + 2..].strip_suffix(')')
        && let Some(effort) = effort_of_label(label)
        && let Some(family) = id.strip_suffix(&format!("-{}", effort.as_str()))
    {
        return (family.to_owned(), name[..open].to_owned(), Some(effort));
    }
    (id.to_owned(), name.to_owned(), None)
}

impl ModelTable {
    /// Reads `availableModels` (`[{modelId, name}]`); entries without an id are skipped.
    pub(crate) fn from_available(list: &Value) -> Self {
        let mut table = Self::default();
        for entry in list.as_array().into_iter().flatten() {
            let Some(id) = entry.get("modelId").and_then(Value::as_str) else {
                continue;
            };
            let name = entry.get("name").and_then(Value::as_str).unwrap_or(id);
            let (family, display_name, effort) = split(id, name);
            match table.families.iter_mut().find(|f| f.name == family) {
                Some(f) => f.ids.push((effort, id.to_owned())),
                None => table.families.push(Family {
                    name: family,
                    display_name,
                    ids: vec![(effort, id.to_owned())],
                }),
            }
        }
        for family in &mut table.families {
            family.ids.sort();
        }
        table
    }

    /// Reads the `agy models` listing: an optional "Fetching available models..." line, then
    /// `<id>\t<label>` lines. Anything else is skipped.
    pub(crate) fn from_listing(text: &str) -> Self {
        let list: Vec<Value> = text
            .lines()
            .filter_map(|line| {
                let (id, name) = line.split_once('\t')?;
                let id = id.trim();
                (!id.is_empty() && !id.contains(' '))
                    .then(|| serde_json::json!({"modelId": id, "name": name.trim()}))
            })
            .collect();
        Self::from_available(&Value::Array(list))
    }

    /// The selector's view: one entry per family with the efforts it accepts.
    pub(crate) fn infos(&self) -> Vec<ModelInfo> {
        self.families
            .iter()
            .map(|f| ModelInfo {
                value: f.name.clone(),
                resolved: None,
                display_name: Some(f.display_name.clone()),
                efforts: f.ids.iter().filter_map(|(e, _)| *e).collect(),
            })
            .collect()
    }

    /// The agy id for `family` at `effort`. A family with effort levels and no effort asked
    /// gets its highest level; an effort it does not list is `None`.
    pub(crate) fn id_for(&self, family: &str, effort: Option<ReasoningEffort>) -> Option<&str> {
        let f = self.families.iter().find(|f| f.name == family)?;
        let found = match effort {
            Some(e) => f.ids.iter().find(|(fe, _)| *fe == Some(e)),
            None => f
                .ids
                .iter()
                .find(|(fe, _)| fe.is_none())
                .or_else(|| f.ids.last()),
        };
        found.map(|(_, id)| id.as_str())
    }

    /// The family and effort an agy id stands for.
    pub(crate) fn choice_of(&self, id: &str) -> Option<(String, Option<ReasoningEffort>)> {
        self.families.iter().find_map(|f| {
            f.ids
                .iter()
                .find(|(_, fid)| fid == id)
                .map(|(e, _)| (f.name.clone(), *e))
        })
    }

    /// What a `configOptions` list says is in use: the `model` option's value with the
    /// `thought_level` option's, or the value alone when it is a whole id.
    pub(crate) fn choice_from_config(
        &self,
        options: &Value,
    ) -> Option<(String, Option<ReasoningEffort>)> {
        let current = |id: &str| {
            options
                .as_array()?
                .iter()
                .find(|o| o.get("id").and_then(Value::as_str) == Some(id))?
                .get("currentValue")?
                .as_str()
                .map(str::to_owned)
        };
        let model = current("model")?;
        if let Some(level) = current("thought_level")
            && let Some(choice) = self.choice_of(&format!("{model}-{level}"))
        {
            return Some(choice);
        }
        self.choice_of(&model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_models_listing_groups_into_families_and_skips_the_progress_line() {
        let table = ModelTable::from_listing(
            "Fetching available models...\n\
             gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n\
             gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n\
             gemini-pro-orchestrator-neon\tOrchestrator (Neon)\n",
        );
        let infos = table.infos();
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].value, "gemini-3.8-flash");
        assert_eq!(
            infos[0].efforts,
            vec![ReasoningEffort::Low, ReasoningEffort::High]
        );
        assert!(infos[1].efforts.is_empty());
    }

    fn table() -> ModelTable {
        ModelTable::from_available(&json!([
            {"modelId": "gemini-3.8-flash-high", "name": "Gemini 3.8 Flash (High)"},
            {"modelId": "gemini-3.8-flash-low", "name": "Gemini 3.8 Flash (Low)"},
            {"modelId": "gemini-4-neon-xhigh", "name": "Gemini 4 Neon (XHigh)"},
            {"modelId": "gemini-pro-orchestrator-neon", "name": "Orchestrator (Neon)"},
            {"modelId": "gpt-oss-120b-medium", "name": "GPT-OSS 120B (Medium)"},
            {"name": "no id"},
        ]))
    }

    #[test]
    fn ids_group_into_families_with_their_efforts() {
        let infos = table().infos();
        let names: Vec<_> = infos.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(
            names,
            [
                "gemini-3.8-flash",
                "gemini-4-neon",
                "gemini-pro-orchestrator-neon",
                "gpt-oss-120b"
            ]
        );
        assert_eq!(
            infos[0].efforts,
            [ReasoningEffort::Low, ReasoningEffort::High]
        );
        assert_eq!(infos[0].display_name.as_deref(), Some("Gemini 3.8 Flash"));
        assert_eq!(infos[1].efforts, [ReasoningEffort::Xhigh]);
        assert!(
            infos[2].efforts.is_empty(),
            "the orchestrator has no levels"
        );
        assert_eq!(
            infos[2].display_name.as_deref(),
            Some("Orchestrator (Neon)")
        );
    }

    #[test]
    fn a_choice_becomes_an_id_and_back() {
        let t = table();
        assert_eq!(
            t.id_for("gemini-3.8-flash", Some(ReasoningEffort::Low)),
            Some("gemini-3.8-flash-low")
        );
        assert_eq!(
            t.id_for("gemini-3.8-flash", None),
            Some("gemini-3.8-flash-high"),
            "no effort asked: the highest level"
        );
        assert_eq!(
            t.id_for("gemini-3.8-flash", Some(ReasoningEffort::Max)),
            None
        );
        assert_eq!(
            t.id_for("gemini-pro-orchestrator-neon", None),
            Some("gemini-pro-orchestrator-neon")
        );
        assert_eq!(t.id_for("unknown", None), None);
        assert_eq!(
            t.choice_of("gpt-oss-120b-medium"),
            Some(("gpt-oss-120b".into(), Some(ReasoningEffort::Medium)))
        );
    }

    #[test]
    fn config_options_name_the_choice_in_use() {
        let t = table();
        let options = json!([
            {"id": "model", "currentValue": "gemini-3.8-flash"},
            {"id": "thought_level", "currentValue": "low"},
        ]);
        assert_eq!(
            t.choice_from_config(&options),
            Some(("gemini-3.8-flash".into(), Some(ReasoningEffort::Low)))
        );
        // agy lists GPT-OSS's whole id as the model value.
        let options = json!([
            {"id": "model", "currentValue": "gpt-oss-120b-medium"},
            {"id": "thought_level", "currentValue": "high"},
        ]);
        assert_eq!(
            t.choice_from_config(&options),
            Some(("gpt-oss-120b".into(), Some(ReasoningEffort::Medium)))
        );
    }
}

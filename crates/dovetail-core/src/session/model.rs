//! The model selector's core side (task #39, `docs/spikes/live-model-effort.md`): switching a
//! session's model and effort live, storing the choice for its next start, and the list of
//! models the selector offers.
//!
//! A running session switches through the driver and the agent changes only when Claude Code
//! confirms (`DriverEvent::ModelSettings`), never optimistically. A refused switch keeps the old
//! model and is reported; nothing restarts.

use dovetail_api::{AgentId, AgentKind, Effort, Event, ModelOption};
use dovetail_driver::ModelInfo;

use crate::db::{AgentRow, ModelChoice};
use crate::service::Core;

impl Core {
    /// `Command::SetModel`: switch `id`'s session (a subagent's parent, since subagents share
    /// it) to `model` at `effort`.
    pub(crate) fn switch_model(&mut self, id: &AgentId, model: String, effort: Option<Effort>) {
        let model = model.trim().to_owned();
        if model.is_empty() {
            self.fail("No model given");
            return;
        }
        let Some(row) = self.agents.get(id) else {
            self.fail(format!("no agent {}", id.0));
            return;
        };
        let target = match (&row.agent.kind, &row.agent.parent) {
            (AgentKind::Subagent { .. }, Some(parent)) => parent.clone(),
            _ => id.clone(),
        };
        let Some(name) = self.agents.get(&target).map(|r| r.agent.name.clone()) else {
            return;
        };
        let choice = ModelChoice { model, effort };
        let Some(live) = self.live.get_mut(&target) else {
            // Not running (or taken over): the choice is used when it next starts.
            self.update_agent(&target, |row| row.model_choice = Some(choice.clone()));
            self.notice(
                &target,
                format!(
                    "{name} will use {} when it next starts",
                    describe(&choice.model, choice.effort)
                ),
            );
            return;
        };
        if let Err(e) = live.session.set_model(&choice.model, choice.effort) {
            self.fail(format!("Could not switch {name}'s model: {e}"));
            return;
        }
        live.switching = Some(choice);
    }

    /// `get_settings`' answer: at start, or at the end of a switch.
    pub(crate) fn on_model_settings(
        &mut self,
        agent: &AgentId,
        model: Option<String>,
        effort: Option<Effort>,
    ) {
        let asked = self.live.get_mut(agent).and_then(|l| l.switching.take());
        self.set_model(agent, model.as_deref());
        self.update_agent(agent, |row| row.agent.effort = effort);
        let Some(mut asked) = asked else {
            return;
        };
        // `apply_flag_settings` accepts a level the model lacks and ignores it: only the
        // effort read back counts.
        if let Some(wanted) = asked.effort
            && effort != Some(wanted)
        {
            let name = self.agent_name(agent);
            self.fail(format!(
                "{name} switched to {} but kept effort {}: the model did not take {}",
                asked.model,
                effort.map_or("none", Effort::as_str),
                wanted.as_str()
            ));
            asked.effort = effort;
        }
        self.update_agent(agent, |row| row.model_choice = Some(asked));
    }

    /// A switch was refused: the old model is still in use.
    pub(crate) fn on_model_switch_failed(&mut self, agent: &AgentId, error: &str) {
        let asked = self.live.get_mut(agent).and_then(|l| l.switching.take());
        let name = self.agent_name(agent);
        let old = self
            .agents
            .get(agent)
            .and_then(|r| r.agent.model.clone())
            .unwrap_or_else(|| "its model".into());
        let what = asked.map_or_else(|| "another model".into(), |c| c.model);
        self.fail(format!(
            "Could not switch {name} to {what}: {error}. It keeps {old}."
        ));
    }

    /// A session listed its models; the selector learns which effort levels each accepts.
    pub(crate) fn on_models_listed(&mut self, models: Vec<ModelInfo>) {
        if models.is_empty() || models == self.model_catalog {
            return;
        }
        self.model_catalog = models;
        self.emit(Event::ModelsChanged(self.model_options()));
    }

    /// The models the selector offers: `config.toml`'s names, with what sessions reported.
    pub(crate) fn model_options(&self) -> Vec<ModelOption> {
        options(&self.config.model_names(), &self.model_catalog)
    }

    /// `--model` and `--effort` for `row`'s next start: the user's choice, else the default
    /// of its kind.
    pub(crate) fn launch_model(&self, row: &AgentRow) -> (Option<String>, Option<Effort>) {
        if let Some(choice) = &row.model_choice {
            return (Some(choice.model.clone()), choice.effort);
        }
        let default = if matches!(
            row.agent.kind,
            AgentKind::SideChat | AgentKind::Ask | AgentKind::Tangent
        ) {
            &self.config.side_chat_model
        } else {
            &self.config.default_model
        };
        (default.clone(), None)
    }

    fn agent_name(&self, agent: &AgentId) -> String {
        self.agents
            .get(agent)
            .map_or_else(|| agent.0.clone(), |r| r.agent.name.clone())
    }
}

/// "sonnet at high effort", or just the model.
fn describe(model: &str, effort: Option<Effort>) -> String {
    match effort {
        Some(e) => format!("{model} at {} effort", e.as_str()),
        None => model.to_owned(),
    }
}

/// `names` in order, each with what `catalog` says about it (matched by alias or full name).
fn options(names: &[String], catalog: &[ModelInfo]) -> Vec<ModelOption> {
    names
        .iter()
        .map(|name| {
            let info = catalog
                .iter()
                .find(|m| &m.value == name || m.resolved.as_ref() == Some(name));
            ModelOption {
                name: name.clone(),
                resolved: info.and_then(|m| m.resolved.clone()),
                efforts: info.map(|m| m.efforts.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(value: &str, resolved: &str, efforts: &[Effort]) -> ModelInfo {
        ModelInfo {
            value: value.into(),
            resolved: Some(resolved.into()),
            display_name: None,
            efforts: efforts.to_vec(),
        }
    }

    #[test]
    fn options_take_effort_levels_from_the_catalog_by_alias_or_full_name() {
        let names: Vec<String> = ["opus", "claude-haiku-4-5-20251001", "mystery"]
            .map(String::from)
            .to_vec();
        let catalog = [
            info("opus", "claude-opus-5-5", &Effort::ALL),
            info(
                "claude-haiku-4-5-20251001",
                "claude-haiku-4-5-20251001",
                &[],
            ),
        ];
        let got = options(&names, &catalog);
        assert_eq!(got[0].resolved.as_deref(), Some("claude-opus-5-5"));
        assert!(got[0].supports(Effort::Max));
        assert_eq!(got[1].efforts, Some(Vec::new()));
        assert!(!got[1].supports(Effort::Low));
        // Not listed by any session: every level is offered.
        assert_eq!(got[2], ModelOption::named("mystery"));
        assert!(got[2].supports(Effort::Xhigh));
    }
}

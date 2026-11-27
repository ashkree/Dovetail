//! The model selector's core side (task #39, `docs/spikes/live-model-effort.md`, task #46):
//! switching a session's model and effort live, storing the choice for its next start, and the
//! per-driver list of models the selector offers.
//!
//! A running session switches through its driver and the agent changes only when the driver
//! confirms (`DriverEvent::ModelSettings`), never optimistically. A refused switch keeps the old
//! model and is reported; nothing restarts. Every answer names its switch, and only the answer
//! to the session's latest request counts ([`is_current`]): a late start-up read or an earlier
//! switch's answer never confirms another switch. One switch at a time: another is refused until
//! the first is confirmed or refused. Each driver keeps its own entry in `model_catalog`.

use dovetail_api::Event;
use dovetail_api::agents::{
    agent::{AgentId, AgentKind},
    model::{DriverKind, ModelEvent, ModelOption, ReasoningEffort},
};
use dovetail_driver::{ModelInfo, SwitchId};

use crate::db::{AgentRow, ModelChoice};
use crate::service::Core;

/// A switch sent to a live session and not yet answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSwitch {
    /// The driver's id for it.
    pub(crate) id: SwitchId,
    /// What the user picked.
    pub(crate) choice: ModelChoice,
}

/// Whether a settings answer for `answer` (`None`: the start-up read) is the answer to the
/// session's latest request, `latest` being its latest switch (`None`: none sent yet). An older
/// answer, arriving out of order, says nothing about the session now.
pub(crate) fn is_current(latest: Option<SwitchId>, answer: Option<SwitchId>) -> bool {
    latest == answer
}

impl Core {
    /// `ModelCommand::Set`: switch `id`'s session (a subagent's parent, since subagents share
    /// it) to `model` at `effort`.
    pub(crate) fn switch_model(
        &mut self,
        id: &AgentId,
        model: String,
        effort: Option<ReasoningEffort>,
    ) {
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
        let Some(row) = self.agents.get(&target) else {
            return;
        };
        let name = row.agent.name.clone();
        // Only the effort changes: no `set_model` (spike note).
        let keep_model = self.runs_model(row, &model);
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
        if let Some(pending) = &live.switching {
            let waiting = describe(&pending.choice.model, pending.choice.effort);
            self.fail(format!(
                "{name} is still switching to {waiting}: try again once it has answered"
            ));
            return;
        }
        let model = (!keep_model).then_some(choice.model.as_str());
        match live.session.set_model(model, choice.effort) {
            Ok(id) => {
                live.last_switch = Some(id);
                live.switching = Some(PendingSwitch { id, choice });
            }
            Err(e) => self.fail(format!("Could not switch {name}'s model: {e}")),
        }
    }

    /// Whether `row`'s session already runs `model` (an alias or a full name).
    fn runs_model(&self, row: &AgentRow, model: &str) -> bool {
        row.agent
            .model
            .as_deref()
            .is_some_and(|running| self.runs_model_named(row.agent.driver, running, model))
    }

    /// Whether `running` (a resolved name) on `driver` is `model` (an alias or a full name).
    fn runs_model_named(&self, driver: DriverKind, running: &str, model: &str) -> bool {
        running == model
            || self
                .model_catalog
                .get(&driver)
                .into_iter()
                .flatten()
                .any(|m| m.value == model && m.resolved.as_deref() == Some(running))
    }

    /// Whether `driver` lists `model` (by alias or full name): `None` while no session of that
    /// driver has listed its models, when it cannot be told.
    pub(crate) fn model_listed(&self, driver: DriverKind, model: &str) -> Option<bool> {
        self.model_catalog
            .get(&driver)
            .filter(|c| !c.is_empty())
            .map(|catalog| {
                catalog
                    .iter()
                    .any(|m| m.value == model || m.resolved.as_deref() == Some(model))
            })
    }

    /// The listed models' names for `driver`, comma-separated, for an error.
    pub(crate) fn model_catalog_names(&self, driver: DriverKind) -> String {
        let names: Vec<&str> = self
            .model_catalog
            .get(&driver)
            .into_iter()
            .flatten()
            .map(|m| m.value.as_str())
            .collect();
        names.join(", ")
    }

    /// A session launched with a model and effort: the start-up read says what the driver
    /// applied. A mismatch goes in the session's log and, for a slice, to the coordinator.
    /// Returns what the session actually runs, to keep on its row: a later start compares
    /// with that, so a mismatch is reported once.
    fn check_launch_choice(
        &mut self,
        agent: &AgentId,
        choice: ModelChoice,
        model: Option<&str>,
        effort: Option<ReasoningEffort>,
    ) -> ModelChoice {
        let name = self.agent_name(agent);
        let driver = self
            .agents
            .get(agent)
            .map_or(DriverKind::Claude, |r| r.agent.driver);
        let mut kept = choice.clone();
        let mut problems = Vec::new();
        // The applied model is the resolved name (`claude-sonnet-5-5`). Without a model list to
        // map the alias, the name must at least contain it: a typo like `opuss` does not.
        if let Some(applied) = model
            && !self.runs_model_named(driver, applied, &choice.model)
            && !applied.contains(&choice.model)
        {
            problems.push(format!("model {} (it runs {applied})", choice.model));
            kept.model = applied.to_owned();
        }
        if let Some(wanted) = choice.effort
            && effort != Some(wanted)
        {
            problems.push(format!(
                "effort {} (it runs at {})",
                wanted.as_str(),
                effort.map_or("none", ReasoningEffort::as_str)
            ));
            kept.effort = effort;
        }
        if !problems.is_empty() {
            let text = format!(
                "{name} did not get what was asked for: {}",
                problems.join(" and ")
            );
            self.notice(agent, text.clone());
            if self.is_slice(agent) {
                self.tell_coordinator(format!("slice {text}"));
            }
        }
        kept
    }

    pub(crate) fn is_slice(&self, agent: &AgentId) -> bool {
        self.agents
            .get(agent)
            .is_some_and(|r| r.agent.kind == AgentKind::Slice)
    }

    /// `get_settings`' answer: the start-up read (`switch` is `None`) or the end of a switch.
    pub(crate) fn on_model_settings(
        &mut self,
        agent: &AgentId,
        switch: Option<SwitchId>,
        model: Option<String>,
        effort: Option<ReasoningEffort>,
    ) {
        let Some(live) = self.live.get_mut(agent) else {
            return;
        };
        if !is_current(live.last_switch, switch) {
            tracing::debug!(agent = %agent.0, ?switch, "an older settings answer, ignored");
            return;
        }
        let asked = live.switching.take_if(|p| Some(p.id) == switch);
        // Only the start-up read (no switch) answers for the launch flags.
        let launched = if switch.is_none() {
            live.launch_choice.take()
        } else {
            None
        };
        self.set_model(agent, model.as_deref());
        self.update_agent(agent, |row| row.agent.effort = effort);
        if let Some(choice) = launched {
            let kept = self.check_launch_choice(agent, choice, model.as_deref(), effort);
            self.update_agent(agent, |row| row.model_choice = Some(kept));
        }
        let Some(PendingSwitch { mut choice, .. }) = asked else {
            return;
        };
        // `apply_flag_settings` accepts a level the model lacks and ignores it: only the
        // effort read back counts.
        if let Some(wanted) = choice.effort
            && effort != Some(wanted)
        {
            let name = self.agent_name(agent);
            self.fail(format!(
                "{name} switched to {} but kept effort {}: the model did not take {}",
                choice.model,
                effort.map_or("none", ReasoningEffort::as_str),
                wanted.as_str()
            ));
            choice.effort = effort;
        }
        self.update_agent(agent, |row| row.model_choice = Some(choice));
    }

    /// Switch `switch` was refused: the old model is still in use.
    pub(crate) fn on_model_switch_failed(
        &mut self,
        agent: &AgentId,
        switch: SwitchId,
        error: &str,
    ) {
        let asked = self
            .live
            .get_mut(agent)
            .and_then(|l| l.switching.take_if(|p| p.id == switch));
        let Some(asked) = asked else {
            tracing::debug!(agent = %agent.0, ?switch, "a refusal for no pending switch, ignored");
            return;
        };
        let name = self.agent_name(agent);
        let old = self
            .agents
            .get(agent)
            .and_then(|r| r.agent.model.clone())
            .unwrap_or_else(|| "its model".into());
        self.fail(format!(
            "Could not switch {name} to {}: {error}. It keeps {old}.",
            asked.choice.model
        ));
    }

    /// A session listed its models; the selector learns which models and effort levels each
    /// driver accepts.
    pub(crate) fn on_models_listed(&mut self, agent: &AgentId, models: Vec<ModelInfo>) {
        let Some(driver) = self.agents.get(agent).map(|r| r.agent.driver) else {
            return;
        };
        if models.is_empty() || self.model_catalog.get(&driver) == Some(&models) {
            return;
        }
        self.model_catalog.insert(driver, models.clone());
        self.save_models(driver, &models);
        self.emit(Event::Model(ModelEvent::Changed(self.model_options())));
    }

    /// The models the selector offers: `config.toml`'s Claude names matched against Claude's
    /// catalog, followed by agy's reported families.
    pub(crate) fn model_options(&self) -> Vec<ModelOption> {
        let mut out = options(
            &self.config.model_names(),
            self.model_catalog
                .get(&DriverKind::Claude)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
        if let Some(agy) = self.model_catalog.get(&DriverKind::Agy) {
            out.extend(agy.iter().map(|m| ModelOption {
                name: m.value.clone(),
                resolved: m.resolved.clone(),
                efforts: Some(m.efforts.clone()),
                driver: DriverKind::Agy,
            }));
        }
        out
    }

    /// `--model` and `--effort` for `row`'s next start: the user's choice, else the default
    /// of its kind (Claude Code only; agy uses its own default when no choice is stored).
    pub(crate) fn launch_model(&self, row: &AgentRow) -> (Option<String>, Option<ReasoningEffort>) {
        if let Some(choice) = &row.model_choice {
            return (Some(choice.model.clone()), choice.effort);
        }
        if row.agent.driver != DriverKind::Claude {
            return (None, None);
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

    pub(crate) fn agent_name(&self, agent: &AgentId) -> String {
        self.agents
            .get(agent)
            .map_or_else(|| agent.0.clone(), |r| r.agent.name.clone())
    }
}

/// "sonnet at high effort", or just the model.
pub(crate) fn describe(model: &str, effort: Option<ReasoningEffort>) -> String {
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
                driver: dovetail_api::agents::model::DriverKind::Claude,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(value: &str, resolved: &str, efforts: &[ReasoningEffort]) -> ModelInfo {
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
            info("opus", "claude-opus-5-5", &ReasoningEffort::ALL),
            info(
                "claude-haiku-4-5-20251001",
                "claude-haiku-4-5-20251001",
                &[],
            ),
        ];
        let got = options(&names, &catalog);
        assert_eq!(got[0].resolved.as_deref(), Some("claude-opus-5-5"));
        assert!(got[0].supports(ReasoningEffort::Max));
        assert_eq!(got[1].efforts, Some(Vec::new()));
        assert!(!got[1].supports(ReasoningEffort::Low));
        // Not listed by any session: every level is offered.
        assert_eq!(got[2], ModelOption::named("mystery"));
        assert!(got[2].supports(ReasoningEffort::Xhigh));
    }
}

//! The model selector's core side (task #39, `docs/spikes/live-model-effort.md`): switching a
//! session's model and effort live, storing the choice for its next start, and the list of
//! models the selector offers.
//!
//! A running session switches through the driver and the agent changes only when Claude Code
//! confirms (`DriverEvent::ModelSettings`), never optimistically. A refused switch keeps the old
//! model and is reported; nothing restarts. Every answer names its switch, and only the answer
//! to the session's latest request counts ([`is_current`]): a late start-up read or an earlier
//! switch's answer never confirms another switch. One switch at a time: another is refused until
//! the first is confirmed or refused.

use dovetail_api::{AgentId, AgentKind, Event, ModelOption, ReasoningEffort};
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
    /// `Command::SetModel`: switch `id`'s session (a subagent's parent, since subagents share
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
        let Some(running) = row.agent.model.as_deref() else {
            return false;
        };
        running == model
            || self
                .model_catalog
                .iter()
                .any(|m| m.value == model && m.resolved.as_deref() == Some(running))
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
        self.set_model(agent, model.as_deref());
        self.update_agent(agent, |row| row.agent.effort = effort);
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
    pub(crate) fn launch_model(&self, row: &AgentRow) -> (Option<String>, Option<ReasoningEffort>) {
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
fn describe(model: &str, effort: Option<ReasoningEffort>) -> String {
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

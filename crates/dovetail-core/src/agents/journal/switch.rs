//! Driver switching (`ModelCommand::SwitchDriver`): the target, the checks, and applying it to the
//! coordinator row. The handoff and relaunch flow is shared with `:fresh` in the parent module.

use super::*;
use crate::storage::AgentRow;

/// Where [`ModelCommand::SwitchDriver`](dovetail_api::agents::model::ModelCommand::SwitchDriver) moves the coordinator
/// once its old session stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwitchTarget {
    pub(crate) driver: DriverKind,
    pub(crate) model_choice: ModelChoice,
}

/// Points the coordinator row at the switch target. Shared by the never-started case and the
/// relaunch after a handoff ([`Core::fresh_coordinator`]).
pub(super) fn apply_switch(row: &mut AgentRow, target: &SwitchTarget) {
    row.agent.driver = target.driver;
    row.model_choice = Some(target.model_choice.clone());
    row.agent.model = None;
    row.agent.effort = None;
    if target.driver != DriverKind::Claude {
        row.claude_home = None;
    }
}

/// The first prompt of a driver-switched coordinator when the old session could not write a
/// handoff: names why (`NoHandoff`), Dovetail's facts, and the old transcript path.
pub(super) fn fallback_seed_prompt(
    today: Date,
    time: &str,
    why: &NoHandoff,
    facts: &str,
    old_transcript: Option<&Path>,
) -> String {
    let transcript = old_transcript
        .map(|p| format!(" Its transcript is at {}.", p.display()))
        .unwrap_or_default();
    format!(
        "This is a fresh coordinator session, started at {time} on {today}. The previous \
         coordinator could not write a handoff ({why}); its conversation is kept but not \
         continued.{transcript}\n\n# Facts\n\n{facts}\n\n\
         Read this, check anything you need to, then tell the user in a few lines what you \
         would pick up first. Start no work until they answer."
    )
}

impl Core {
    /// `ModelCommand::SwitchDriver`: move the coordinator to `driver` on `model` / `effort`.
    /// When the coordinator has never started, updates the row directly; otherwise keeps the
    /// old driver on the row until the old session finishes its handoff (or clean stop) and
    /// exits.
    pub(crate) fn switch_driver(
        &mut self,
        id: &AgentId,
        driver: DriverKind,
        model: String,
        effort: Option<ReasoningEffort>,
        handoff: bool,
    ) {
        let Some(row) = self.agents.rows.get(id) else {
            self.fail(format!("no agent {}", id.0));
            return;
        };
        if id.0 != COORDINATOR || row.agent.kind != AgentKind::Coordinator {
            self.fail("only the coordinator can switch driver");
            return;
        }
        if self.sessions.live.get(id).is_some_and(|l| l.turn.running) {
            self.fail("turn running; try again when it finishes");
            return;
        }
        if self.agents.closing.is_some() {
            self.fail("already closing the day: waiting for the coordinator's handoff");
            return;
        }
        if self.agents.pending_fresh.is_some() {
            self.fail("a fresh coordinator is starting");
            return;
        }
        if self.locked(id) {
            self.fail(SessionError::TakenOver(row.agent.name.clone()));
            return;
        }
        if self.options.drivers.get(driver).is_none() {
            self.fail(SessionError::NoDriver(driver));
            return;
        }
        let model = model.trim().to_owned();
        if model.is_empty() {
            self.fail("No model given");
            return;
        }
        if self.model_listed(driver, &model) == Some(false) {
            self.fail(format!(
                "there is no model `{model}` for {}; the models are: {}",
                driver.label(),
                self.model_catalog_names(driver)
            ));
            return;
        }
        if let Some(wanted) = effort
            && let Some(catalog) = self.agents.model_catalog.get(&driver)
            && let Some(info) = catalog
                .iter()
                .find(|m| m.value == model || m.resolved.as_deref() == Some(model.as_str()))
            && !info.efforts.contains(&wanted)
        {
            self.fail(format!(
                "model `{model}` on {} does not support effort {}",
                driver.label(),
                wanted.as_str()
            ));
            return;
        }

        let target = SwitchTarget {
            driver,
            model_choice: ModelChoice { model, effort },
        };
        let has_run = row.transcript_path.is_some();
        let is_live = self.sessions.live.contains_key(id);
        if !has_run && !is_live {
            self.update_agent(id, |row| apply_switch(row, &target));
            self.notice(
                id,
                format!(
                    "Coordinator will start on {} ({})",
                    target.driver.label(),
                    describe(&target.model_choice.model, target.model_choice.effort)
                ),
            );
            return;
        }

        let time = jiff::Zoned::now().strftime("%H:%M").to_string();
        if handoff {
            self.ask_for_handoff(Some(time), Some(target));
        } else {
            let date = today();
            self.activity(
                dovetail_api::activity::ActivityKind::Lifecycle,
                Some(id),
                format!(
                    "switching coordinator to {} at {time}, no handoff",
                    target.driver.label()
                ),
            );
            self.begin_fresh(PendingFresh {
                kind: Fresh::Clean { date, time },
                seed: None,
                switch: Some(target),
            });
        }
    }
}

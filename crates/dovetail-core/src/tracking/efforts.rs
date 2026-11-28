//! Efforts, their acceptance criteria and the effort tag on chat lines (`docs/design-v2.md` §4).
//!
//! The core keeps every effort in memory (there are few), loaded at open and replaced whenever
//! the database returns a changed one. Each handler writes through the DB thread first, then
//! tells the UI with [`EffortEvent::Upserted`], which carries the whole effort.
//!
//! Approving a criterion is reachable only from [`dovetail_api::tracking::effort::EffortCommand::ApproveCriterion`]:
//! no tool handler calls it, so an agent can claim (`coordination_claim_criterion`) but never
//! tick. The user may approve a criterion nobody claimed, as a shortcut (decided 2026-11-16).

use dovetail_api::Event;
use dovetail_api::activity::ActivityKind;
use dovetail_api::agents::{agent::AgentId, conversation::EntryKind};
use dovetail_api::tracking::{
    effort::{
        CriterionId, Effort, EffortChange, EffortEvent, EffortId, NewEffort, WhereWasI, effort_name,
    },
    task::TaskId,
};

use dovetail_tools::{SocketResponseBody, coordination::ClaimCriterion};

use crate::engine::Core;
use crate::storage::{CriterionStep, DbError};
use crate::tools::{error, ok};

impl Core {
    /// Reads every effort, and each agent's effort tag from its conversation: the tag of the
    /// last message the user sent it.
    pub(crate) async fn load_efforts(&mut self) -> Result<(), DbError> {
        self.tracking.efforts = self
            .db
            .load_efforts()
            .await?
            .into_iter()
            .map(|e| (e.id, e))
            .collect();
        for (agent, tail) in &self.agents.tails {
            let last_user = tail
                .entries
                .iter()
                .rev()
                .find(|e| e.kind == EntryKind::User);
            if let Some(effort) = last_user.and_then(|e| e.effort) {
                self.tracking.effort_tags.insert(agent.clone(), effort);
            }
        }
        Ok(())
    }

    /// The tag a new entry of `agent` gets: the agent's current tag for what the user wrote and
    /// the agent's answer to it, none for Dovetail's own notices and for messages from other
    /// agents.
    pub(crate) fn entry_tag(&self, agent: &AgentId, kind: EntryKind) -> Option<EffortId> {
        match kind {
            EntryKind::User | EntryKind::Assistant | EntryKind::ToolSummary => {
                self.tracking.effort_tags.get(agent).copied()
            }
            EntryKind::Notice | EntryKind::AgentMessage => None,
        }
    }

    /// Sets the tag for `agent`'s next entries from a prompt's `effort`. `false`, with the
    /// command failed, when it names no effort of this project.
    pub(crate) fn tag_prompt(&mut self, agent: &AgentId, effort: Option<EffortId>) -> bool {
        match effort {
            Some(id) if !self.tracking.efforts.contains_key(&id) => {
                self.fail(format!("no effort {}", id.0));
                false
            }
            Some(id) => {
                self.tracking.effort_tags.insert(agent.clone(), id);
                true
            }
            None => {
                self.tracking.effort_tags.remove(agent);
                true
            }
        }
    }

    /// Keeps a changed effort and tells the UI; a refusal goes back as a failed command.
    fn effort_changed(&mut self, result: Result<Effort, DbError>) -> Option<Effort> {
        match result {
            Ok(effort) => {
                self.tracking.efforts.insert(effort.id, effort.clone());
                self.emit(Event::Effort(EffortEvent::Upserted(effort.clone())));
                Some(effort)
            }
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }

    pub(crate) async fn create_effort(&mut self, mut new: NewEffort) {
        new.name = effort_name(&new.name);
        new.question_or_goal = new.question_or_goal.trim().to_owned();
        let result = self.db.create_effort(new).await;
        if let Some(effort) = self.effort_changed(result) {
            self.activity(
                ActivityKind::Task,
                None,
                format!("effort {}: created", effort.name),
            );
        }
    }

    pub(crate) async fn rename_effort(&mut self, id: EffortId, name: String) {
        let result = self.db.rename_effort(id, effort_name(&name)).await;
        self.effort_changed(result);
    }

    pub(crate) async fn update_effort(&mut self, id: EffortId, patch: Vec<EffortChange>) {
        let result = self.db.update_effort(id, patch).await;
        self.effort_changed(result);
    }

    pub(crate) async fn set_where_was_i(&mut self, id: EffortId, card: WhereWasI) {
        let result = self.db.set_where_was_i(id, card).await;
        self.effort_changed(result);
    }

    pub(crate) async fn add_criterion(&mut self, effort: EffortId, text: String) {
        let result = self.db.add_criterion(effort, text).await;
        self.effort_changed(result);
    }

    /// The user claims a criterion met, with evidence; it now waits for approval.
    pub(crate) async fn claim_criterion(&mut self, id: CriterionId, evidence: String) {
        let result = self
            .db
            .step_criterion(id, CriterionStep::Claim { evidence })
            .await;
        if let Some(effort) = self.effort_changed(result) {
            self.criterion_activity(&effort, id, "claimed met");
        }
    }

    /// `coordination_claim_criterion` from `agent`: the same claim, answered to the agent.
    /// Validation already refused other roles and empty evidence. It can only claim: a met
    /// criterion is refused, and nothing here approves.
    pub(crate) async fn claim_criterion_tool(
        &mut self,
        agent: &AgentId,
        claim: ClaimCriterion,
    ) -> SocketResponseBody {
        let id = claim.criterion;
        let result = self
            .db
            .step_criterion(
                id,
                CriterionStep::Claim {
                    evidence: claim.evidence,
                },
            )
            .await;
        match result {
            Ok(effort) => {
                self.tracking.efforts.insert(effort.id, effort.clone());
                self.emit(Event::Effort(EffortEvent::Upserted(effort.clone())));
                let who = self
                    .agents
                    .rows
                    .get(agent)
                    .map_or_else(|| agent.0.clone(), |r| r.agent.name.clone());
                self.criterion_activity(&effort, id, &format!("claimed met by {who}"));
                ok(format!(
                    "Criterion {} of effort {} is claimed; it waits for the user's approval.",
                    id.0, effort.name
                ))
            }
            Err(e) => error(format!("could not claim criterion {}: {e}", id.0)),
        }
    }

    /// The user approves a criterion. Called for [`dovetail_api::tracking::effort::EffortCommand::ApproveCriterion`]
    /// only.
    pub(crate) async fn approve_criterion(&mut self, id: CriterionId) {
        let result = self.db.step_criterion(id, CriterionStep::Approve).await;
        if let Some(effort) = self.effort_changed(result) {
            self.criterion_activity(&effort, id, "approved by you");
            let text = criterion_text(&effort, id);
            self.tell_coordinator(format!(
                "the user approved criterion {} of effort {}: \"{text}\" is met",
                id.0, effort.name
            ));
        }
    }

    /// The user sends a criterion back to open; the coordinator is told why.
    pub(crate) async fn return_criterion(&mut self, id: CriterionId, reason: Option<String>) {
        let result = self.db.step_criterion(id, CriterionStep::Return).await;
        if let Some(effort) = self.effort_changed(result) {
            self.criterion_activity(&effort, id, "sent back by you");
            let text = criterion_text(&effort, id);
            let why = reason
                .map(|r| r.trim().to_owned())
                .filter(|r| !r.is_empty())
                .map(|r| format!(": {r}"))
                .unwrap_or_default();
            self.tell_coordinator(format!(
                "the user sent criterion {} of effort {} back as not met (\"{text}\"){why}",
                id.0, effort.name
            ));
        }
    }

    pub(crate) async fn move_task(
        &mut self,
        id: TaskId,
        effort: EffortId,
        criterion: Option<CriterionId>,
    ) {
        match self.db.move_task(id, effort, criterion).await {
            Ok(tasks) => self.emit_tasks(tasks),
            Err(e) => self.fail(e),
        }
    }

    fn criterion_activity(&mut self, effort: &Effort, id: CriterionId, what: &str) {
        let text = criterion_text(effort, id);
        self.activity(
            ActivityKind::Task,
            None,
            format!("effort {}: \"{text}\" {what}", effort.name),
        );
    }
}

/// A criterion's text, for messages; empty when the effort does not hold it.
fn criterion_text(effort: &Effort, id: CriterionId) -> String {
    effort
        .criteria
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.text.clone())
        .unwrap_or_default()
}

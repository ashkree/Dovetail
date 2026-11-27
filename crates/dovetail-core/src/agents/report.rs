//! A slice's report to the coordinator (`coordination_report`, ADR 0022).
//!
//! A report goes through Dovetail, never by session name, so it cannot reach another project's
//! coordinator. It shows as one line in the coordinator's conversation, tagged with the effort of
//! the slice's task, and reaches the coordinator's session as a message. A `done` report moves
//! the task to review (ADR 0024).

use dovetail_api::activity::ActivityKind;
use dovetail_api::agents::{
    agent::AgentId,
    conversation::{EntryKind, agent_message_text},
};
use dovetail_api::tracking::{
    effort::EffortId,
    task::{TaskChange, TaskStatus},
};
use dovetail_driver::UserInput;
use dovetail_tools::{
    SocketResponseBody,
    coordination::{Report, ReportState},
};

use crate::service::{COORDINATOR, Core, EntryTag};
use crate::tools::{error, ok};

impl Core {
    /// Handles `coordination_report` from `slice`. Validation already refused other roles and
    /// an empty summary.
    pub(crate) async fn slice_report(
        &mut self,
        slice: &AgentId,
        report: Report,
    ) -> SocketResponseBody {
        let Some(row) = self.agents.get(slice) else {
            return error(format!("unknown agent `{}`", slice.0));
        };
        let name = row.agent.name.clone();
        let task = match row.agent.task {
            Some(id) => self.task(id).await,
            None => None,
        };
        // A new report ends the hold on a task the user sent back from review.
        if let Some(task) = &task {
            self.sent_back.remove(&task.id);
        }
        let pr = report.pr.map(|n| format!(" (PR #{n})")).unwrap_or_default();
        let body = format!("[{}] {}{pr}", report.state.as_str(), report.summary.trim());
        let about = task
            .as_ref()
            .map(|t| format!(" on task #{} {}", t.id.0, t.title))
            .unwrap_or_default();
        self.report_to_coordinator(
            format!("Report from slice {name}{about}: {body}"),
            agent_message_text(&name, false, &body),
            task.as_ref().map(|t| t.effort),
        );
        let coordinator = AgentId(COORDINATOR.into());
        self.activity(
            ActivityKind::Message,
            Some(slice),
            format!("{name} → {COORDINATOR}"),
        );
        self.message(slice.clone(), coordinator, body, None);

        // A report that names a pull request, or says the work is done, is a reason to look.
        if report.pr.is_some() || report.state == ReportState::Done {
            self.refresh_worktree_after_turn(slice);
        }
        // Done moves the task to review at once when this slice is the only one on it; with
        // several, their pull requests decide (the one furthest behind).
        let mut moved = String::new();
        if let Some(task) = &task
            && report.state == ReportState::Done
            && task.status == TaskStatus::InProgress
            && self.task_slices(task.id).len() == 1
        {
            self.update_task(task.id, vec![TaskChange::Status(TaskStatus::Review)])
                .await;
            moved = format!(" Task #{} is now in review.", task.id.0);
        }
        ok(format!("Reported to the coordinator.{moved}"))
    }

    /// Shows a report in the coordinator's conversation with `effort` as its tag, and sends
    /// `text` to its session. A coordinator that cannot be woken now gets the text when its
    /// session next starts; the line shows at once either way.
    fn report_to_coordinator(&mut self, text: String, shown: String, effort: Option<EffortId>) {
        let id = AgentId(COORDINATOR.into());
        let tag = EntryTag::Effort(effort);
        if self.coordinator_wakeable() {
            let sent = self.send_input_tagged(
                &id,
                UserInput::text(text.as_str()),
                EntryKind::AgentMessage,
                shown.clone(),
                Vec::new(),
                tag,
            );
            match sent {
                Ok(()) => return,
                Err(e) => tracing::warn!("could not send a report to the coordinator: {e}"),
            }
        }
        self.add_entry_tagged(&id, EntryKind::AgentMessage, shown, Vec::new(), tag);
        self.hold_for_coordinator(text);
    }
}

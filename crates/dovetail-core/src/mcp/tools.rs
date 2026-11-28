//! The MCP tools' thin router (M9, ADR 0002–0005). The bridge parses and validates each call,
//! and the core checks the role and tool suites again before delegating to the owning domain.

use dovetail_api::tracking::task::TaskId;
use dovetail_tools::{
    SocketResponseBody, Tool,
    coordination::CoordinationTool,
    project::{
        ProjectTool,
        archive_result::{ARCHIVED, WAITING_FOR_USER},
    },
    report::ReportTool,
};
use tokio::sync::oneshot;

use crate::agents::SliceError;
use crate::agents::slice::{ArchiveCall, SliceStart};
use crate::engine::Core;
use crate::mcp::socket::ToolCall;

pub(crate) fn ok(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: false,
    }
}

pub(crate) fn error(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: true,
    }
}

/// `project_start_slice`'s answer: `started`, `queued` or `denied` first, then what to do next.
pub(crate) fn slice_reply(result: Result<SliceStart, SliceError>) -> SocketResponseBody {
    match result {
        Ok(SliceStart::Started(name)) => ok(format!(
            "started: slice {name} is setting up. Wait for \"slice {name} ready\" before you \
             message it."
        )),
        Ok(SliceStart::Queued(name)) => ok(format!(
            "queued: the maximum number of slices is running; {name} starts when one is \
             archived, and you will get \"slice {name} ready\" then."
        )),
        Err(reason) => error(format!("denied: {reason}")),
    }
}

/// Answers a settled `project_archive_slice` / `tasks_archive` call: `archived` or `waiting_for_user`
/// first, or an error.
pub(crate) fn answer_archive(call: ArchiveCall) {
    let _ = call.reply.send(archive_reply(
        call.task,
        &call.archived,
        &call.waiting,
        &call.failed,
    ));
}

fn archive_reply(
    task: Option<TaskId>,
    archived: &[String],
    waiting: &[String],
    failed: &[String],
) -> SocketResponseBody {
    let names = |list: &[String]| list.join(", ");
    let waiting_text = |list: &[String]| {
        format!(
            "{WAITING_FOR_USER}: slice {} has uncommitted or unpushed work. The user is asked \
             whether it may go; the answer arrives as a message, \"slice <name> archived\" or \
             \"slice <name> kept\".",
            names(list)
        )
    };
    match task {
        None => {
            if !failed.is_empty() {
                error(format!("could not archive slice {}", names(failed)))
            } else if !waiting.is_empty() {
                ok(waiting_text(waiting))
            } else {
                ok(format!(
                    "{ARCHIVED}: slice {} is gone: session stopped, worktree and branch \
                     removed, its place freed.",
                    names(archived)
                ))
            }
        }
        Some(id) => {
            let mut text = if !waiting.is_empty() {
                format!("{} Task #{} is archived.", waiting_text(waiting), id.0)
            } else {
                format!("{ARCHIVED}: task #{}.", id.0)
            };
            if !archived.is_empty() {
                text.push_str(&format!(" Archived slice {}.", names(archived)));
            }
            if failed.is_empty() {
                ok(text)
            } else {
                error(format!("{text} Could not archive slice {}.", names(failed)))
            }
        }
    }
}

impl Core {
    /// Answers one tool call, at once: `core_ask_user` too, whose answer arrives later as a message
    /// (ADR 0007).
    pub(crate) async fn tool(
        &mut self,
        call: ToolCall,
        reply: oneshot::Sender<SocketResponseBody>,
    ) {
        let ToolCall { agent, call, .. } = call;
        let Some(role) = self.role_of(&agent) else {
            let _ = reply.send(SocketResponseBody::Error {
                message: format!("unknown agent `{}`", agent.0),
            });
            return;
        };
        if !Tool::allowed(call.name(), self.config.suites.for_role(role)) {
            let _ = reply.send(error(format!(
                "`{}` is not in this session's tool suites",
                call.name()
            )));
            return;
        }
        if let Err(problem) = call.validate(role) {
            let _ = reply.send(error(problem.to_string()));
            return;
        }
        let answer = match call {
            Tool::Report(ReportTool::AskUser(ask)) => self.handle_ask_user_tool(&agent, ask),
            Tool::Report(ReportTool::ReportStatus(report)) => {
                self.handle_report_status_tool(&agent, report)
            }
            Tool::Report(ReportTool::LogDecision(decision)) => {
                self.handle_log_decision_tool(&agent, decision).await
            }
            Tool::Coordination(CoordinationTool::Report(report)) => {
                self.slice_report(&agent, report).await
            }
            Tool::Coordination(CoordinationTool::ClaimCriterion(claim)) => {
                self.claim_criterion_tool(&agent, claim).await
            }
            Tool::Canvas(tool) => self.handle_canvas_tool(&agent, tool).await,
            Tool::Design(tool) => self.handle_design_tool(&agent, tool).await,
            Tool::Map(tool) => self.handle_map_tool(&agent, tool).await,
            Tool::Research(tool) => self.handle_research_tool(&agent, tool).await,
            Tool::Project(ProjectTool::StartSlice(start)) => {
                self.handle_start_slice_tool(&agent, start).await
            }
            Tool::Project(ProjectTool::AddProject(_)) => self.handle_add_project_tool(&agent).await,
            Tool::Project(ProjectTool::ArchiveSlice(archive)) => {
                self.handle_archive_slice_tool(archive, reply).await;
                return;
            }
            Tool::Task(tool) => {
                self.handle_task_tool(&agent, tool, reply).await;
                return;
            }
        };
        let _ = reply.send(answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_tools::Role;

    #[test]
    fn start_slice_answers_lead_with_the_outcome() {
        let text = |body| match body {
            SocketResponseBody::ToolResult { text, .. } => text,
            other => panic!("{other:?}"),
        };
        assert!(text(slice_reply(Ok(SliceStart::Started("a".into())))).starts_with("started"));
        assert!(text(slice_reply(Ok(SliceStart::Queued("a".into())))).starts_with("queued"));
        assert!(text(slice_reply(Err(SliceError::Reserved))).starts_with("denied"));
    }

    #[test]
    fn suites_are_checked_with_the_shared_rule_for_new_and_old_names() {
        let slice = Role::Slice.default_suites();
        let coordinator = Role::Coordinator.default_suites();
        for name in [
            "project_start_slice",
            "start_slice",
            "project_add",
            "add_project",
            "project_archive_slice",
            "tasks_archive",
            "tasks_list",
            "tasks_get",
            "tasks_update",
            "research_get",
        ] {
            assert!(!Tool::allowed(name, slice), "{name}");
            assert!(Tool::allowed(name, coordinator), "{name}");
        }
        for name in ["core_ask_user", "ask_user", "maps_draw", "draw_map"] {
            assert!(Tool::allowed(name, slice), "{name}");
        }
        let side_chat = Role::SideChat.default_suites();
        assert!(Tool::allowed("core_log_decision", side_chat));
        assert!(!Tool::allowed("maps_draw", side_chat));
    }
}

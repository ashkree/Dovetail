//! The Research suite's tools (R2, `docs/research.md` §6, ADR 0015): `research_start_round`,
//! `research_write_report`, `research_propose` and `research_get`, and what happens when the user
//! answers a proposal.
//!
//! Rounds open through the same core path as `ResearchCommand::StartRound`. A report is validated
//! (`research::report`) before anything is written. A proposal never writes anything itself: it
//! raises an inbox item, and `y` does the write.

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::tools::{ResearchGet, ResearchPropose, ResearchStartRound, ResearchWriteReport};
use dovetail_api::{
    Event,
    activity::ActivityKind,
    agent::AgentId,
    decision::{DecisionAuthor, NewDecision},
    inbox::{InboxId, InboxKind, PermissionAnswer},
    task::{NewTask, TaskId, TaskStatus},
};
use dovetail_api::{
    decision::DecisionEvent,
    research::{Evidence, ProposalKind, ResearchContent},
};
use serde_json::json;

use super::{error, ok};
use crate::db::research::TaskResearch;
use crate::inbox::Waiter;
use crate::research::files;
use crate::research::report::{self, Known, KnownSource, id_ranges};
use crate::service::Core;

/// Most characters of a document `research_get` returns; the rest is left on disk.
const GET_MAX_CHARS: usize = 20_000;
/// Most characters of a follow-up task's title.
const TITLE_MAX: usize = 120;

/// What a task has to cite, from its stored research.
fn known(r: &TaskResearch) -> Known {
    Known {
        findings: r
            .findings
            .iter()
            .map(|f| (f.id, f.sources.clone()))
            .collect(),
        sources: r
            .sources
            .iter()
            .map(|s| {
                (
                    s.id,
                    KnownSource {
                        kind: s.kind,
                        locator: s.locator.clone(),
                    },
                )
            })
            .collect(),
    }
}

impl Core {
    /// The task, if it exists and is not archived; else the error to return.
    async fn live_task(&self, id: TaskId) -> Result<dovetail_api::task::Task, SocketResponseBody> {
        match self.task(id).await {
            None => Err(error(format!("there is no task #{}", id.0))),
            Some(t) if t.status == TaskStatus::Archived => {
                Err(error(format!("task #{} is archived", id.0)))
            }
            Some(t) => Ok(t),
        }
    }

    /// `research_start_round`.
    pub(super) async fn research_start_round(
        &mut self,
        args: ResearchStartRound,
    ) -> SocketResponseBody {
        let task = args.task;
        if let Err(refused) = self.live_task(task).await {
            return refused;
        }
        if let Some(kind) = args.kind
            && let Err(e) = self
                .db
                .set_research_kind(task, Some(kind.as_str().into()))
                .await
        {
            return error(format!("could not set the research kind: {e}"));
        }
        match self.open_round_for(task, args.brief).await {
            Ok((round, None)) => {
                let head = json!({ "round": round.n, "dir": round.dir, "status": "open" });
                ok(format!(
                    "{head}\nRound {} is open: the subagents you launch now are filed under it, \
                     with their sources. Write the report with research_write_report when they \
                     are done.",
                    round.n
                ))
            }
            Ok((round, Some(position))) => {
                let head = json!({
                    "round": round.n, "dir": round.dir, "status": "queued", "position": position
                });
                ok(format!(
                    "{head}\nRound {} is queued ({} in line): another round is open, and its \
                     brief is saved. Do not launch its subagents yet; when it opens you get the \
                     message \"Round {} for #{} is now open; send its subagents.\"",
                    round.n,
                    dovetail_api::research::ordinal(position),
                    round.n,
                    task.0
                ))
            }
            Err(e) => error(e.to_string()),
        }
    }

    /// `research_write_report`.
    pub(super) async fn research_write_report(
        &mut self,
        args: ResearchWriteReport,
    ) -> SocketResponseBody {
        let task = args.task;
        let t = match self.live_task(task).await {
            Ok(t) => t,
            Err(refused) => return refused,
        };
        let r = match self.db.task_research(task).await {
            Ok(r) => r,
            Err(e) => return error(format!("could not read the research: {e}")),
        };
        let open = r.rounds.iter().find(|x| x.closed_ms.is_none() && !x.queued);
        if open.is_none() && r.rounds.iter().all(|x| x.queued) {
            return error(format!(
                "task #{} has no open round and no earlier round, so there is nothing to \
                 report on: open one with research_start_round first",
                task.0
            ));
        }
        let decision = match self.db.research_kind(task).await {
            Ok(kind) => kind.as_deref() == Some("decision"),
            Err(e) => return error(format!("could not read the task: {e}")),
        };
        let checked = match report::check(&args.body, decision, &known(&r)) {
            Ok(checked) => checked,
            Err(refusal) => {
                return error(format!("The report was not written. {refusal}"));
            }
        };
        let dir = match self.research_dir(task, &t.title) {
            Ok(dir) => dir,
            Err(e) => return error(e.to_string()),
        };
        // The round this report closes, or else the latest one.
        let round_id = open.or(r.rounds.iter().rfind(|x| !x.queued)).map(|x| x.id);
        let flags = (
            u32::try_from(checked.uncited.len()).unwrap_or(u32::MAX),
            u32::try_from(checked.unlabelled_figures.len()).unwrap_or(u32::MAX),
        );
        if let Err(e) = self
            .db
            .insert_report(task, round_id, checked.body.clone(), flags)
            .await
        {
            return error(format!("could not store the report: {e}"));
        }
        let path = dir.join("report.md");
        let file = files::write_atomic(&path, &checked.body);
        // Ask sessions were grounded in the old report (ADR 0018).
        self.close_asks_of(task);
        let closed = open.is_some();
        if closed {
            if let Err(e) = self.close_round_for(task).await {
                tracing::warn!("closing the round after the report: {e}");
            }
        } else {
            self.emit_research(task, &t.title).await;
        }
        self.activity(
            ActivityKind::Task,
            None,
            format!("research report written on #{} {}", task.0, t.title),
        );
        let head = json!({
            "written": true,
            "uncited": checked.uncited,
            "unlabelled_figures": checked.unlabelled_figures,
            "citations_resolved": checked.citations_resolved,
        });
        let mut text = format!(
            "{head}\nEvidence line rewritten by Dovetail: {} sources, {} independent.",
            checked.sources, checked.independent
        );
        if closed {
            text.push_str(" The open round is closed.");
        }
        if flags != (0, 0) {
            text.push_str(
                " Flagged claims and figures were stored as written and are marked for the \
                 user; cite them, label them, or soften them in the next version.",
            );
        }
        match file {
            Ok(()) => ok(text),
            Err(e) => {
                // The row is the source of truth; the file is rebuilt at the next start.
                error(format!(
                    "{text}\nBut {} could not be written ({e}); the report is stored and the \
                     file is rebuilt when Dovetail next starts.",
                    path.display()
                ))
            }
        }
    }

    /// `research_propose`: raises the inbox item and returns at once.
    pub(super) async fn research_propose(
        &mut self,
        agent: &AgentId,
        args: ResearchPropose,
    ) -> SocketResponseBody {
        let task = args.task;
        if let Err(refused) = self.live_task(task).await {
            return refused;
        }
        let evidence = match args.parsed_evidence() {
            Ok(evidence) => evidence,
            Err(e) => return error(e.to_string()),
        };
        let r = match self.db.task_research(task).await {
            Ok(r) => r,
            Err(e) => return error(format!("could not read the research: {e}")),
        };
        for e in &evidence {
            let unresolved = match e {
                Evidence::Finding(id) if !r.findings.iter().any(|f| f.id == *id) => {
                    let ids: Vec<i64> = r.findings.iter().map(|f| f.id.0).collect();
                    Some(format!("findings in this task: {}", id_ranges('F', &ids)))
                }
                Evidence::Source(id) if !r.sources.iter().any(|s| s.id == *id) => {
                    let ids: Vec<i64> = r.sources.iter().map(|s| s.id.0).collect();
                    Some(format!("sources in this task: {}", id_ranges('S', &ids)))
                }
                _ => None,
            };
            if let Some(known) = unresolved {
                return error(format!("evidence {e} does not resolve; {known}"));
            }
        }
        let kind = args.kind;
        let text = args.text.trim().to_owned();
        let item = InboxKind::Proposal {
            task,
            kind,
            text: text.clone(),
            evidence,
            why: args.why.filter(|w| !w.trim().is_empty()),
            alternatives: args.alternatives,
        };
        self.raise(agent, item, Waiter::Proposal);
        ok(format!(
            "pending: the user is asked to accept this {0}; nothing is written until they do. \
             The answer arrives as a message: \"research_propose accepted ...\" or \
             \"research_propose dropped ...\". Carry on, or end your turn.",
            kind.as_str()
        ))
    }

    /// `research_get`: one stored document, shortened.
    pub(super) async fn research_get(&mut self, args: ResearchGet) -> SocketResponseBody {
        let what = match args.target() {
            Ok(what) => what,
            Err(e) => return error(e.to_string()),
        };
        match self.research_content(args.task, &what).await {
            Ok(ResearchContent::Document(doc)) => {
                let total = doc.markdown.chars().count();
                let mut text: String = doc.markdown.chars().take(GET_MAX_CHARS).collect();
                if total > GET_MAX_CHARS {
                    text.push_str(&format!(
                        "\n\n[cut at {GET_MAX_CHARS} of {total} characters{}]",
                        doc.path
                            .map(|p| format!("; the whole file is {}", p.display()))
                            .unwrap_or_default()
                    ));
                }
                ok(text)
            }
            Ok(ResearchContent::Listing(_)) => error("a listing is not a document"),
            Err(e) => error(e.to_string()),
        }
    }

    /// `y`, `A` or `d` on a proposal. The inbox item leaves Needs you first; the write follows,
    /// and the coordinator is told what happened as a short user message.
    pub(crate) async fn answer_proposal(&mut self, id: InboxId, answer: PermissionAnswer) {
        let label = match &answer {
            PermissionAnswer::Allow => "accepted",
            PermissionAnswer::AllowAlways => "accepted as a new round",
            PermissionAnswer::Deny { .. } => "dropped",
        };
        let Some(entry) = self.resolve(id, Some(label.into())) else {
            return;
        };
        let agent = entry.item.agent;
        let InboxKind::Proposal {
            task,
            kind,
            text,
            evidence,
            why,
            alternatives,
        } = entry.item.kind
        else {
            return;
        };
        let what = kind.as_str();
        let shown = first_line(&text).to_owned();
        self.activity(
            ActivityKind::Task,
            Some(&agent),
            format!("research {what} on #{}: {label}: {shown}", task.0),
        );
        let message = match (answer, kind) {
            (PermissionAnswer::Deny { message }, _) => {
                let why = message.map(|m| format!(": {m}")).unwrap_or_default();
                format!("research_propose dropped: the user declined your {what} \"{shown}\"{why}")
            }
            (PermissionAnswer::AllowAlways, ProposalKind::Idea) => {
                match self.open_round_for(task, text).await {
                    Ok((round, None)) => format!(
                        "research_propose accepted: round {} is open on task #{} with your idea \
                         as its brief; launch the subagents",
                        round.n, task.0
                    ),
                    Ok((round, Some(position))) => format!(
                        "research_propose accepted: round {} on task #{} is queued ({} in line) \
                         with your idea as its brief; wait for \"Round {} for #{} is now open\" \
                         before you launch its subagents",
                        round.n,
                        task.0,
                        dovetail_api::research::ordinal(position),
                        round.n,
                        task.0
                    ),
                    Err(e) => format!(
                        "research_propose accepted \"{shown}\" as a new round, but it could not \
                         open: {e}"
                    ),
                }
            }
            (_, ProposalKind::Idea) => self.accept_idea(&agent, task, &text, why, &evidence).await,
            (_, ProposalKind::Decision) => {
                let new = NewDecision {
                    task,
                    text,
                    why,
                    alternatives,
                    supersedes: None,
                };
                let by = DecisionAuthor::Agent(agent.clone());
                match self.db.log_decision_with_evidence(new, by, evidence).await {
                    Ok(logged) => {
                        let text = format!(
                            "research_propose accepted: logged decision D{} on task #{} with its \
                             evidence",
                            logged.id.0, task.0
                        );
                        self.emit(Event::Decision(DecisionEvent::Added(logged)));
                        text
                    }
                    Err(e) => format!(
                        "research_propose accepted \"{shown}\", but it could not be logged: {e}"
                    ),
                }
            }
        };
        self.tell_coordinator(message);
    }

    /// An accepted idea: a follow-up task in Unsorted (ADR 0015's routing rule for a different
    /// question). Resolves to what the coordinator is told.
    async fn accept_idea(
        &mut self,
        _agent: &AgentId,
        task: TaskId,
        text: &str,
        why: Option<String>,
        evidence: &[Evidence],
    ) -> String {
        let title: String = first_line(text).chars().take(TITLE_MAX).collect();
        let mut notes = text.to_owned();
        if let Some(why) = why {
            notes.push_str(&format!("\n\nWhy: {why}"));
        }
        if !evidence.is_empty() {
            let ids: Vec<String> = evidence.iter().map(ToString::to_string).collect();
            notes.push_str(&format!("\n\nEvidence: {}", ids.join(", ")));
        }
        let new = NewTask {
            title,
            p_level: None,
            bucket: None,
            notes,
            follow_up_of: Some(task),
            pr: None,
            related: Vec::new(),
            effort: Default::default(),
            criterion: None,
        };
        // The user accepted it, so it is not marked as coordinator-created.
        match self.create_task(new, false).await {
            Ok(id) => format!(
                "research_propose accepted: created task #{} in Unsorted, a follow-up of #{}",
                id.0, task.0
            ),
            Err(e) => format!("research_propose accepted, but the task could not be created: {e}"),
        }
    }
}

/// The first non-blank line, trimmed.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

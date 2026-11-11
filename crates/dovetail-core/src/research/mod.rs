//! Research capture (R1, `docs/research.md` §3–§5, ADR 0015): rounds, findings and sources,
//! taken from the stream with nothing sent to any session.
//!
//! A round is opened by the user (`Command::StartRound`; R2 adds the coordinator's
//! `research_start_round`). While it is open, every subagent the coordinator launches is
//! watched ([`capture`]) and filed as a finding of the round when it ends, with the sources its
//! tool calls reached. A research-shaped subagent (`WebFetch`/`WebSearch`) of a session that
//! works on a task, launched with no round for it, becomes a quick finding (§5.7). Rows go to
//! `state.db` first; the files ([`files`]) follow and are rebuilt at start when missing.
//!
//! Context isolation (§5.6): nothing here sends input to a session or touches its prompt.

pub(crate) mod capture;
pub(crate) mod files;
pub(crate) mod report;

use std::path::PathBuf;

use dovetail_api::research::{
    FindingSummary, ResearchContent, ResearchDocument, ResearchKind, ResearchListing,
    ResearchSummary, ResearchWhat, RoundSummary, SourceSummary, ordinal,
};
use dovetail_api::{ActivityKind, AgentId, Event, InboxKind, Task, TaskId};
use dovetail_driver::{ContentBlock, DriverEvent};

use crate::db::research::{FindingRow, RoundRow, SourceRow, SummaryRow, TaskResearch};
use crate::db::{DbError, now_ms};
use crate::inbox::Waiter;
use crate::service::{COORDINATOR, Core};
use capture::{Capture, Outcome};
use files::ResearchDirError;

/// Why research content could not be loaded.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoadError {
    /// No such task, round, finding, source or report.
    #[error("{0}")]
    Missing(String),
    /// The research folder is unknown.
    #[error(transparent)]
    Dir(#[from] ResearchDirError),
    /// Storage failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// Why a round could not be opened or closed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RoundError {
    /// No such task.
    #[error("there is no task #{}", .0.0)]
    NoTask(TaskId),
    /// The research folder is unknown.
    #[error(transparent)]
    Dir(#[from] ResearchDirError),
    /// Storage refused or failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// What the core keeps about research between events.
#[derive(Debug, Default)]
pub(crate) struct Research {
    /// Subagents being watched.
    pub(crate) capture: Capture,
    /// The round open now, if any (also in `state.db`, which wins at start).
    pub(crate) open: Option<RoundRow>,
}

impl Core {
    /// The project's research root, from its recipe.
    pub(crate) fn research_root(&self) -> Result<PathBuf, ResearchDirError> {
        let project = self
            .project
            .registered()
            .ok_or(ResearchDirError::NotRegistered)?;
        root_of(&self.paths, project)
    }

    /// The research folder of `task` (absolute), given its title.
    pub(crate) fn research_dir(
        &self,
        task: TaskId,
        title: &str,
    ) -> Result<PathBuf, ResearchDirError> {
        Ok(files::task_dir(&self.research_root()?, task, title))
    }

    /// Reads the open round at start.
    pub(crate) async fn load_research(&mut self) -> Result<(), DbError> {
        self.research.open = self.db.open_round().await?;
        Ok(())
    }

    /// Every task's research summary, for the snapshot; empty for a folder that is not a
    /// project.
    pub(crate) async fn research_summaries(
        &self,
        tasks: &[Task],
    ) -> Result<Vec<ResearchSummary>, DbError> {
        let Ok(root) = self.research_root() else {
            return Ok(Vec::new());
        };
        let rows = self.db.research_summaries().await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let title = tasks
                    .iter()
                    .find(|t| t.id == row.task)
                    .map_or("task", |t| t.title.as_str());
                let dir = files::task_dir(&root, row.task, title);
                summary(row, dir)
            })
            .collect())
    }

    /// Sends the task's summary as it is now.
    pub(crate) async fn emit_research(&mut self, task: TaskId, title: &str) {
        let Ok(dir) = self.research_dir(task, title) else {
            return;
        };
        match self.db.research_summary(task).await {
            Ok(row) => self.emit(Event::ResearchUpserted(summary(row, dir))),
            Err(e) => tracing::warn!("loading research of task #{}: {e}", task.0),
        }
    }

    /// `Command::StartRound`: the row, `brief.md`, and the round open for the coordinator, or
    /// queued behind the open one.
    pub(crate) async fn start_round(&mut self, task: TaskId, brief: String) {
        if let Err(message) = self.open_round_for(task, brief).await {
            self.fail(message);
        }
    }

    /// Requests the next round of `task`: the one path under `Command::StartRound`,
    /// `research_start_round` and an accepted follow-up round. The round opens at once, or is
    /// queued while another is open (`brief.md` is written either way). Resolves to the round
    /// and, when it is queued, its place in line (the open round is 1st), or what to tell
    /// whoever asked.
    pub(crate) async fn open_round_for(
        &mut self,
        task: TaskId,
        brief: String,
    ) -> Result<(RoundRow, Option<usize>), RoundError> {
        let t = self.task(task).await.ok_or(RoundError::NoTask(task))?;
        let dir = self.research_dir(task, &t.title)?;
        let round = self.db.start_round(task, brief).await?;
        let path = dir.join(files::brief_path(&round));
        if let Err(e) = files::write_atomic(&path, &files::brief_md(task, &t.title, &round)) {
            self.fail(format!("research: writing {}: {e}", path.display()));
        }
        let position = if round.queued {
            let queued = self.db.queued_rounds().await?;
            let position = queued
                .iter()
                .position(|q| q.id == round.id)
                .map_or(2, |i| i + 2);
            self.activity(
                ActivityKind::Task,
                None,
                format!(
                    "research round {} queued on #{} {} ({} in line)",
                    round.n,
                    task.0,
                    t.title,
                    ordinal(position)
                ),
            );
            Some(position)
        } else {
            self.activity(
                ActivityKind::Task,
                None,
                format!(
                    "research round {} opened on #{} {}",
                    round.n, task.0, t.title
                ),
            );
            self.research.open = Some(round.clone());
            None
        };
        self.emit_research(task, &t.title).await;
        Ok((round, position))
    }

    /// `Command::CloseRound`. Subagents already launched in it are still filed under it.
    pub(crate) async fn close_round(&mut self, task: TaskId) {
        if let Err(message) = self.close_round_for(task).await {
            self.fail(message);
        }
    }

    /// Closes the open round of `task` (also `research_write_report`'s last step). The next
    /// queued round, in request order, opens and the coordinator is told to send its subagents.
    pub(crate) async fn close_round_for(&mut self, task: TaskId) -> Result<RoundRow, RoundError> {
        let (round, next) = self.db.close_round(task).await?;
        self.research.open = None;
        let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
        self.activity(
            ActivityKind::Task,
            None,
            format!("research round {} closed on #{} {title}", round.n, task.0),
        );
        self.emit_research(task, &title).await;
        if let Some(next) = next {
            self.round_from_queue(next).await;
        }
        self.emit_queued_tasks().await;
        Ok(round)
    }

    /// A queued round has opened: it is the open one now, and the coordinator waits for this
    /// message before it fans out.
    async fn round_from_queue(&mut self, round: RoundRow) {
        let title = self
            .task(round.task)
            .await
            .map(|t| t.title)
            .unwrap_or_default();
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "research round {} opened on #{} {title} (from the queue)",
                round.n, round.task.0
            ),
        );
        let (task, n) = (round.task, round.n);
        self.research.open = Some(round);
        self.emit_research(task, &title).await;
        self.tell_coordinator(format!(
            "Round {n} for #{} is now open; send its subagents.",
            task.0
        ));
    }

    /// Sends the summary of every task that still has a queued round, so each place in line is
    /// current.
    async fn emit_queued_tasks(&mut self) {
        let Ok(queued) = self.db.queued_rounds().await else {
            return;
        };
        let mut tasks: Vec<TaskId> = Vec::new();
        for q in queued {
            if !tasks.contains(&q.task) {
                tasks.push(q.task);
            }
        }
        for task in tasks {
            let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
            self.emit_research(task, &title).await;
        }
    }

    /// `Command::DropRound`: a queued round, its row and its `brief.md`.
    pub(crate) async fn drop_round(&mut self, task: TaskId, n: Option<u32>) {
        let round = match self.db.drop_round(task, n).await {
            Ok(round) => round,
            Err(e) => {
                self.fail(e.to_string());
                return;
            }
        };
        let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
        self.remove_round_files(task, &title, &round);
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "research round {} dropped from the queue on #{} {title}",
                round.n, task.0
            ),
        );
        self.emit_research(task, &title).await;
        self.emit_queued_tasks().await;
    }

    /// Removes a dropped round's `brief.md` (and its folder when nothing else is in it).
    fn remove_round_files(&self, task: TaskId, title: &str, round: &RoundRow) {
        let Ok(dir) = self.research_dir(task, title) else {
            return;
        };
        let _ = std::fs::remove_file(dir.join(files::brief_path(round)));
        let _ = std::fs::remove_dir(dir.join(&round.dir));
    }

    /// Archiving `task`: its queued rounds are dropped and its open round closed (findings kept),
    /// each noted in Activity. The next queued round, of another task, then opens.
    pub(crate) async fn release_research_for_archive(&mut self, task: TaskId) {
        let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
        match self.db.drop_queued_for(task).await {
            Ok(dropped) => {
                for round in dropped {
                    self.remove_round_files(task, &title, &round);
                    self.activity(
                        ActivityKind::Task,
                        None,
                        format!(
                            "research round {} dropped from the queue: #{} {title} is archived",
                            round.n, task.0
                        ),
                    );
                }
            }
            Err(e) => tracing::warn!("dropping the queued rounds of #{}: {e}", task.0),
        }
        if self.research.open.as_ref().is_some_and(|r| r.task == task) {
            match self.close_round_for(task).await {
                Ok(round) => self.activity(
                    ActivityKind::Task,
                    None,
                    format!(
                        "research round {} closed because #{} {title} was archived; its \
                         findings are kept",
                        round.n, task.0
                    ),
                ),
                Err(e) => tracing::warn!("closing the round of #{}: {e}", task.0),
            }
        } else {
            self.emit_research(task, &title).await;
            self.emit_queued_tasks().await;
        }
    }

    /// Words for the research round that archiving `task` would close or drop ("an open research
    /// round", "a queued research round"), or `None` when it has none.
    pub(crate) async fn round_blocking_archive(&self, task: TaskId) -> Option<&'static str> {
        if self.research.open.as_ref().is_some_and(|r| r.task == task) {
            return Some("an open research round");
        }
        let queued = self.db.queued_rounds().await.ok()?;
        queued
            .iter()
            .any(|r| r.task == task)
            .then_some("a queued research round")
    }

    /// `Command::SetResearchKind`.
    pub(crate) async fn set_research_kind(&mut self, task: TaskId, kind: ResearchKind) {
        let Some(t) = self.task(task).await else {
            self.fail(format!("there is no task #{}", task.0));
            return;
        };
        if let Err(e) = self
            .db
            .set_research_kind(task, Some(kind.as_str().into()))
            .await
        {
            self.fail(e.to_string());
            return;
        }
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "research kind of #{} {} set to {}",
                task.0,
                t.title,
                kind.as_str()
            ),
        );
        self.emit_research(task, &t.title).await;
    }

    /// `Command::LoadResearch`.
    pub(crate) async fn load_research_content(&mut self, task: TaskId, what: ResearchWhat) {
        match self.research_content(task, &what).await {
            Ok(content) => self.emit(Event::ResearchLoaded {
                task,
                what,
                content,
            }),
            Err(message) => self.fail(message),
        }
    }

    pub(crate) async fn research_content(
        &self,
        task: TaskId,
        what: &ResearchWhat,
    ) -> Result<ResearchContent, LoadError> {
        let t = self
            .task(task)
            .await
            .ok_or_else(|| LoadError::Missing(format!("there is no task #{}", task.0)))?;
        let dir = self.research_dir(task, &t.title)?;
        let r = self.db.task_research(task).await?;
        content_of(&dir, task, &t.title, &r, what)
    }

    /// Writes research files that are missing (the database is the source of truth), on a
    /// blocking thread after the first snapshot.
    pub(crate) async fn rebuild_research_files(&mut self) {
        let Ok(root) = self.research_root() else {
            return;
        };
        if let Ok(unfiled) = self.db.all_unfiled().await {
            let dir = files::unfiled_dir(&root);
            for (row, sources) in unfiled {
                let missing = !dir.join(&row.path).exists();
                if missing {
                    self.write_finding_files(&dir, &row, &sources);
                }
            }
        }
        let (all, tasks) = match (self.db.all_research().await, self.db.load_tasks().await) {
            (Ok(all), Ok(tasks)) => (all, tasks),
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!("loading research to rebuild its files: {e}");
                return;
            }
        };
        if all.is_empty() {
            return;
        }
        let jobs: Vec<(TaskId, String, TaskResearch)> = all
            .into_iter()
            .map(|(task, r)| {
                let title = tasks
                    .iter()
                    .find(|t| t.id == task)
                    .map_or_else(|| "task".to_owned(), |t| t.title.clone());
                (task, title, r)
            })
            .collect();
        let rebuilt = tokio::task::spawn_blocking(move || {
            let mut n = 0;
            for (task, title, r) in jobs {
                let dir = files::task_dir(&root, task, &title);
                match files::rebuild(&dir, task, &title, &r) {
                    Ok(k) => n += k,
                    Err(e) => tracing::warn!("rebuilding research files in {}: {e}", dir.display()),
                }
            }
            n
        })
        .await
        .unwrap_or(0);
        if rebuilt > 0 {
            tracing::info!("rebuilt {rebuilt} missing research files");
        }
    }

    /// Watches the stream for research (called for every event of a session, before routing).
    pub(crate) async fn research_event(&mut self, agent: &AgentId, event: &DriverEvent) {
        // A tangent files into Tangents, whatever the open folder is (ADR 0017).
        let tangent = self.tangent_of(agent);
        if self.project.registered().is_none() && tangent.is_none() {
            return;
        }
        if let Some(origin) = event.origin().filter(|o| !o.is_main()) {
            if let Some(key) = origin.parent_tool_use_id.as_deref() {
                self.research.capture.subagent_event(key, event);
            }
            return;
        }
        if let Some(task) = tangent {
            self.tangent_turn_event(agent, task, event).await;
        }
        let outcomes = match event {
            DriverEvent::Assistant(message) => {
                for block in &message.blocks {
                    if let ContentBlock::ToolUse { id, name, input } = block
                        && crate::session::subagent::launches_subagent(name)
                    {
                        self.research_launch(agent, id, input);
                    }
                }
                Vec::new()
            }
            DriverEvent::Task(task) => self.research.capture.task_event(task),
            DriverEvent::ToolResult(result)
                if self.research.capture.watches(&result.tool_use_id) =>
            {
                self.research.capture.agent_result(result)
            }
            DriverEvent::Exited(_) => self.research.capture.parent_exited(agent),
            _ => Vec::new(),
        };
        for outcome in outcomes {
            self.research_outcome(outcome).await;
        }
    }

    /// The parent `agent`'s `Agent` call `key`: watched when a round is open for the coordinator,
    /// when the parent works on a task (a quick finding if it turns out research-shaped), or when
    /// it is the coordinator with no round open (an unfiled finding if research-shaped).
    fn research_launch(&mut self, agent: &AgentId, key: &str, input: &serde_json::Value) {
        // A tangent's research subagents become quick findings of its task in Tangents.
        if let Some(task) = self.tangent_of(agent) {
            let sub = crate::session::subagent_id(agent, key);
            self.research
                .capture
                .launched(agent, sub, key, input, Some(task), None, now_ms());
            self.research.capture.mark_tangent(key);
            return;
        }
        let (task, round) = match (&self.research.open, agent.0 == COORDINATOR) {
            (Some(open), true) => (Some(open.task), Some(open.id)),
            (None, true) => (None, None),
            _ => match self.agents.get(agent).and_then(|r| r.agent.task) {
                Some(task) => (Some(task), None),
                None => return,
            },
        };
        let sub = crate::session::subagent_id(agent, key);
        self.research
            .capture
            .launched(agent, sub, key, input, task, round, now_ms());
    }

    pub(crate) async fn research_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::File {
                key,
                from,
                finding,
                tangent: true,
            } => self.file_tangent_finding(key, from, *finding).await,
            Outcome::Upgrade {
                finding,
                task: Some(task),
                body,
                from,
                tangent: true,
            } => {
                self.upgrade_tangent_finding(task, finding, body, from)
                    .await
            }
            Outcome::File {
                key, from, finding, ..
            } => {
                let Some(task) = finding.task else {
                    self.file_unfiled_outcome(key, from, *finding).await;
                    return;
                };
                let agent = finding.agent.clone();
                let (row, sources) = match self.db.file_finding(*finding).await {
                    Ok(filed) => filed,
                    Err(e) => {
                        tracing::warn!("filing a research finding: {e}");
                        self.fail(format!("research: could not file a finding: {e}"));
                        return;
                    }
                };
                self.research.capture.filed(&key, row.id, from, &row.body);
                let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
                match self.research_dir(task, &title) {
                    Ok(dir) => {
                        let mut problems = Vec::new();
                        for s in &sources {
                            let Some(rel) = files::source_path(s) else {
                                continue;
                            };
                            let path = dir.join(rel);
                            if !path.exists()
                                && let Err(e) = files::write_atomic(&path, &files::source_md(s))
                            {
                                problems.push(format!("{}: {e}", path.display()));
                            }
                        }
                        let path = dir.join(&row.path);
                        if let Err(e) =
                            files::write_atomic(&path, &files::finding_md(&row, &sources))
                        {
                            problems.push(format!("{}: {e}", path.display()));
                        }
                        if !problems.is_empty() {
                            self.fail(format!("research: writing {}", problems.join("; ")));
                        }
                    }
                    Err(e) => self.fail(e.to_string()),
                }
                let round = row
                    .round
                    .map_or_else(|| "quick finding".to_owned(), |n| format!("round {n}"));
                self.activity(
                    ActivityKind::Task,
                    Some(&agent),
                    format!(
                        "research #{} {round}: filed F{} {} ({} sources)",
                        task.0,
                        row.id.0,
                        row.label,
                        row.sources.len()
                    ),
                );
                self.emit_research(task, &title).await;
            }
            Outcome::Upgrade {
                finding,
                task: None,
                body,
                from,
                ..
            } => {
                match self
                    .db
                    .set_unfiled_body(finding.0, body, from.as_str())
                    .await
                {
                    Ok(true) => self.rewrite_unfiled(finding.0).await,
                    Ok(false) => {}
                    Err(e) => tracing::warn!("updating unfiled finding {}: {e}", finding.0),
                }
            }
            Outcome::Upgrade {
                finding,
                task: Some(task),
                body,
                from,
                ..
            } => {
                if let Err(e) = self.db.set_finding_body(finding, body, from.as_str()).await {
                    tracing::warn!("updating finding F{}: {e}", finding.0);
                    return;
                }
                self.rewrite_finding(task, finding).await;
            }
        }
    }

    /// The folder of findings with no task, under the research root.
    fn unfiled_dir(&self) -> Result<PathBuf, ResearchDirError> {
        Ok(files::unfiled_dir(&self.research_root()?))
    }

    /// A coordinator subagent with no round open fetched something: store its report with no task,
    /// and ask in Needs you where it goes.
    async fn file_unfiled_outcome(
        &mut self,
        key: String,
        from: capture::BodyFrom,
        finding: crate::db::research::NewFinding,
    ) {
        let (row, sources) = match self.db.file_unfiled(finding).await {
            Ok(filed) => filed,
            Err(e) => {
                tracing::warn!("filing an unfiled finding: {e}");
                self.fail(format!("research: could not store a finding: {e}"));
                return;
            }
        };
        self.research.capture.filed(&key, row.id, from, &row.body);
        match self.unfiled_dir() {
            Ok(dir) => self.write_finding_files(&dir, &row, &sources),
            Err(e) => self.fail(e.to_string()),
        }
        let coordinator = AgentId(COORDINATOR.into());
        let count = row.sources.len();
        self.activity(
            ActivityKind::Task,
            Some(&row.agent),
            format!(
                "research (no round open): unfiled finding {} ({count} sources)",
                row.label
            ),
        );
        self.raise(
            &coordinator,
            InboxKind::UnfiledFinding {
                finding: row.id.0,
                label: row.label.clone(),
                sources: u32::try_from(count).unwrap_or(u32::MAX),
            },
            Waiter::UnfiledFinding { finding: row.id.0 },
        );
    }

    /// Writes a finding's file and its sources' cached copies (the ones not there yet) under
    /// `dir`, telling the user about any that fail.
    pub(crate) fn write_finding_files(
        &mut self,
        dir: &std::path::Path,
        row: &FindingRow,
        sources: &[SourceRow],
    ) {
        let mut problems = Vec::new();
        for s in sources {
            let Some(rel) = files::source_path(s) else {
                continue;
            };
            let path = dir.join(rel);
            if !path.exists()
                && let Err(e) = files::write_atomic(&path, &files::source_md(s))
            {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
        let path = dir.join(&row.path);
        if let Err(e) = files::write_atomic(&path, &files::finding_md(row, sources)) {
            problems.push(format!("{}: {e}", path.display()));
        }
        if !problems.is_empty() {
            self.fail(format!("research: writing {}", problems.join("; ")));
        }
    }

    /// Writes an unfiled finding's file again from its row.
    async fn rewrite_unfiled(&mut self, id: i64) {
        let (Ok(dir), Ok(Some((row, sources)))) =
            (self.unfiled_dir(), self.db.unfiled_finding(id).await)
        else {
            return;
        };
        self.write_finding_files(&dir, &row, &sources);
    }

    /// Deletes the files of an unfiled finding that left the database (filed or dropped): its
    /// own, and the cached sources no other unfiled finding cites.
    fn remove_unfiled_files(&self, removed: &crate::db::research::Removed) {
        let Ok(dir) = self.unfiled_dir() else {
            return;
        };
        let _ = std::fs::remove_file(dir.join(&removed.finding.path));
        for s in &removed.orphans {
            if let Some(rel) = files::source_path(s) {
                let _ = std::fs::remove_file(dir.join(rel));
            }
        }
        // Folders that are empty now go too, so a dropped finding leaves nothing behind.
        for sub in ["findings", "sources", ""] {
            let _ = std::fs::remove_dir(dir.join(sub));
        }
    }

    /// `d` on an unfiled finding (or `x`): deleted, files removed.
    pub(crate) async fn drop_unfiled_finding(&mut self, finding: i64) {
        match self.db.drop_unfiled(finding).await {
            Ok(Some(removed)) => {
                self.remove_unfiled_files(&removed);
                self.activity(
                    ActivityKind::Task,
                    None,
                    format!(
                        "research: unfiled finding {} dropped",
                        removed.finding.label
                    ),
                );
            }
            Ok(None) => {}
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// `Command::FileUnfiled`: the finding becomes a quick finding of `task`, its files move to
    /// the task's research folder, and the item leaves Needs you.
    pub(crate) async fn file_unfiled(&mut self, item: dovetail_api::InboxId, task: TaskId) {
        let finding = match self.inbox.get(&item).map(|e| &e.waiter) {
            Some(Waiter::UnfiledFinding { finding }) => *finding,
            _ => {
                self.fail("that item is not an unfiled finding");
                return;
            }
        };
        let Some(t) = self.task(task).await else {
            self.fail(format!("there is no task #{}", task.0));
            return;
        };
        let (row, sources, removed) = match self.db.file_unfiled_to_task(finding, task).await {
            Ok(filed) => filed,
            Err(e) => {
                self.fail(e.to_string());
                return;
            }
        };
        self.resolve(item, Some(format!("filed under #{}", task.0)));
        match self.research_dir(task, &t.title) {
            Ok(dir) => self.write_finding_files(&dir, &row, &sources),
            Err(e) => self.fail(e.to_string()),
        }
        self.remove_unfiled_files(&removed);
        self.activity(
            ActivityKind::Task,
            Some(&row.agent),
            format!(
                "research #{} quick finding: filed F{} {} from the unfiled ({} sources)",
                task.0,
                row.id.0,
                row.label,
                row.sources.len()
            ),
        );
        self.emit_research(task, &t.title).await;
    }

    /// Writes a finding's file again from its row.
    async fn rewrite_finding(&mut self, task: TaskId, id: dovetail_api::research::FindingId) {
        let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
        let (Ok(dir), Ok(r)) = (
            self.research_dir(task, &title),
            self.db.task_research(task).await,
        ) else {
            return;
        };
        if let Some(f) = r.findings.iter().find(|f| f.id == id) {
            let path = dir.join(&f.path);
            if let Err(e) = files::write_atomic(&path, &files::finding_md(f, &r.sources)) {
                self.fail(format!("research: writing {}: {e}", path.display()));
            }
        }
    }
}

/// The API's summary of one task.
pub(crate) fn summary(row: SummaryRow, dir: PathBuf) -> ResearchSummary {
    ResearchSummary {
        task: row.task,
        dir,
        rounds: row
            .rounds
            .into_iter()
            .map(|(r, findings)| RoundSummary {
                id: r.id,
                queued: r.queued,
                n: r.n,
                brief: r.brief,
                dir: PathBuf::from(r.dir),
                opened_ms: r.opened_ms,
                closed_ms: r.closed_ms,
                findings,
            })
            .collect(),
        findings: row.findings,
        quick_findings: row.quick_findings,
        sources: row.sources,
        report_ms: row.report_ms,
        kind: row.kind.as_deref().and_then(ResearchKind::parse),
    }
}

/// The API's listing of one task, with absolute paths.
pub(crate) fn listing(dir: &std::path::Path, r: &TaskResearch) -> ResearchListing {
    ResearchListing {
        findings: r
            .findings
            .iter()
            .map(|f| FindingSummary {
                id: f.id,
                round: f.round,
                n: f.n,
                agent: f.agent.clone(),
                label: f.label.clone(),
                path: dir.join(&f.path),
                created_ms: f.created_ms,
                sources: f.sources.clone(),
            })
            .collect(),
        sources: r
            .sources
            .iter()
            .map(|s| SourceSummary {
                id: s.id,
                kind: s.kind,
                locator: s.locator.clone(),
                tool: s.tool.clone(),
                path: files::source_path(s).map(|p| dir.join(p)),
                is_error: s.is_error,
                fetched_ms: s.fetched_ms,
            })
            .collect(),
    }
}

/// The research root of any registered project, from its recipe: what [`Core::research_root`]
/// is for the open one. Filing a tangent into another project writes there.
pub(crate) fn root_of(
    paths: &crate::paths::Paths,
    project: &crate::projects::Project,
) -> Result<PathBuf, ResearchDirError> {
    // A research project has no repo for a research folder to sit in: always the default.
    let Some(repo) = project
        .repo_path
        .as_deref()
        .filter(|_| !project.is_research())
    else {
        return files::root(&paths.data, &project.id, std::path::Path::new(""), None);
    };
    let recipe = crate::recipe::Recipe::load(&paths.recipe(&project.id))?;
    files::root(
        &paths.data,
        &project.id,
        repo,
        recipe.research_dir.as_deref(),
    )
}

/// Research content of `task` (titled `title`), whose folder is `dir` and whose rows are `r`:
/// the open project's tasks and Tangents' tangents alike.
pub(crate) fn content_of(
    dir: &std::path::Path,
    task: TaskId,
    title: &str,
    r: &TaskResearch,
    what: &ResearchWhat,
) -> Result<ResearchContent, LoadError> {
    let missing = |what: String| LoadError::Missing(format!("task #{} {what}", task.0));
    let doc = |path: Option<String>, markdown: String| {
        let path = path.map(|p| dir.join(p));
        let base = path
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(dir);
        let images = crate::docs::images(&markdown, base);
        ResearchContent::Document(ResearchDocument {
            path,
            markdown,
            images,
        })
    };
    Ok(match what {
        ResearchWhat::Listing => ResearchContent::Listing(listing(dir, r)),
        ResearchWhat::Brief { round } => {
            let round = r
                .rounds
                .iter()
                .find(|x| x.n == *round)
                .ok_or_else(|| missing(format!("has no round {round}")))?;
            doc(
                Some(files::brief_path(round)),
                files::brief_md(task, title, round),
            )
        }
        ResearchWhat::Finding(id) => {
            let f = r
                .findings
                .iter()
                .find(|f| f.id == *id)
                .ok_or_else(|| missing(format!("has no finding F{}", id.0)))?;
            doc(Some(f.path.clone()), files::finding_md(f, &r.sources))
        }
        ResearchWhat::Source(id) => {
            let s = r
                .sources
                .iter()
                .find(|s| s.id == *id)
                .ok_or_else(|| missing(format!("cites no source S{}", id.0)))?;
            doc(files::source_path(s), files::source_md(s))
        }
        ResearchWhat::Report => {
            let (_, body) = r
                .report
                .clone()
                .ok_or_else(|| missing("has no report yet".into()))?;
            doc(Some("report.md".into()), body)
        }
    })
}

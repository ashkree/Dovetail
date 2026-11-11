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
    FindingSummary, ResearchContent, ResearchDocument, ResearchListing, ResearchSummary,
    ResearchWhat, RoundSummary, SourceSummary,
};
use dovetail_api::{ActivityKind, AgentId, Event, Task, TaskId};
use dovetail_driver::{ContentBlock, DriverEvent};

use crate::db::research::{RoundRow, SummaryRow, TaskResearch};
use crate::db::{DbError, now_ms};
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
        // A research project has no repo for a research folder to sit in: always the default.
        let Some(repo) = project.repo_path.as_deref().filter(|_| !project.is_research()) else {
            return files::root(&self.paths.data, &project.id, std::path::Path::new(""), None);
        };
        let recipe = crate::recipe::Recipe::load(&self.paths.recipe(&project.id))?;
        files::root(
            &self.paths.data,
            &project.id,
            repo,
            recipe.research_dir.as_deref(),
        )
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

    /// `Command::StartRound`: the row, `brief.md`, and the round open for the coordinator.
    pub(crate) async fn start_round(&mut self, task: TaskId, brief: String) {
        if let Err(message) = self.open_round_for(task, brief).await {
            self.fail(message);
        }
    }

    /// Opens the next round of `task`: the one path under `Command::StartRound`,
    /// `research_start_round` and an accepted follow-up round. Resolves to the round, or what to
    /// tell whoever asked.
    pub(crate) async fn open_round_for(
        &mut self,
        task: TaskId,
        brief: String,
    ) -> Result<RoundRow, RoundError> {
        let t = self.task(task).await.ok_or(RoundError::NoTask(task))?;
        let dir = self.research_dir(task, &t.title)?;
        let round = self.db.start_round(task, brief).await?;
        let path = dir.join(files::brief_path(&round));
        if let Err(e) = files::write_atomic(&path, &files::brief_md(task, &t.title, &round)) {
            self.fail(format!("research: writing {}: {e}", path.display()));
        }
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "research round {} opened on #{} {}",
                round.n, task.0, t.title
            ),
        );
        self.research.open = Some(round.clone());
        self.emit_research(task, &t.title).await;
        Ok(round)
    }

    /// `Command::CloseRound`. Subagents already launched in it are still filed under it.
    pub(crate) async fn close_round(&mut self, task: TaskId) {
        if let Err(message) = self.close_round_for(task).await {
            self.fail(message);
        }
    }

    /// Closes the open round of `task` (also `research_write_report`'s last step).
    pub(crate) async fn close_round_for(&mut self, task: TaskId) -> Result<RoundRow, RoundError> {
        let round = self.db.close_round(task).await?;
        self.research.open = None;
        let title = self.task(task).await.map(|t| t.title).unwrap_or_default();
        self.activity(
            ActivityKind::Task,
            None,
            format!("research round {} closed on #{} {title}", round.n, task.0),
        );
        self.emit_research(task, &title).await;
        Ok(round)
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
        let missing = |what: String| LoadError::Missing(format!("task #{} {what}", task.0));
        let t = self
            .task(task)
            .await
            .ok_or_else(|| LoadError::Missing(format!("there is no task #{}", task.0)))?;
        let dir = self.research_dir(task, &t.title)?;
        let r = self.db.task_research(task).await?;
        let doc = |path: Option<String>, markdown: String| {
            ResearchContent::Document(ResearchDocument {
                path: path.map(|p| dir.join(p)),
                markdown,
            })
        };
        Ok(match what {
            ResearchWhat::Listing => ResearchContent::Listing(listing(&dir, &r)),
            ResearchWhat::Brief { round } => {
                let round = r
                    .rounds
                    .iter()
                    .find(|x| x.n == *round)
                    .ok_or_else(|| missing(format!("has no round {round}")))?;
                doc(
                    Some(files::brief_path(round)),
                    files::brief_md(task, &t.title, round),
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

    /// Writes research files that are missing (the database is the source of truth), on a
    /// blocking thread after the first snapshot.
    pub(crate) async fn rebuild_research_files(&mut self) {
        let Ok(root) = self.research_root() else {
            return;
        };
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
        if self.project.registered().is_none() {
            return;
        }
        if let Some(origin) = event.origin().filter(|o| !o.is_main()) {
            if let Some(key) = origin.parent_tool_use_id.as_deref() {
                self.research.capture.subagent_event(key, event);
            }
            return;
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
    /// or when the parent works on a task (a quick finding if it turns out research-shaped).
    fn research_launch(&mut self, agent: &AgentId, key: &str, input: &serde_json::Value) {
        let (task, round) = match (&self.research.open, agent.0 == COORDINATOR) {
            (Some(open), true) => (open.task, Some(open.id)),
            _ => match self.agents.get(agent).and_then(|r| r.agent.task) {
                Some(task) => (task, None),
                None => return,
            },
        };
        let sub = crate::session::subagent_id(agent, key);
        self.research
            .capture
            .launched(agent, sub, key, input, task, round, now_ms());
    }

    async fn research_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::File { key, from, finding } => {
                let task = finding.task;
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
                task,
                body,
                from,
            } => {
                if let Err(e) = self.db.set_finding_body(finding, body, from.as_str()).await {
                    tracing::warn!("updating finding F{}: {e}", finding.0);
                    return;
                }
                self.rewrite_finding(task, finding).await;
            }
        }
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
fn summary(row: SummaryRow, dir: PathBuf) -> ResearchSummary {
    ResearchSummary {
        task: row.task,
        dir,
        rounds: row
            .rounds
            .into_iter()
            .map(|(r, findings)| RoundSummary {
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
    }
}

/// The API's listing of one task, with absolute paths.
fn listing(dir: &std::path::Path, r: &TaskResearch) -> ResearchListing {
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

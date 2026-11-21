//! Tangents (ADR 0017, amended 2026-11-11): questions that do not belong to the open project.
//!
//! Tangents is a built-in research project, `tangents`, registered at every start when missing
//! ([`open`]). A tangent is a task in its Unsorted list. Asking one ([`Core::start_tangent`])
//! saves that task in Tangents' `state.db` and starts a read-only session for it in Tangents'
//! folder, run by whatever project is open; its id names the task ([`tangent_agent`]). Each
//! turn's answer, with the sources the session's own tool calls reached, is filed on the task as
//! a finding, and so are its research subagents, through the R1 capture pipeline
//! (`research::capture`, `tangent` candidates). Nothing goes to the open project unless the user
//! sends it there: attach to a task ([`Core::attach_tangent`]), file into a project, start a
//! task from it, or drop it (the triage of the morning's "new tangents" item).
//!
//! Tangents' rows are written through a second [`Db`] handle on its `state.db` (the open
//! project's own when Tangents is the open project). A tangent never deletes anything: one that
//! leaves Tangents is archived there, with a note saying where it went.

use std::path::PathBuf;

use dovetail_api::{
    Event,
    activity::ActivityKind,
    agent::{AgentId, AgentKind, PermissionMode},
    conversation::EntryKind,
    inbox::InboxKind,
    project::ProjectId,
    tangent::{TANGENTS_PROJECT, TangentSummary, tangent_agent, tangent_task},
    task::{NewTask, Task, TaskChange, TaskId, TaskStatus},
};
use dovetail_api::{
    inbox::InboxEvent,
    research::{FindingId, ResearchWhat},
    tangent::TangentEvent,
    task::TaskEvent,
};
use dovetail_driver::DriverEvent;

use crate::db::research::{FindingRow, NewFinding, NewSource, SourceRow, TaskResearch};
use crate::db::{AgentRow, Db, DbError, now_ms};
use crate::inbox::Waiter;
use crate::paths::Paths;
use crate::registration::ProjectState;
use crate::research::capture::{self, BodyFrom};
use crate::research::files;
use crate::service::{COORDINATOR, Core, new_agent};

/// Longest task title a question gives; the whole question is kept in the notes.
const TITLE_MAX: usize = 80;

/// The file in a project's state folder holding the day its morning last looked for new
/// tangents, so the "new tangents" item comes once a day.
const MORNING_MARKER: &str = "tangents-morning";

/// Why a tangent command could not be carried out.
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum TangentError {
    /// Tangents could not be opened at start: no tangent can be asked or moved.
    #[error("tangents are off: {0}")]
    Off(String),
    /// No such tangent, or not one that can do this now.
    #[error("{0}")]
    Refused(String),
    /// Storage or a file failed.
    #[error("{0}")]
    Failed(String),
}

impl From<DbError> for TangentError {
    fn from(e: DbError) -> Self {
        TangentError::Failed(e.to_string())
    }
}

/// A refusal worded for the user.
impl From<String> for TangentError {
    fn from(why: String) -> Self {
        TangentError::Refused(why)
    }
}

/// A refusal worded for the user.
impl From<&str> for TangentError {
    fn from(why: &str) -> Self {
        TangentError::Refused(why.to_owned())
    }
}

impl From<tokio::task::JoinError> for TangentError {
    fn from(e: tokio::task::JoinError) -> Self {
        TangentError::Failed(e.to_string())
    }
}

impl From<crate::research::files::ResearchDirError> for TangentError {
    fn from(e: crate::research::files::ResearchDirError) -> Self {
        TangentError::Failed(e.to_string())
    }
}

/// Opens Tangents for the core: registers it when missing, makes its folder, and returns its
/// database (`own` when Tangents is the open project), or why tangents are off. Blocks on the
/// DB thread: call it before the core's runtime starts.
pub(crate) fn open(paths: &Paths, own: &Db, project: &ProjectState) -> Result<Db, TangentError> {
    let off = |e: &dyn std::fmt::Display| TangentError::Off(e.to_string());
    own.ensure_tangents().wait().map_err(|e| off(&e))?;
    let dir = paths.project_dir(TANGENTS_PROJECT);
    std::fs::create_dir_all(&dir).map_err(|e| off(&format!("creating {}: {e}", dir.display())))?;
    if project.key() == TANGENTS_PROJECT {
        return Ok(own.clone());
    }
    Db::open(paths, Some(TANGENTS_PROJECT)).map_err(|e| off(&format!("opening Tangents: {e}")))
}

/// The task title a question gives: its first line, cut at [`TITLE_MAX`] characters.
pub(crate) fn title_of(question: &str) -> String {
    let line = question
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let line = line.trim();
    if line.chars().count() <= TITLE_MAX {
        return line.to_owned();
    }
    let cut: String = line.chars().take(TITLE_MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The capture key of a tangent session's own turn (an `Agent` call id never looks like it).
fn turn_key(agent: &AgentId) -> String {
    format!("turn:{}", agent.0)
}

/// Today as `YYYY-MM-DD`, for the notes a tangent collects.
fn today() -> String {
    crate::journal::today().to_string()
}

/// `notes`, then a blank line and `line`.
fn append(notes: &str, line: &str) -> String {
    if notes.trim().is_empty() {
        line.to_owned()
    } else {
        format!("{}\n\n{line}", notes.trim_end())
    }
}

/// A tangent's findings as new findings of `task` (quick findings: rounds stay behind), with
/// their sources. The source ids change; the dedup key is computed as capture does.
fn copies(r: &TaskResearch, task: TaskId, tangent: TaskId) -> Vec<NewFinding> {
    r.findings
        .iter()
        .map(|f| NewFinding {
            task: Some(task),
            round: None,
            agent: f.agent.clone(),
            label: f.label.clone(),
            description: f.description.clone(),
            body: f.body.clone(),
            body_from: f.body_from.clone(),
            subagent_type: f.subagent_type.clone(),
            started_ms: f.started_ms,
            tokens: f.tokens,
            sources: f
                .sources
                .iter()
                .filter_map(|id| r.sources.iter().find(|s| s.id == *id))
                .map(|s| NewSource {
                    kind: s.kind,
                    locator: s.locator.clone(),
                    tool: s.tool.clone(),
                    query: s.query.clone(),
                    content: s.content.clone(),
                    cached: s.cached,
                    is_error: s.is_error,
                    content_hash: capture::content_hash(&[
                        &s.tool,
                        s.query.as_deref().unwrap_or(""),
                        &s.content,
                        if s.is_error { "error" } else { "" },
                    ]),
                    tool_use_id: format!("tangent-{}", tangent.0),
                })
                .collect(),
        })
        .collect()
}

/// Files `findings` into `db`, all or nothing visible: stops at the first failure.
async fn file_all(
    db: &Db,
    findings: Vec<NewFinding>,
) -> Result<Vec<(FindingRow, Vec<SourceRow>)>, DbError> {
    let mut out = Vec::new();
    for finding in findings {
        out.push(db.file_finding(finding).await?);
    }
    Ok(out)
}

impl Core {
    /// Tangents' database, or why tangents are off.
    pub(crate) fn tangents_db(&self) -> Result<Db, TangentError> {
        self.tangents.clone()
    }

    /// Whether the open project is Tangents itself: its tasks are then the snapshot's too.
    fn in_tangents(&self) -> bool {
        self.project.key() == TANGENTS_PROJECT
    }

    /// The Tangents task of a tangent session, if `agent` is one.
    pub(crate) fn tangent_of(&self, agent: &AgentId) -> Option<TaskId> {
        self.agents
            .get(agent)
            .filter(|r| r.agent.kind == AgentKind::Tangent)
            .and_then(|_| tangent_task(agent))
    }

    /// The folder tangent sessions run in: Tangents' own (a research project's sessions run in
    /// its state folder).
    pub(crate) fn tangents_folder(&self) -> PathBuf {
        self.paths.project_dir(TANGENTS_PROJECT)
    }

    /// The research folder of tangent `task`.
    fn tangent_dir(&self, task: TaskId, title: &str) -> PathBuf {
        let root = self.paths.project_dir(TANGENTS_PROJECT).join("research");
        files::task_dir(&root, task, title)
    }

    /// Tangent `task` of Tangents.
    async fn tangent_task(&self, db: &Db, task: TaskId) -> Result<Task, TangentError> {
        let tasks = db.load_tasks().await?;
        tasks
            .into_iter()
            .find(|t| t.id == task)
            .ok_or_else(|| TangentError::Refused(format!("there is no tangent #{}", task.0)))
    }

    /// The summary of one tangent task, with its research when it has any.
    async fn summarise(&self, db: &Db, task: Task) -> TangentSummary {
        let research = match db.research_summary(task.id).await {
            Ok(row) if row.findings > 0 || !row.rounds.is_empty() || row.report_ms.is_some() => {
                let dir = self.tangent_dir(task.id, &task.title);
                Some(crate::research::summary(row, dir))
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!("loading the research of tangent #{}: {e}", task.id.0);
                None
            }
        };
        TangentSummary { task, research }
    }

    /// Every tangent that is not archived, oldest first, for the snapshot.
    pub(crate) async fn tangent_summaries(&self) -> Vec<TangentSummary> {
        let Ok(db) = self.tangents_db() else {
            return Vec::new();
        };
        let tasks = match db.load_tasks().await {
            Ok(tasks) => tasks,
            Err(e) => {
                tracing::warn!("loading tangents: {e}");
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        let mut open: Vec<Task> = tasks
            .into_iter()
            .filter(|t| t.status != TaskStatus::Archived)
            .collect();
        open.sort_by_key(|t| t.id);
        for task in open {
            out.push(self.summarise(&db, task).await);
        }
        out
    }

    /// Tells the UI how tangent `task` stands now: upserted, or removed once archived. When
    /// Tangents is the open project, its task and research change in the snapshot too.
    pub(crate) async fn emit_tangent(&mut self, task: TaskId) {
        let Ok(db) = self.tangents_db() else {
            return;
        };
        let Ok(t) = self.tangent_task(&db, task).await else {
            return;
        };
        if self.in_tangents() {
            let title = t.title.clone();
            self.emit(Event::Task(TaskEvent::Upserted(t.clone())));
            self.emit_research(task, &title).await;
        }
        if t.status == TaskStatus::Archived {
            self.emit(Event::Tangent(TangentEvent::Removed(task)));
        } else {
            let summary = self.summarise(&db, t).await;
            self.emit(Event::Tangent(TangentEvent::Upserted(summary)));
        }
    }

    /// `TangentCommand::New`: saves the question as a task in Tangents' Unsorted list and starts
    /// its session in Tangents' folder, read-only like a side chat, with the question as its
    /// first prompt.
    pub(crate) async fn start_tangent(&mut self, question: String) {
        let question = question.trim().to_owned();
        if question.is_empty() {
            return self.fail("a tangent needs a question: :tangent <question>");
        }
        let db = match self.tangents_db() {
            Ok(db) => db,
            Err(e) => return self.fail(e),
        };
        let title = title_of(&question);
        let asked = format!(
            "Asked from {} on {}.",
            self.project.summary(&self.paths).name,
            today()
        );
        let notes = if title == question {
            asked
        } else {
            format!("{question}\n\n{asked}")
        };
        let new = NewTask {
            title: title.clone(),
            p_level: None,
            bucket: None,
            notes,
            follow_up_of: None,
            pr: None,
            related: Vec::new(),
            effort: Default::default(),
            criterion: None,
        };
        let task = match db.create_task(new, false).await {
            Ok(tasks) => match tasks.first() {
                Some(t) => t.id,
                None => return self.fail("Tangents created no task"),
            },
            Err(e) => return self.fail(format!("saving the tangent: {e}")),
        };
        let id = tangent_agent(task);
        let name = id.0.clone();
        let mut agent = new_agent(id.clone(), name.clone(), AgentKind::Tangent, None);
        // It runs in `dontAsk` with side chats' read-only allow-list (launch).
        agent.permission_mode = PermissionMode::Manual;
        let row = AgentRow {
            agent,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            transcript_path: None,
            task_list_id: Some(self.task_list_id(None, &name)),
            created_ms: now_ms(),
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        };
        self.agents.insert(id.clone(), row);
        self.save_agent(&id);
        self.activity(
            ActivityKind::Lifecycle,
            Some(&id),
            format!("tangent #{} asked: {title} (kept in Tangents)", task.0),
        );
        self.emit_tangent(task).await;
        let cwd = self.tangents_folder();
        if let Err(e) = self.launch(&id, cwd, None) {
            // The question stays in Tangents; only the session is gone.
            self.closed(&id);
            return self.fail(e);
        }
        self.research
            .capture
            .watch_turn(&id, &turn_key(&id), task, now_ms());
        if let Err(e) = self.send_text(&id, &question, EntryKind::User, question.clone()) {
            self.fail(e);
        }
    }

    /// An event on a tangent session's main stream: its text and its own source tool calls are
    /// watched, and its answer is filed when the turn's `result` arrives. A later `result` of
    /// the same turn finds nothing watched and files nothing.
    pub(crate) async fn tangent_turn_event(
        &mut self,
        agent: &AgentId,
        task: TaskId,
        event: &DriverEvent,
    ) {
        let key = turn_key(agent);
        match event {
            DriverEvent::Assistant(_) | DriverEvent::ToolResult(_) => {
                self.research
                    .capture
                    .watch_turn(agent, &key, task, now_ms());
                self.research.capture.subagent_event(&key, event);
            }
            DriverEvent::TurnResult(result) => {
                let outcomes = self
                    .research
                    .capture
                    .turn_ended(&key, result.result.as_deref());
                for outcome in outcomes {
                    Box::pin(self.research_outcome(outcome)).await;
                }
            }
            _ => {}
        }
    }

    /// A tangent's answer, or one of its subagents' findings: filed on its task in Tangents,
    /// with its files in Tangents' research folder.
    pub(crate) async fn file_tangent_finding(
        &mut self,
        key: String,
        from: BodyFrom,
        finding: NewFinding,
    ) {
        let Some(task) = finding.task else {
            return;
        };
        let agent = finding.agent.clone();
        let db = match self.tangents_db() {
            Ok(db) => db,
            Err(e) => return self.fail(e),
        };
        let (row, sources) = match db.file_finding(finding).await {
            Ok(filed) => filed,
            Err(e) => {
                tracing::warn!("filing a tangent's finding: {e}");
                return self.fail(format!(
                    "tangent #{}: could not file a finding: {e}",
                    task.0
                ));
            }
        };
        self.research.capture.filed(&key, row.id, from, &row.body);
        let title = self
            .tangent_task(&db, task)
            .await
            .map(|t| t.title)
            .unwrap_or_default();
        let dir = self.tangent_dir(task, &title);
        self.write_finding_files(&dir, &row, &sources);
        self.activity(
            ActivityKind::Task,
            Some(&agent),
            format!(
                "tangent #{}: filed F{} {} ({} sources)",
                task.0,
                row.id.0,
                row.label,
                row.sources.len()
            ),
        );
        self.emit_tangent(task).await;
    }

    /// A better body for a tangent's finding: stored and its file rewritten.
    pub(crate) async fn upgrade_tangent_finding(
        &mut self,
        task: TaskId,
        finding: FindingId,
        body: String,
        from: BodyFrom,
    ) {
        let Ok(db) = self.tangents_db() else {
            return;
        };
        if let Err(e) = db.set_finding_body(finding, body, from.as_str()).await {
            tracing::warn!("updating tangent finding F{}: {e}", finding.0);
            return;
        }
        let (Ok(t), Ok(r)) = (
            self.tangent_task(&db, task).await,
            db.task_research(task).await,
        ) else {
            return;
        };
        if let Some(f) = r.findings.iter().find(|f| f.id == finding) {
            let path = self.tangent_dir(task, &t.title).join(&f.path);
            if let Err(e) = files::write_atomic(&path, &files::finding_md(f, &r.sources)) {
                self.fail(format!("tangent: writing {}: {e}", path.display()));
            }
        }
    }

    /// `TangentCommand::Load`.
    pub(crate) async fn load_tangent(&mut self, tangent: TaskId, what: ResearchWhat) {
        let db = match self.tangents_db() {
            Ok(db) => db,
            Err(e) => return self.fail(e),
        };
        let t = match self.tangent_task(&db, tangent).await {
            Ok(t) => t,
            Err(e) => return self.fail(e),
        };
        let r = match db.task_research(tangent).await {
            Ok(r) => r,
            Err(e) => return self.fail(e.to_string()),
        };
        let dir = self.tangent_dir(tangent, &t.title);
        match crate::research::content_of(&dir, tangent, &t.title, &r, &what) {
            Ok(content) => self.emit(Event::Tangent(TangentEvent::Loaded {
                tangent,
                what,
                content,
            })),
            Err(e) => self.fail(format!("tangent: {e}")),
        }
    }

    /// The tangent and its research, for a move out of Tangents; refused when it has no answer
    /// yet (nothing to move) and `need_answer`.
    async fn tangent_for_move(
        &self,
        tangent: TaskId,
        need_answer: bool,
    ) -> Result<(Db, Task, TaskResearch), TangentError> {
        let db = self.tangents_db()?;
        let t = self.tangent_task(&db, tangent).await?;
        if t.status == TaskStatus::Archived {
            return Err(format!("tangent #{} has already left Tangents", tangent.0).into());
        }
        let r = db.task_research(tangent).await?;
        if need_answer && r.findings.is_empty() {
            return Err(format!(
                "tangent #{} has no answer yet: wait for it, or drop it",
                tangent.0
            )
            .into());
        }
        Ok((db, t, r))
    }

    /// `TangentCommand::Attach`: the tangent's answers and sources become quick findings of a
    /// task of the open project, and the tangent leaves Tangents.
    pub(crate) async fn attach_tangent(&mut self, tangent: TaskId, task: TaskId) {
        if self.project.registered().is_none() {
            return self.fail("this folder is not a project yet: :project add keeps it first");
        }
        if self.in_tangents() {
            return self.fail("Tangents is open, so the tangent is here already");
        }
        let Some(target) = self.task(task).await else {
            return self.fail(format!("there is no task #{}", task.0));
        };
        let (db, t, r) = match self.tangent_for_move(tangent, true).await {
            Ok(found) => found,
            Err(e) => return self.fail(e),
        };
        let dir = match self.research_dir(task, &target.title) {
            Ok(dir) => dir,
            Err(e) => return self.fail(e.to_string()),
        };
        let filed = match file_all(&self.db, copies(&r, task, tangent)).await {
            Ok(filed) => filed,
            Err(e) => return self.fail(format!("attaching tangent #{}: {e}", tangent.0)),
        };
        for (row, sources) in &filed {
            self.write_finding_files(&dir, row, sources);
        }
        self.emit_research(task, &target.title).await;
        let here = self.project.summary(&self.paths).name;
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "tangent \"{}\" attached to #{} {}: {} findings",
                t.title,
                task.0,
                target.title,
                filed.len()
            ),
        );
        let note = format!(
            "Attached to #{} {} in {here} on {}.",
            task.0,
            target.title,
            today()
        );
        self.leave_tangents(&db, &t, note).await;
    }

    /// The project `id` and a handle on its database (the open project's own when it is open).
    async fn project_db(
        &self,
        id: &ProjectId,
    ) -> Result<(crate::projects::Project, Db, bool), TangentError> {
        if id.0 == TANGENTS_PROJECT {
            return Err("the tangent is in Tangents already: pick another project".into());
        }
        let project =
            self.db.get_project(id.0.clone()).await?.ok_or_else(|| {
                crate::projects::ProjectError::UnknownId(id.0.clone()).to_string()
            })?;
        if self.project.registered().is_some_and(|p| p.id == id.0) {
            return Ok((project, self.db.clone(), true));
        }
        let paths = self.paths.clone();
        let key = project.id.clone();
        let db = tokio::task::spawn_blocking(move || Db::open(&paths, Some(&key))).await??;
        Ok((project, db, false))
    }

    /// Creates a task in a project's `db` (the open one when `here`), telling the UI when it is
    /// the open project's.
    async fn task_in(&mut self, db: &Db, here: bool, new: NewTask) -> Result<Task, TangentError> {
        if here {
            let id = self.create_task(new, false).await?;
            return self
                .task(id)
                .await
                .ok_or_else(|| TangentError::Failed("the new task is gone".into()));
        }
        db.create_task(new, false)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| TangentError::Failed("no task was created".into()))
    }

    /// `TangentCommand::File`: a new task in the project with the tangent's question, notes,
    /// answers and sources; the tangent leaves Tangents.
    pub(crate) async fn file_tangent(&mut self, tangent: TaskId, project: ProjectId) {
        if let Err(e) = self.try_file_tangent(tangent, &project).await {
            self.fail(e);
        }
    }

    async fn try_file_tangent(
        &mut self,
        tangent: TaskId,
        id: &ProjectId,
    ) -> Result<(), TangentError> {
        let (db, t, r) = self.tangent_for_move(tangent, false).await?;
        let (project, target, here) = self.project_db(id).await?;
        let root = crate::research::root_of(&self.paths, &project)?;
        let new = NewTask {
            title: t.title.clone(),
            p_level: None,
            bucket: None,
            notes: append(
                &t.notes,
                &format!(
                    "Filed from Tangents (tangent #{}) on {}.",
                    tangent.0,
                    today()
                ),
            ),
            follow_up_of: None,
            pr: None,
            related: Vec::new(),
            effort: Default::default(),
            criterion: None,
        };
        let task = self.task_in(&target, here, new).await?;
        let filed = file_all(&target, copies(&r, task.id, tangent))
            .await
            .map_err(|e| format!("filing tangent #{}: {e}", tangent.0))?;
        let dir = files::task_dir(&root, task.id, &task.title);
        for (row, sources) in &filed {
            self.write_finding_files(&dir, row, sources);
        }
        if here {
            self.emit_research(task.id, &task.title).await;
        }
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "tangent \"{}\" filed into {} as #{} ({} findings)",
                t.title,
                project.name,
                task.id.0,
                filed.len()
            ),
        );
        let note = format!(
            "Filed into {} as #{} on {}.",
            project.name,
            task.id.0,
            today()
        );
        self.leave_tangents(&db, &t, note).await;
        Ok(())
    }

    /// `TangentCommand::StartFrom`: a new task in the project whose notes link back to the
    /// tangent; the tangent stays in Tangents, marked done.
    pub(crate) async fn start_from_tangent(&mut self, tangent: TaskId, project: ProjectId) {
        if let Err(e) = self.try_start_from_tangent(tangent, &project).await {
            self.fail(e);
        }
    }

    async fn try_start_from_tangent(
        &mut self,
        tangent: TaskId,
        id: &ProjectId,
    ) -> Result<(), TangentError> {
        let (db, t, r) = self.tangent_for_move(tangent, false).await?;
        let (project, target, here) = self.project_db(id).await?;
        let dir = self.tangent_dir(tangent, &t.title);
        let mut notes = format!(
            "Started from the tangent \"{}\" (Tangents #{}), on {}. Its answers and sources stay \
             in Tangents: reading mode's Tangents section, or {}.",
            t.title,
            tangent.0,
            today(),
            dir.display()
        );
        if let Some(answer) = r
            .findings
            .iter()
            .rev()
            .find(|f| f.label == capture::TURN_LABEL)
        {
            notes = append(&notes, &format!("Latest answer:\n\n{}", answer.body.trim()));
        }
        let new = NewTask {
            title: t.title.clone(),
            p_level: None,
            bucket: None,
            notes,
            follow_up_of: None,
            pr: None,
            related: Vec::new(),
            effort: Default::default(),
            criterion: None,
        };
        let task = self.task_in(&target, here, new).await?;
        let line = format!(
            "Started as #{} in {} on {}.",
            task.id.0,
            project.name,
            today()
        );
        db.update_task(
            tangent,
            vec![
                TaskChange::Notes(append(&t.notes, &line)),
                TaskChange::Status(TaskStatus::Done),
            ],
        )
        .await?;
        self.activity(
            ActivityKind::Task,
            None,
            format!(
                "tangent \"{}\" started as #{} in {}",
                t.title, task.id.0, project.name
            ),
        );
        self.emit_tangent(tangent).await;
        self.refresh_new_tangents().await;
        Ok(())
    }

    /// `TangentCommand::Drop`: archived in Tangents, nothing deleted.
    pub(crate) async fn drop_tangent(&mut self, tangent: TaskId) {
        let (db, t, _) = match self.tangent_for_move(tangent, false).await {
            Ok(found) => found,
            Err(e) => return self.fail(e),
        };
        self.activity(
            ActivityKind::Task,
            None,
            format!("tangent \"{}\" dropped", t.title),
        );
        self.leave_tangents(&db, &t, format!("Dropped on {}.", today()))
            .await;
    }

    /// The tangent leaves Tangents: archived there with `note`, its session (if this project
    /// runs it) closed, and the "new tangents" count brought up to date.
    async fn leave_tangents(&mut self, db: &Db, t: &Task, note: String) {
        if let Err(e) = db
            .update_task(
                t.id,
                vec![
                    TaskChange::Notes(append(&t.notes, &note)),
                    TaskChange::Status(TaskStatus::Archived),
                ],
            )
            .await
        {
            return self.fail(format!("archiving tangent #{}: {e}", t.id.0));
        }
        let agent = tangent_agent(t.id);
        if self
            .agents
            .get(&agent)
            .is_some_and(|r| r.agent.kind == AgentKind::Tangent)
        {
            self.close_slice(&agent, true);
        }
        self.emit_tangent(t.id).await;
        self.refresh_new_tangents().await;
    }

    /// How many tangents nobody has triaged yet.
    async fn new_tangents(&self) -> u32 {
        let n = self
            .tangent_summaries()
            .await
            .iter()
            .filter(|t| t.is_new())
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }

    /// The first open of a day (ADR 0012's morning path): one "N new tangents" item when
    /// Tangents has untriaged tangents. `last_used` is when the coordinator was last used,
    /// read before anything was added to its conversation.
    pub(crate) async fn morning_tangents(&mut self, last_used: crate::journal::LastUse) {
        let today = crate::journal::today();
        let first_open = last_used.date.is_some_and(|d| d < today);
        if !first_open || self.project.registered().is_none() {
            return;
        }
        // Once a day: a second open before the coordinator is used again asks nothing more.
        let marker = self
            .paths
            .project_dir(self.project.key())
            .join(MORNING_MARKER);
        let stamp = today.to_string();
        if std::fs::read_to_string(&marker).is_ok_and(|seen| seen.trim() == stamp) {
            return;
        }
        if let Err(e) = files::write_atomic(&marker, &stamp) {
            tracing::warn!("writing {}: {e}", marker.display());
        }
        let count = self.new_tangents().await;
        if count == 0 {
            return;
        }
        if self.new_tangents_item().is_some() {
            return self.refresh_new_tangents().await;
        }
        self.raise(
            &AgentId(COORDINATOR.into()),
            InboxKind::NewTangents { count },
            Waiter::None,
        );
    }

    fn new_tangents_item(&self) -> Option<dovetail_api::inbox::InboxId> {
        self.inbox
            .values()
            .find(|e| matches!(e.item.kind, InboxKind::NewTangents { .. }))
            .map(|e| e.item.id)
    }

    /// Keeps an open "new tangents" item's count true, and takes it out once none are left.
    pub(crate) async fn refresh_new_tangents(&mut self) {
        let Some(id) = self.new_tangents_item() else {
            return;
        };
        let count = self.new_tangents().await;
        if count == 0 {
            self.resolve(id, Some("all triaged".into()));
            return;
        }
        let Some(entry) = self.inbox.get_mut(&id) else {
            return;
        };
        if entry.item.kind == (InboxKind::NewTangents { count }) {
            return;
        }
        entry.item.kind = InboxKind::NewTangents { count };
        let item = entry.item.clone();
        self.persist("an inbox item", self.db.insert_inbox(item.clone()));
        self.emit(Event::Inbox(InboxEvent::Upserted(item)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_gives_a_short_title() {
        assert_eq!(title_of("  Why is the sky blue?  "), "Why is the sky blue?");
        assert_eq!(title_of("\nfirst line\nsecond"), "first line");
        let long = "x".repeat(200);
        let title = title_of(&long);
        assert_eq!(title.chars().count(), TITLE_MAX);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn notes_gain_a_line() {
        assert_eq!(append("", "b"), "b");
        assert_eq!(append("a\n", "b"), "a\n\nb");
    }
}

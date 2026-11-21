//! The single DB thread: typed requests, migrations, `registry.db` and per-project `state.db`.
//!
//! Every SQLite call in the app goes through one thread that owns the connections. Callers hold a
//! cheap, cloneable [`Db`] and call its typed methods; each returns a [`Pending`], which they
//! either `.await` (in the core) or [`Pending::wait`] on (in the binary's start-up and
//! `dovetail add`). Migrations are forward-only and tracked with `PRAGMA user_version`: step `n`
//! of a list moves a database from version `n` to `n + 1`.
//!
//! A folder that is not a project yet keeps its state in an in-memory `state.db`
//! ([`Db::open_unregistered`]) until [`Db::persist_state`] writes it to the new project's file
//! (ADR 0005).

mod canvas;
mod counts;
mod design;
mod efforts;
mod maps;
mod registry;

pub use registry::NewRepo;
pub mod research;
mod sessions;
mod tasks;

pub use canvas::{CanvasState, USER};
pub use efforts::CriterionStep;
pub use maps::{Drawn, NewMapVersion};
pub use sessions::{AgentRow, ModelChoice, QueuedSlice};

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};

use dovetail_api::map::{Map, MapId, MapSummary, NodeId};
use dovetail_api::{
    activity::{ActivityItem, AgentMessage},
    agent::AgentId,
    conversation::{ConversationEntry, EntryId},
    decision::{Decision, DecisionAuthor, NewDecision},
    effort::{CriterionId, Effort, EffortChange, EffortId, NewEffort, WhereWasI},
    inbox::{InboxId, InboxItem},
    task::{NewTask, Task, TaskChange, TaskId},
};
use rusqlite::Connection;
use tokio::sync::oneshot;

use crate::paths::Paths;
use crate::projects::Project;

/// One migration: its path under `db/migrations/` (`<db>/NNNN_slug.sql`) and its SQL. The path is
/// kept so `migrations_are_contiguous_and_listed_in_order` can check each list against its directory:
/// `user_version` counts steps, so a gap or a misordered entry would silently skip a migration.
type Migration = (&'static str, &'static str);

macro_rules! migration {
    ($db:literal, $file:literal) => {
        (
            concat!($db, "/", $file),
            include_str!(concat!("db/migrations/", $db, "/", $file)),
        )
    };
}

/// Migrations of `registry.db`, in order.
const REGISTRY_MIGRATIONS: &[Migration] = &[
    migration!("registry", "0001_init.sql"),
    migration!("registry", "0002_optional_git.sql"),
    migration!("registry", "0003_project_kind.sql"),
    migration!("registry", "0004_repos.sql"),
];
/// Migrations of a project's `state.db`, in order.
const STATE_MIGRATIONS: &[Migration] = &[
    migration!("state", "0001_init.sql"),
    migration!("state", "0002_agents_conversation.sql"),
    migration!("state", "0003_inbox_activity.sql"),
    migration!("state", "0004_slice_queue.sql"),
    migration!("state", "0005_restart_state.sql"),
    migration!("state", "0006_entry_attachments.sql"),
    migration!("state", "0007_todos.sql"),
    migration!("state", "0008_subagent_progress.sql"),
    migration!("state", "0009_subagent_tokens.sql"),
    migration!("state", "0010_agent_claude_home.sql"),
    migration!("state", "0011_agent_takeover.sql"),
    migration!("state", "0012_maps.sql"),
    migration!("state", "0013_research.sql"),
    migration!("state", "0014_decision_evidence.sql"),
    migration!("state", "0015_research_followups.sql"),
    migration!("state", "0016_agent_model_choice.sql"),
    migration!("state", "0017_efforts.sql"),
    migration!("state", "0018_canvases.sql"),
    migration!("state", "0019_agent_repo.sql"),
    migration!("state", "0020_design_variants.sql"),
    migration!("state", "0021_blocks.sql"),
    migration!("state", "0022_slice_queue_model.sql"),
];

/// Errors from the database layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite failed.
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A file or directory could not be prepared.
    #[error("{path}: {source}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The database was written by a newer Dovetail.
    #[error("{path} has schema version {found}, this build supports up to {supported}")]
    SchemaTooNew {
        /// The database file.
        path: PathBuf,
        /// Its version.
        found: u32,
        /// The newest version this build knows.
        supported: u32,
    },
    /// A row did not look like Dovetail wrote it.
    #[error("corrupt data: {0}")]
    Corrupt(String),
    /// A referenced row does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The request is not allowed in the current state (a round is already open, the task is
    /// archived).
    #[error("{0}")]
    Refused(String),
    /// A project request on a `Db` opened without a project.
    #[error("no project database is open")]
    NoProject,
    /// A project's `state.db` already exists where unregistered state was to be written.
    #[error("{0} already exists; move it away and register again")]
    StateExists(PathBuf),
    /// The DB thread has stopped.
    #[error("the database thread has stopped")]
    Closed,
}

/// The reply to a request: await it, or block on [`Pending::wait`] outside a tokio runtime.
#[derive(Debug)]
pub struct Pending<T>(oneshot::Receiver<Result<T, DbError>>);

impl<T> Pending<T> {
    /// Blocks the calling thread until the DB thread replies. Never call this from async code.
    pub fn wait(self) -> Result<T, DbError> {
        self.0.blocking_recv().unwrap_or(Err(DbError::Closed))
    }
}

impl<T> Future for Pending<T> {
    type Output = Result<T, DbError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(DbError::Closed)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// The connections the DB thread owns.
struct Conns {
    registry: Connection,
    state: Option<Connection>,
}

impl Conns {
    fn state(&mut self) -> Result<&mut Connection, DbError> {
        self.state.as_mut().ok_or(DbError::NoProject)
    }
}

/// One request: runs on the DB thread with the connections and the time it started. Built only
/// by [`Db`]'s typed methods, each of which sends its result back on its own reply channel.
type Job = Box<dyn FnOnce(&mut Conns, i64) + Send>;

/// A handle to the DB thread. Clone it freely; the thread ends when the last handle is dropped.
#[derive(Debug, Clone)]
pub struct Db {
    requests: mpsc::Sender<Job>,
}

impl Db {
    /// Opens `registry.db` and, when `project` is given, that project's `state.db`, migrating both,
    /// then starts the DB thread.
    pub fn open(paths: &Paths, project: Option<&str>) -> Result<Self, DbError> {
        let state = project
            .map(|id| open_connection(&paths.state_db(id), STATE_MIGRATIONS))
            .transpose()?;
        Self::start(paths, state)
    }

    /// Opens `registry.db` and an empty in-memory `state.db` for a folder that is not a project
    /// yet. Nothing is written for the folder until [`Db::persist_state`].
    pub fn open_unregistered(paths: &Paths) -> Result<Self, DbError> {
        let mut state = Connection::open_in_memory()?;
        state.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut state, Path::new(":memory:"), STATE_MIGRATIONS)?;
        Self::start(paths, Some(state))
    }

    fn start(paths: &Paths, state: Option<Connection>) -> Result<Self, DbError> {
        let registry = open_connection(&paths.registry_db(), REGISTRY_MIGRATIONS)?;
        let (requests, inbox) = mpsc::channel::<Job>();
        let mut conns = Conns { registry, state };
        std::thread::Builder::new()
            .name("dovetail-db".into())
            .spawn(move || {
                for job in inbox {
                    job(&mut conns, now_ms());
                }
            })
            .map_err(|source| DbError::Io {
                path: PathBuf::from("dovetail-db"),
                source,
            })?;
        Ok(Self { requests })
    }

    fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Conns, i64) -> Result<T, DbError> + Send + 'static,
    ) -> Pending<T> {
        let (reply, pending) = oneshot::channel();
        // If the thread is gone the job is dropped with its reply, and the caller sees `Closed`.
        let _ = self.requests.send(Box::new(move |conns, now| {
            let _ = reply.send(f(conns, now));
        }));
        Pending(pending)
    }

    // --- registry ---

    /// Registers a folder as a project with one repo, or returns the project that has it
    /// already. The id is `slug`, numbered (`slug-2`, ...) when taken.
    pub fn register_project(&self, slug: String, name: String, repo: NewRepo) -> Pending<Project> {
        self.run(move |c, now| registry::register(&mut c.registry, &slug, &name, &repo, now))
    }

    /// Creates a project (ADR 0021) with an optional account and first repo.
    pub fn create_project(
        &self,
        slug: String,
        name: String,
        account: Option<String>,
        repo: Option<NewRepo>,
    ) -> Pending<Project> {
        self.run(move |c, now| {
            registry::create(
                &mut c.registry,
                &slug,
                &name,
                account.as_deref(),
                repo.as_ref(),
                now,
            )
        })
    }

    /// Adds a repo to project `id`. Refused when any project has it already, or this one a
    /// repo of the same name.
    pub fn add_repo(&self, id: String, repo: NewRepo) -> Pending<Project> {
        self.run(move |c, now| registry::add_repo(&mut c.registry, &id, &repo, now))
    }

    /// Removes a repo from project `id`.
    pub fn remove_repo(&self, id: String, repo: dovetail_api::project::RepoId) -> Pending<Project> {
        self.run(move |c, _| registry::remove_repo(&c.registry, &id, repo))
    }

    /// Renames project `id`.
    pub fn rename_project(&self, id: String, name: String) -> Pending<Project> {
        self.run(move |c, _| registry::rename(&c.registry, &id, &name))
    }

    /// Sets or clears project `id`'s account.
    pub fn set_project_account(&self, id: String, account: Option<String>) -> Pending<Project> {
        self.run(move |c, _| registry::set_account(&c.registry, &id, account.as_deref()))
    }

    /// The open slices in a project's `state.db` at `path` that work in `repo` (`first`: the
    /// project's first repo), read without migrating it.
    pub fn slices_in_repo(&self, path: PathBuf, repo: i64, first: bool) -> Pending<Vec<String>> {
        self.run(move |_, _| counts::slices_in_repo(&path, repo, first))
    }

    /// Home's counts for project `id`, read from its `state.db` without migrating it.
    pub fn project_counts(&self, path: PathBuf) -> Pending<dovetail_api::project::ProjectCounts> {
        self.run(move |_, _| counts::read(&path))
    }

    /// Registers a research project named `name` under id `slug` (numbered when taken). `None`
    /// when a research project of that name exists already.
    pub fn register_research(&self, slug: String, name: String) -> Pending<Option<Project>> {
        self.run(move |c, now| registry::register_research(&mut c.registry, &slug, &name, now))
    }

    /// Registers Tangents, the built-in research project, if it is missing (ADR 0017), and
    /// returns it. Refused when a code project holds its reserved id.
    pub fn ensure_tangents(&self) -> Pending<Project> {
        self.run(|c, now| registry::ensure_tangents(&mut c.registry, now))
    }

    /// The project with this id, if registered.
    pub fn get_project(&self, id: String) -> Pending<Option<Project>> {
        self.run(move |c, _| registry::get(&c.registry, &id))
    }

    /// Every registered project, most recently opened first.
    pub fn list_projects(&self) -> Pending<Vec<Project>> {
        self.run(|c, _| registry::list(&c.registry))
    }

    /// The project with this git common dir, if registered.
    pub fn find_project(&self, git_common_dir: PathBuf) -> Pending<Option<Project>> {
        self.run(move |c, _| registry::find(&c.registry, &git_common_dir))
    }

    /// The project without git registered for this folder, if any.
    pub fn find_folder(&self, path: PathBuf) -> Pending<Option<Project>> {
        self.run(move |c, _| registry::find_folder(&c.registry, &path))
    }

    /// Records the git common dir of a folder repo that has become a git repo.
    pub fn set_git_dir(
        &self,
        repo: dovetail_api::project::RepoId,
        git_common_dir: PathBuf,
    ) -> Pending<Project> {
        self.run(move |c, _| registry::set_git_dir(&c.registry, repo, &git_common_dir))
    }

    /// Marks a project as opened now.
    pub fn touch_project(&self, id: String) -> Pending<()> {
        self.run(move |c, now| registry::touch(&c.registry, &id, now))
    }

    /// Writes the in-memory `state.db` of [`Db::open_unregistered`] to `paths.state_db(project)`
    /// and continues on the file (ADR 0005). A project already on file is left alone.
    pub fn persist_state(&self, path: PathBuf) -> Pending<()> {
        self.run(move |c, _| {
            let state = c.state()?;
            if state.path().is_some_and(|p| !p.is_empty()) {
                return Ok(());
            }
            if path.exists() {
                return Err(DbError::StateExists(path));
            }
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|source| DbError::Io {
                    path: dir.to_owned(),
                    source,
                })?;
            }
            state.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
            c.state = Some(open_connection(&path, STATE_MIGRATIONS)?);
            Ok(())
        })
    }

    // --- tasks and decisions ---

    /// Every task of the open project, archived ones included.
    pub fn load_tasks(&self) -> Pending<Vec<Task>> {
        self.run(|c, _| tasks::load_tasks(c.state()?))
    }

    /// Every decision of the open project, oldest first.
    pub fn load_decisions(&self) -> Pending<Vec<Decision>> {
        self.run(|c, _| tasks::load_decisions(c.state()?, None))
    }

    /// The decisions of task `task`, oldest first.
    pub fn task_decisions(&self, task: TaskId) -> Pending<Vec<Decision>> {
        self.run(move |c, _| tasks::load_decisions(c.state()?, Some(task)))
    }

    /// Creates a task. Resolves to the new task followed by tasks whose relations changed.
    pub fn create_task(&self, new: NewTask, coordinator_created: bool) -> Pending<Vec<Task>> {
        self.run(move |c, now| tasks::create_task(c.state()?, new, coordinator_created, now))
    }

    /// Applies a patch. Resolves to every task it changed.
    pub fn update_task(&self, id: TaskId, patch: Vec<TaskChange>) -> Pending<Vec<Task>> {
        self.run(move |c, now| tasks::update_task(c.state()?, id, patch, now))
    }

    /// Moves a task within its bucket. Resolves to every task whose position changed.
    pub fn reorder_task(&self, id: TaskId, delta: i32) -> Pending<Vec<Task>> {
        self.run(move |c, _| tasks::reorder(c.state()?, id, delta))
    }

    /// Appends to a task's decision log.
    pub fn log_decision(&self, new: NewDecision, by: DecisionAuthor) -> Pending<Decision> {
        self.run(move |c, now| tasks::log_decision(c.state()?, new, by, Vec::new(), now))
    }

    /// Appends to a task's decision log with the findings and sources it rests on (R2).
    pub fn log_decision_with_evidence(
        &self,
        new: NewDecision,
        by: DecisionAuthor,
        evidence: Vec<dovetail_api::research::Evidence>,
    ) -> Pending<Decision> {
        self.run(move |c, now| tasks::log_decision(c.state()?, new, by, evidence, now))
    }

    // --- efforts and criteria ---

    /// Every effort of the open project with its criteria, in id order.
    pub fn load_efforts(&self) -> Pending<Vec<Effort>> {
        self.run(|c, _| efforts::load_efforts(c.state()?))
    }

    /// Creates an effort. `new.name` must already be in its stored form
    /// ([`dovetail_api::effort::effort_name`]); an empty or taken name is refused.
    pub fn create_effort(&self, new: NewEffort) -> Pending<Effort> {
        self.run(move |c, now| efforts::create_effort(c.state()?, new, now))
    }

    /// Renames an effort; `name` must already be in its stored form. Refused for Unsorted.
    pub fn rename_effort(&self, id: EffortId, name: String) -> Pending<Effort> {
        self.run(move |c, now| efforts::rename_effort(c.state()?, id, name, now))
    }

    /// Applies a patch to an effort. Resolves to the effort as it is now.
    pub fn update_effort(&self, id: EffortId, patch: Vec<EffortChange>) -> Pending<Effort> {
        self.run(move |c, now| efforts::update_effort(c.state()?, id, patch, now))
    }

    /// Replaces an effort's "where was I" card.
    pub fn set_where_was_i(&self, id: EffortId, card: WhereWasI) -> Pending<Effort> {
        self.run(move |c, now| efforts::set_where_was_i(c.state()?, id, &card, now))
    }

    /// Adds an open criterion to an effort. Resolves to the effort.
    pub fn add_criterion(&self, effort: EffortId, text: String) -> Pending<Effort> {
        self.run(move |c, now| efforts::add_criterion(c.state()?, effort, text, now))
    }

    /// Claims, approves or sends back a criterion. Resolves to its effort.
    pub fn step_criterion(&self, id: CriterionId, step: CriterionStep) -> Pending<Effort> {
        self.run(move |c, _| efforts::step_criterion(c.state()?, id, step))
    }

    /// Puts a task in an effort, serving `criterion` or none. Resolves to the task.
    pub fn move_task(
        &self,
        id: TaskId,
        effort: EffortId,
        criterion: Option<CriterionId>,
    ) -> Pending<Vec<Task>> {
        self.run(move |c, now| efforts::move_task(c.state()?, id, effort, criterion, now))
    }

    // --- maps (M15) ---

    /// Every map's shelf entry, most recently drawn first.
    pub fn load_maps(&self) -> Pending<Vec<MapSummary>> {
        self.run(|c, _| maps::load_maps(c.state()?))
    }

    /// One version of a map (`None`: the latest); `None` when there is no such map or version.
    pub fn load_map(&self, id: MapId, version: Option<u32>) -> Pending<Option<Map>> {
        self.run(move |c, _| maps::load_map(c.state()?, &id, version))
    }

    /// Ids of every node on the latest version of each structure map, which lanes may link.
    pub fn structure_nodes(&self) -> Pending<HashSet<NodeId>> {
        self.run(|c, _| maps::structure_nodes(c.state()?))
    }

    /// Stores a new map, or a new version of the one its id names.
    pub fn save_map(&self, new: NewMapVersion) -> Pending<Drawn> {
        self.run(move |c, now| maps::save_map(c.state()?, new, now))
    }

    // --- research (R1) ---

    /// The research round still open, if any.
    pub fn open_round(&self) -> Pending<Option<research::RoundRow>> {
        self.run(|c, _| research::open_round(c.state()?))
    }

    /// Requests the next round of `task`: open, or queued behind the open one.
    pub fn start_round(&self, task: TaskId, brief: String) -> Pending<research::RoundRow> {
        self.run(move |c, now| research::start_round(c.state()?, task, &brief, now))
    }

    /// Closes the open round of `task` and opens the next queued one, if any.
    pub fn close_round(
        &self,
        task: TaskId,
    ) -> Pending<(research::RoundRow, Option<research::RoundRow>)> {
        self.run(move |c, now| research::close_round(c.state()?, task, now))
    }

    /// The queued rounds, oldest request first.
    pub fn queued_rounds(&self) -> Pending<Vec<research::RoundRow>> {
        self.run(|c, _| research::queued_rounds(c.state()?))
    }

    /// Deletes a queued round of `task` (round `n`, or the one requested last).
    pub fn drop_round(&self, task: TaskId, n: Option<u32>) -> Pending<research::RoundRow> {
        self.run(move |c, _| research::drop_round(c.state()?, task, n))
    }

    /// Deletes every queued round of `task`.
    pub fn drop_queued_for(&self, task: TaskId) -> Pending<Vec<research::RoundRow>> {
        self.run(move |c, _| research::drop_queued_for(c.state()?, task))
    }

    /// Stores a finding that has no task yet.
    pub fn file_unfiled(
        &self,
        new: research::NewFinding,
    ) -> Pending<(research::FindingRow, Vec<research::SourceRow>)> {
        self.run(move |c, now| research::file_unfiled(c.state()?, new, now))
    }

    /// One unfiled finding with its sources.
    pub fn unfiled_finding(
        &self,
        id: i64,
    ) -> Pending<Option<(research::FindingRow, Vec<research::SourceRow>)>> {
        self.run(move |c, _| research::load_unfiled(c.state()?, id))
    }

    /// Every unfiled finding with its sources.
    pub fn all_unfiled(&self) -> Pending<Vec<(research::FindingRow, Vec<research::SourceRow>)>> {
        self.run(|c, _| research::load_all_unfiled(c.state()?))
    }

    /// Replaces an unfiled finding's body; resolves to whether it still existed.
    pub fn set_unfiled_body(&self, id: i64, body: String, from: &'static str) -> Pending<bool> {
        self.run(move |c, _| research::set_unfiled_body(c.state()?, id, &body, from))
    }

    /// Deletes an unfiled finding; `None` when it is already gone.
    pub fn drop_unfiled(&self, id: i64) -> Pending<Option<research::Removed>> {
        self.run(move |c, _| research::drop_unfiled(c.state()?, id))
    }

    /// Files an unfiled finding under `task` as a quick finding.
    pub fn file_unfiled_to_task(
        &self,
        id: i64,
        task: TaskId,
    ) -> Pending<(
        research::FindingRow,
        Vec<research::SourceRow>,
        research::Removed,
    )> {
        self.run(move |c, _| research::file_unfiled_to_task(c.state()?, id, task))
    }

    /// Files a finding with its sources.
    pub fn file_finding(
        &self,
        new: research::NewFinding,
    ) -> Pending<(research::FindingRow, Vec<research::SourceRow>)> {
        self.run(move |c, now| research::file_finding(c.state()?, new, now))
    }

    /// Replaces a finding's body.
    pub fn set_finding_body(
        &self,
        id: dovetail_api::research::FindingId,
        body: String,
        from: &'static str,
    ) -> Pending<()> {
        self.run(move |c, _| research::set_finding_body(c.state()?, id, &body, from))
    }

    /// Stores a report: the task, the round it closes or last closed, the body as written to
    /// `report.md`, and how many claims and figures were flagged.
    pub fn insert_report(
        &self,
        task: TaskId,
        round: Option<i64>,
        body: String,
        flags: (u32, u32),
    ) -> Pending<()> {
        self.run(move |c, now| research::insert_report(c.state()?, task, round, &body, flags, now))
    }

    /// The task's research kind: `question`, `decision` or none.
    pub fn research_kind(&self, task: TaskId) -> Pending<Option<String>> {
        self.run(move |c, _| research::research_kind(c.state()?, task))
    }

    /// Sets the task's research kind (`question`, `decision`, or none).
    pub fn set_research_kind(&self, task: TaskId, kind: Option<String>) -> Pending<()> {
        self.run(move |c, _| research::set_research_kind(c.state()?, task, kind.as_deref()))
    }

    /// Summary counts of every task with research.
    pub fn research_summaries(&self) -> Pending<Vec<research::SummaryRow>> {
        self.run(|c, _| research::load_summaries(c.state()?))
    }

    /// Summary counts of one task.
    pub fn research_summary(&self, task: TaskId) -> Pending<research::SummaryRow> {
        self.run(move |c, _| research::load_summary(c.state()?, task))
    }

    /// One task's research in full.
    pub fn task_research(&self, task: TaskId) -> Pending<research::TaskResearch> {
        self.run(move |c, _| research::load_task(c.state()?, task))
    }

    /// Every task's research in full.
    pub fn all_research(&self) -> Pending<Vec<(TaskId, research::TaskResearch)>> {
        self.run(|c, _| research::load_all(c.state()?))
    }

    // --- another project, read-only (`<leader>fr`) ---

    /// What reading mode's library shows of another project, from its `state.db` at `path`
    /// opened read-only for this request (WAL lets it be read while that project's own Dovetail
    /// writes). `None` when the project has no `state.db` yet. Nothing is written.
    pub fn browse_state(&self, path: PathBuf) -> Pending<Option<BrowsedState>> {
        self.run(move |_, _| {
            let Some(mut conn) = open_read_only(&path)? else {
                return Ok(None);
            };
            Ok(Some(BrowsedState {
                tasks: tasks::load_tasks(&mut conn)?,
                decisions: tasks::load_decisions(&mut conn, None)?,
                summaries: research::load_summaries(&conn)?,
                research: research::load_all(&conn)?,
            }))
        })
    }

    /// Task `task` of another project, with its research, read-only as in
    /// [`Db::browse_state`]; `None` when there is no such task or no `state.db`.
    pub fn browse_task(
        &self,
        path: PathBuf,
        task: TaskId,
    ) -> Pending<Option<(Task, research::TaskResearch)>> {
        self.run(move |_, _| {
            let Some(mut conn) = open_read_only(&path)? else {
                return Ok(None);
            };
            let Some(t) = tasks::load_tasks(&mut conn)?
                .into_iter()
                .find(|t| t.id == task)
            else {
                return Ok(None);
            };
            Ok(Some((t, research::load_task(&conn, task)?)))
        })
    }

    // --- agents and conversations ---

    /// Every agent not yet closed.
    pub fn load_agents(&self) -> Pending<Vec<AgentRow>> {
        self.run(|c, _| sessions::load_agents(c.state()?))
    }

    /// Inserts or replaces an agent.
    pub fn upsert_agent(&self, row: AgentRow) -> Pending<()> {
        self.run(move |c, _| sessions::upsert_agent(c.state()?, &row))
    }

    /// Marks an agent closed: it no longer loads.
    pub fn close_agent(&self, id: AgentId) -> Pending<()> {
        self.run(move |c, now| sessions::close_agent(c.state()?, &id, now))
    }

    /// Image files named by an agent's conversation entries.
    pub fn agent_attachments(&self, agent: AgentId) -> Pending<Vec<PathBuf>> {
        self.run(move |c, _| sessions::agent_attachments(c.state()?, &agent))
    }

    /// Inserts or replaces a conversation entry.
    pub fn upsert_entry(&self, agent: AgentId, entry: ConversationEntry) -> Pending<()> {
        self.run(move |c, _| sessions::upsert_entry(c.state()?, &agent, &entry))
    }

    /// Up to `limit` entries of `agent` before `before` (or the newest), oldest first, and
    /// whether older ones exist.
    pub fn load_entries(
        &self,
        agent: AgentId,
        before: Option<EntryId>,
        limit: usize,
    ) -> Pending<(Vec<ConversationEntry>, bool)> {
        self.run(move |c, _| sessions::load_entries(c.state()?, &agent, before, limit))
    }

    /// The highest ids in use, so new ones continue after them.
    pub fn max_ids(&self) -> Pending<sessions::MaxIds> {
        self.run(|c, _| sessions::max_ids(c.state()?))
    }

    // --- the slice queue ---

    /// Queues a slice start (its `id` is ignored); resolves to its row id.
    pub fn enqueue_slice(&self, slice: QueuedSlice) -> Pending<i64> {
        self.run(move |c, now| sessions::enqueue_slice(c.state()?, &slice, now))
    }

    /// Removes a queued start (it started, or its task was archived).
    pub fn dequeue_slice(&self, id: i64) -> Pending<()> {
        self.run(move |c, _| sessions::dequeue_slice(c.state()?, id))
    }

    /// The queued starts, oldest first.
    pub fn load_queue(&self) -> Pending<Vec<QueuedSlice>> {
        self.run(|c, _| sessions::load_queue(c.state()?))
    }

    // --- restart state (M10) ---

    /// Inbox items not yet resolved, oldest first.
    pub fn load_open_inbox(&self) -> Pending<Vec<InboxItem>> {
        self.run(|c, _| sessions::load_open_inbox(c.state()?))
    }

    /// Holds a signal for the coordinator until its session next starts.
    pub fn add_note(&self, text: String) -> Pending<()> {
        self.run(move |c, now| sessions::add_note(c.state()?, &text, now))
    }

    /// The held coordinator signals, oldest first.
    pub fn load_notes(&self) -> Pending<Vec<String>> {
        self.run(|c, _| sessions::load_notes(c.state()?))
    }

    /// Forgets the held coordinator signals, once delivered.
    pub fn clear_notes(&self) -> Pending<()> {
        self.run(|c, _| sessions::clear_notes(c.state()?))
    }

    /// Records that `agent` created to-do item `item` of native list `list` (M11).
    pub fn set_todo_owner(&self, list: String, item: String, agent: AgentId) -> Pending<()> {
        self.run(move |c, _| sessions::set_todo_owner(c.state()?, &list, &item, &agent.0))
    }

    /// Every recorded to-do owner: `(list id, item id, agent)`.
    pub fn load_todo_owners(&self) -> Pending<Vec<(String, String, AgentId)>> {
        self.run(|c, _| sessions::load_todo_owners(c.state()?))
    }

    /// Forgets the owners of a deleted native list.
    pub fn forget_todo_list(&self, list: String) -> Pending<()> {
        self.run(move |c, _| sessions::forget_todo_list(c.state()?, &list))
    }

    /// Records that the user kept the unowned worktree at `path`.
    pub fn keep_worktree(&self, path: PathBuf) -> Pending<()> {
        self.run(move |c, now| sessions::keep_worktree(c.state()?, &path, now))
    }

    /// Unowned worktrees the user chose to keep, by full path.
    pub fn kept_worktrees(&self) -> Pending<Vec<PathBuf>> {
        self.run(|c, _| sessions::kept_worktrees(c.state()?))
    }

    // --- inbox, activity, messages ---

    /// Records an inbox item.
    pub fn insert_inbox(&self, item: InboxItem) -> Pending<()> {
        self.run(move |c, _| sessions::insert_inbox(c.state()?, &item))
    }

    /// Records how an inbox item left Needs you.
    pub fn resolve_inbox(&self, id: InboxId, answer: Option<String>) -> Pending<()> {
        self.run(move |c, now| sessions::resolve_inbox(c.state()?, id, answer.as_deref(), now))
    }

    /// Records an activity item.
    pub fn add_activity(&self, item: ActivityItem) -> Pending<()> {
        self.run(move |c, _| sessions::add_activity(c.state()?, &item))
    }

    /// Records a message between agents.
    pub fn add_message(&self, message: AgentMessage, msg_id: Option<String>) -> Pending<()> {
        self.run(move |c, _| sessions::add_message(c.state()?, &message, msg_id.as_deref()))
    }
}

pub use sessions::MaxIds;

/// Unix milliseconds now.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Image files named by the conversations of agents that are not archived, in every project's
/// `state.db` on disk, so the startup sweep of `attachments/` (shared by all projects) keeps
/// them. Opens each file read-only on its own connection; `None` when any of them cannot be read,
/// so a sweep that cannot see everything does not run.
pub fn live_attachments(paths: &Paths) -> Option<HashSet<PathBuf>> {
    use rusqlite::OpenFlags;
    let mut keep = HashSet::new();
    let Ok(projects) = std::fs::read_dir(paths.data.join("projects")) else {
        return Some(keep);
    };
    for project in projects.flatten() {
        let file = project.path().join("state.db");
        if !file.exists() {
            continue;
        }
        let conn = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        keep.extend(sessions::live_attachments(&conn).ok()?);
    }
    Some(keep)
}

/// Another project's state, as [`Db::browse_state`] reads it.
#[derive(Debug, Default)]
pub struct BrowsedState {
    /// Every task, archived ones included.
    pub tasks: Vec<Task>,
    /// The decision log, oldest first.
    pub decisions: Vec<Decision>,
    /// Research counts per task.
    pub summaries: Vec<research::SummaryRow>,
    /// Every task's research in full.
    pub research: Vec<(TaskId, research::TaskResearch)>,
}

/// Opens another project's `state.db` read-only, on the DB thread, for one request; `None` when
/// it does not exist. Its schema must be this build's: an older one is upgraded only by opening
/// that project, which this never does.
fn open_read_only(path: &Path) -> Result<Option<Connection>, DbError> {
    use rusqlite::OpenFlags;
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let found: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let supported = STATE_MIGRATIONS.len() as u32;
    if found > supported {
        return Err(DbError::SchemaTooNew {
            path: path.to_owned(),
            found,
            supported,
        });
    }
    if found < supported {
        return Err(DbError::Refused(format!(
            "{} was last opened by an older Dovetail: open that project once (<leader>pp) so \
             it is brought up to date, then browse it",
            path.display()
        )));
    }
    Ok(Some(conn))
}

fn open_connection(path: &Path, migrations: &[Migration]) -> Result<Connection, DbError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| DbError::Io {
            path: dir.to_owned(),
            source,
        })?;
    }
    let mut conn = Connection::open(path)?;
    // A running app and `dovetail add` may write at the same time; wait rather than fail.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn, path, migrations)?;
    Ok(conn)
}

/// Applies the steps after the database's `user_version`, each in its own transaction. A database
/// from a newer build is refused rather than touched.
fn migrate(conn: &mut Connection, path: &Path, steps: &[Migration]) -> Result<(), DbError> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let supported = steps.len() as u32;
    if current > supported {
        return Err(DbError::SchemaTooNew {
            path: path.to_owned(),
            found: current,
            supported,
        });
    }
    for (index, (_, sql)) in steps.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mechanical rule (CODING_STANDARDS.md "Architecture"; retro 2026-11-09): every migrations
    /// directory holds `NNNN_slug.sql` files numbered 0001..N with no gaps or duplicates, and its
    /// list in this file names exactly those files in that order. Fix a failure by renumbering the files or the `migration!` list.
    #[test]
    fn migrations_are_contiguous_and_listed_in_order() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db/migrations");
        let lists = [
            ("registry", REGISTRY_MIGRATIONS),
            ("state", STATE_MIGRATIONS),
        ];
        let mut dirs: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        dirs.sort();
        let mut known: Vec<String> = lists.iter().map(|(db, _)| (*db).to_owned()).collect();
        known.sort();
        assert_eq!(
            dirs, known,
            "a migrations directory has no list in db.rs (or vice versa)"
        );
        for (db, list) in lists {
            let mut files: Vec<String> = std::fs::read_dir(root.join(db))
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            files.sort();
            for (index, file) in files.iter().enumerate() {
                let (number, slug) = file
                    .strip_suffix(".sql")
                    .and_then(|stem| stem.split_once('_'))
                    .unwrap_or_else(|| panic!("{db}/{file}: name it NNNN_slug.sql"));
                assert!(
                    number.len() == 4
                        && number.bytes().all(|b| b.is_ascii_digit())
                        && !slug.is_empty(),
                    "{db}/{file}: name it NNNN_slug.sql"
                );
                assert_eq!(
                    number.parse::<usize>().unwrap(),
                    index + 1,
                    "{db}/{file}: migrations must be numbered 0001..N with no gaps or duplicates"
                );
            }
            let expected: Vec<String> = files.iter().map(|f| format!("{db}/{f}")).collect();
            let listed: Vec<&str> = list.iter().map(|(name, _)| *name).collect();
            assert_eq!(
                listed, expected,
                "the {db} migration list in db.rs must name exactly the files in db/migrations/{db}, in order"
            );
        }
    }

    fn paths(dir: &Path) -> Paths {
        Paths {
            data: dir.join("data"),
            config: dir.join("config"),
            state: dir.join("state"),
            runtime: dir.join("run"),
        }
    }

    #[test]
    fn live_attachments_skip_archived_agents_across_projects() {
        use dovetail_api::{
            agent::AgentKind,
            conversation::{Attachment, EntryKind, ImageType},
        };

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let attachment = |name: &str| Attachment {
            path: paths.attachments().join(name),
            media_type: ImageType::Png,
            size: 1,
        };
        let row = |id: &str| AgentRow {
            agent: crate::service::new_agent(AgentId(id.into()), id.into(), AgentKind::Slice, None),
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        };
        let entry = |n: u64, name: &str| ConversationEntry {
            id: EntryId(n),
            at_ms: 0,
            kind: EntryKind::User,
            text: "look".into(),
            attachments: vec![attachment(name)],
            effort: None,
        };
        for (project, live, archived) in [
            ("p", "live-p.png", "gone-p.png"),
            ("q", "live-q.png", "gone-q.png"),
        ] {
            let db = Db::open(&paths, Some(project)).unwrap();
            let (a, b) = (AgentId("a".into()), AgentId("b".into()));
            db.upsert_agent(row("a")).wait().unwrap();
            db.upsert_agent(row("b")).wait().unwrap();
            db.upsert_entry(a.clone(), entry(1, live)).wait().unwrap();
            db.upsert_entry(b.clone(), entry(2, archived))
                .wait()
                .unwrap();
            assert_eq!(
                db.agent_attachments(b.clone()).wait().unwrap(),
                vec![paths.attachments().join(archived)]
            );
            db.close_agent(b).wait().unwrap();
        }
        let keep = live_attachments(&paths).unwrap();
        assert_eq!(
            keep,
            HashSet::from([
                paths.attachments().join("live-p.png"),
                paths.attachments().join("live-q.png")
            ])
        );
    }

    #[test]
    fn agent_messages_and_old_notices_both_load() {
        use dovetail_api::{
            agent::AgentKind,
            conversation::{EntryKind, agent_message_text},
        };

        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&paths(dir.path()), Some("p")).unwrap();
        let agent = AgentId("a".into());
        db.upsert_agent(AgentRow {
            agent: crate::service::new_agent(agent.clone(), "a".into(), AgentKind::Slice, None),
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        })
        .wait()
        .unwrap();
        let entry = |n: u64, kind, text: String| ConversationEntry {
            id: EntryId(n),
            at_ms: 0,
            kind,
            text,
            attachments: Vec::new(),
            effort: None,
        };
        // The old build stored a peer message as a notice.
        let old = entry(1, EntryKind::Notice, "Message from x: y".into());
        let new = entry(
            2,
            EntryKind::AgentMessage,
            agent_message_text("x", true, "y"),
        );
        db.upsert_entry(agent.clone(), old.clone()).wait().unwrap();
        db.upsert_entry(agent.clone(), new.clone()).wait().unwrap();
        let (loaded, _) = db.load_entries(agent, None, 10).wait().unwrap();
        assert_eq!(loaded, vec![old, new]);
    }

    /// Every state migration, in order, from an empty file: the schema a fresh install gets.
    #[test]
    fn a_state_db_built_from_empty_has_the_final_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let conn = open_connection(&path, STATE_MIGRATIONS).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, STATE_MIGRATIONS.len());

        let tables: std::collections::BTreeSet<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for table in [
            "agents",
            "conversation_entries",
            "inbox_items",
            "activity",
            "messages",
            "slice_queue",
            "coordinator_notes",
            "kept_worktrees",
            "tasks",
            "task_deps",
            "task_related",
            "decisions",
            "decision_evidence",
            "todo_items",
            "maps",
            "map_versions",
            "research_rounds",
            "findings",
            "sources",
            "finding_sources",
            "reports",
            "unfiled_findings",
            "unfiled_finding_sources",
            "efforts",
            "criteria",
        ] {
            assert!(tables.contains(table), "missing table {table}: {tables:?}");
        }

        let columns = |table: &str| -> Vec<String> {
            conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        // Columns added by later `ALTER TABLE` steps, which only a full run produces.
        let agents = columns("agents");
        for column in ["progress", "subagent_tokens", "claude_home", "takeover"] {
            assert!(agents.iter().any(|c| c == column), "agents.{column}");
        }
        assert!(
            columns("conversation_entries")
                .iter()
                .any(|c| c == "attachments")
        );
        let tasks = columns("tasks");
        for column in ["research_kind", "altitude", "effort_id", "criterion_id"] {
            assert!(tasks.iter().any(|c| c == column), "tasks.{column}");
        }
        assert!(
            columns("conversation_entries")
                .iter()
                .any(|c| c == "effort_id")
        );
        assert!(columns("research_rounds").iter().any(|c| c == "queued"));
        assert!(!columns("map_versions").is_empty());
        assert!(!columns("maps").is_empty());
    }

    /// The same for the registry, which has its own list.
    #[test]
    fn a_registry_db_built_from_empty_is_at_the_last_version() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_connection(&dir.path().join("registry.db"), REGISTRY_MIGRATIONS).unwrap();
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, REGISTRY_MIGRATIONS.len());
        let column = |table: &str, name: &str| -> i64 {
            conn.query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                [name],
                |row| row.get(0),
            )
            .unwrap()
        };
        // Migration 0004 (ADR 0021): the kind is gone, the account and the repos table are in.
        assert_eq!(column("projects", "kind"), 0);
        assert_eq!(column("projects", "account"), 1);
        assert_eq!(column("repos", "git_common_dir"), 1);
    }

    #[test]
    fn todo_owners_migrate_up_from_empty_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open(&paths, Some("p")).unwrap();
        assert!(db.load_todo_owners().wait().unwrap().is_empty());
        let agent = AgentId("slice-a".into());
        db.set_todo_owner("l1".into(), "1".into(), agent.clone())
            .wait()
            .unwrap();
        db.set_todo_owner("l2".into(), "1".into(), agent.clone())
            .wait()
            .unwrap();
        // A second claim on the same item replaces the first.
        let sub = AgentId("slice-a:toolu_1".into());
        db.set_todo_owner("l1".into(), "1".into(), sub.clone())
            .wait()
            .unwrap();
        db.forget_todo_list("l2".into()).wait().unwrap();
        drop(db);
        let reopened = Db::open(&paths, Some("p")).unwrap();
        assert_eq!(
            reopened.load_todo_owners().wait().unwrap(),
            vec![("l1".into(), "1".into(), sub)]
        );
    }

    /// Migration 0017 on a database from empty: Unsorted is there, and a new task lands in it.
    #[test]
    fn a_new_project_starts_with_the_unsorted_effort() {
        use dovetail_api::effort::{
            EffortKind, EffortStatus, UNSORTED_EFFORT, UNSORTED_EFFORT_NAME,
        };

        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&paths(dir.path()), Some("p")).unwrap();
        let efforts = db.load_efforts().wait().unwrap();
        assert_eq!(efforts.len(), 1);
        assert_eq!(efforts[0].id, UNSORTED_EFFORT);
        assert_eq!(efforts[0].name, UNSORTED_EFFORT_NAME);
        assert_eq!(efforts[0].kind, EffortKind::Cleanup);
        assert_eq!(efforts[0].status, EffortStatus::Open);
        assert!(efforts[0].criteria.is_empty());

        let created = db.create_task(new_task("first"), false).wait().unwrap();
        assert_eq!(created[0].effort, UNSORTED_EFFORT);
        assert_eq!(created[0].criterion, None);
    }

    /// ADR 0021: state migration 0019 from version 18. Agents and queued starts are kept; none
    /// has a repo yet (read as the project's first), and a stored one round-trips.
    #[test]
    fn agent_repos_migrate_up_from_version_18() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let path = paths.state_db("p");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut old = Connection::open(&path).unwrap();
        migrate(&mut old, &path, &STATE_MIGRATIONS[..18]).unwrap();
        old.execute_batch(
            "INSERT INTO agents (id, kind, name, status, permission_mode, created_ms)
               VALUES ('slice-a', 'slice', 'a', 'idle', 'auto', 1);
             INSERT INTO slice_queue (name, brief, created_ms) VALUES ('b', 'go', 2);",
        )
        .unwrap();
        drop(old);

        let db = Db::open(&paths, Some("p")).unwrap();
        let mut agents = db.load_agents().wait().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].repo, None);
        let queue = db.load_queue().wait().unwrap();
        assert_eq!(
            (queue[0].name.as_str(), queue[0].repo.as_deref()),
            ("b", None)
        );

        agents[0].repo = Some(dovetail_api::project::RepoId(7));
        db.upsert_agent(agents[0].clone()).wait().unwrap();
        db.enqueue_slice(QueuedSlice {
            id: 0,
            task: None,
            name: "c".into(),
            brief: "go".into(),
            by_coordinator: false,
            repo: Some("web".into()),
            model_choice: Some(ModelChoice {
                model: "haiku".into(),
                effort: Some(dovetail_api::model::ReasoningEffort::Low),
            }),
        })
        .wait()
        .unwrap();
        drop(db);
        let db = Db::open(&paths, Some("p")).unwrap();
        assert_eq!(
            db.load_agents().wait().unwrap()[0].repo,
            Some(dovetail_api::project::RepoId(7))
        );
        assert_eq!(
            db.load_queue().wait().unwrap()[1].repo.as_deref(),
            Some("web")
        );
        assert_eq!(
            db.load_queue().wait().unwrap()[1].model_choice,
            Some(ModelChoice {
                model: "haiku".into(),
                effort: Some(dovetail_api::model::ReasoningEffort::Low)
            })
        );
    }

    /// Task #42: state migration 0022 from version 21. A queued start made before it has no
    /// model choice, and one queued after it keeps its own.
    #[test]
    fn queued_starts_migrate_up_from_version_21_with_no_model_choice() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let path = paths.state_db("p");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut old = Connection::open(&path).unwrap();
        migrate(&mut old, &path, &STATE_MIGRATIONS[..21]).unwrap();
        old.execute_batch(
            "INSERT INTO slice_queue (name, brief, created_ms) VALUES ('old', 'go', 1);",
        )
        .unwrap();
        drop(old);

        let db = Db::open(&paths, Some("p")).unwrap();
        let queue = db.load_queue().wait().unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].name, "old");
        assert_eq!(queue[0].model_choice, None);
    }

    /// Migration 0017 on a database as version 16 left it: every task goes to Unsorted, chat
    /// lines keep no tag, and nothing else about them changes.
    #[test]
    fn efforts_migrate_up_from_version_16_and_existing_tasks_go_to_unsorted() {
        use dovetail_api::{conversation::EntryKind, effort::UNSORTED_EFFORT, task::TaskStatus};

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let path = paths.state_db("p");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut old = Connection::open(&path).unwrap();
        migrate(&mut old, &path, &STATE_MIGRATIONS[..16]).unwrap();
        let version: u32 = old
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 16);
        old.execute_batch(
            "INSERT INTO tasks (id, title, status, bucket, notes) VALUES
               (4, 'kept', 'in_progress', 'now', 'as it was'),
               (9, 'old', 'archived', NULL, '');
             INSERT INTO conversation_entries (id, agent_id, at_ms, kind, text)
               VALUES (1, 'coordinator', 5, 'user', 'hello');",
        )
        .unwrap();
        drop(old);

        let db = Db::open(&paths, Some("p")).unwrap();
        let tasks = db.load_tasks().wait().unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|t| t.effort == UNSORTED_EFFORT));
        assert!(tasks.iter().all(|t| t.criterion.is_none()));
        assert_eq!(tasks[0].title, "kept");
        assert_eq!(tasks[0].status, TaskStatus::InProgress);
        assert_eq!(tasks[0].notes, "as it was");
        assert_eq!(tasks[1].status, TaskStatus::Archived);

        let efforts = db.load_efforts().wait().unwrap();
        assert_eq!(efforts.len(), 1);
        assert_eq!(efforts[0].id, UNSORTED_EFFORT);

        let (entries, _) = db
            .load_entries(AgentId("coordinator".into()), None, 10)
            .wait()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::User);
        assert_eq!(entries[0].effort, None);

        // Opening again changes nothing: the step ran once.
        drop(db);
        let again = Db::open(&paths, Some("p")).unwrap();
        assert_eq!(again.load_efforts().wait().unwrap().len(), 1);
    }

    fn new_task(title: &str) -> NewTask {
        NewTask {
            title: title.into(),
            p_level: None,
            bucket: None,
            notes: String::new(),
            follow_up_of: None,
            pr: None,
            related: Vec::new(),
            effort: None,
            criterion: None,
        }
    }

    fn new_effort(name: &str) -> NewEffort {
        NewEffort {
            name: name.into(),
            kind: dovetail_api::effort::EffortKind::Feature,
            question_or_goal: String::new(),
        }
    }

    #[test]
    fn effort_names_are_unique_and_unsorted_is_fixed() {
        use dovetail_api::effort::{EffortStatus, UNSORTED_EFFORT};

        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&paths(dir.path()), Some("p")).unwrap();
        let effort = db
            .create_effort(new_effort("guest-checkout"))
            .wait()
            .unwrap();
        assert_eq!(effort.name, "guest-checkout");
        assert!(matches!(
            db.create_effort(new_effort("guest-checkout")).wait(),
            Err(DbError::Refused(_))
        ));
        assert!(matches!(
            db.create_effort(new_effort("")).wait(),
            Err(DbError::Refused(_))
        ));
        // Renaming to its own name is fine; to another's is not.
        db.rename_effort(effort.id, "guest-checkout".into())
            .wait()
            .unwrap();
        assert!(matches!(
            db.rename_effort(effort.id, "unsorted".into()).wait(),
            Err(DbError::Refused(_))
        ));
        assert!(matches!(
            db.rename_effort(UNSORTED_EFFORT, "misc".into()).wait(),
            Err(DbError::Refused(_))
        ));
        assert!(matches!(
            db.update_effort(
                UNSORTED_EFFORT,
                vec![EffortChange::Status(EffortStatus::Closed)]
            )
            .wait(),
            Err(DbError::Refused(_))
        ));
        assert!(matches!(
            db.rename_effort(dovetail_api::effort::EffortId(99), "x".into())
                .wait(),
            Err(DbError::NotFound(_))
        ));
    }

    #[test]
    fn a_criterion_goes_open_claimed_met_and_can_be_sent_back() {
        use dovetail_api::effort::CriterionState;

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open(&paths, Some("p")).unwrap();
        let effort = db
            .create_effort(new_effort("guest-checkout"))
            .wait()
            .unwrap();
        let effort = db
            .add_criterion(effort.id, " pay as guest end to end ".into())
            .wait()
            .unwrap();
        let id = effort.criteria[0].id;
        assert_eq!(effort.criteria[0].text, "pay as guest end to end");
        assert_eq!(effort.criteria[0].state, CriterionState::Open);
        assert_eq!(effort.criteria[0].evidence, None);

        // Nothing to send back yet, and no claim without evidence.
        assert!(matches!(
            db.step_criterion(id, CriterionStep::Return).wait(),
            Err(DbError::Refused(_))
        ));
        let empty = CriterionStep::Claim {
            evidence: "  ".into(),
        };
        assert!(matches!(
            db.step_criterion(id, empty).wait(),
            Err(DbError::Refused(_))
        ));

        let claim = |evidence: &str| CriterionStep::Claim {
            evidence: evidence.into(),
        };
        let claimed = db.step_criterion(id, claim("PR #41")).wait().unwrap();
        assert_eq!(claimed.criteria[0].state, CriterionState::Claimed);
        assert_eq!(claimed.criteria[0].evidence.as_deref(), Some("PR #41"));

        // Sent back: open again, the evidence stays on record until the next claim.
        let returned = db.step_criterion(id, CriterionStep::Return).wait().unwrap();
        assert_eq!(returned.criteria[0].state, CriterionState::Open);
        assert_eq!(returned.criteria[0].evidence.as_deref(), Some("PR #41"));

        db.step_criterion(id, claim("PR #44")).wait().unwrap();
        let met = db
            .step_criterion(id, CriterionStep::Approve)
            .wait()
            .unwrap();
        assert_eq!(met.criteria[0].state, CriterionState::Met);
        assert_eq!(met.criteria[0].evidence.as_deref(), Some("PR #44"));
        // A met criterion is not claimed again.
        assert!(matches!(
            db.step_criterion(id, claim("again")).wait(),
            Err(DbError::Refused(_))
        ));

        // It is all still there after a reopen.
        drop(db);
        let reopened = Db::open(&paths, Some("p")).unwrap();
        let efforts = reopened.load_efforts().wait().unwrap();
        assert_eq!(efforts[1], met);
    }

    #[test]
    fn a_task_moves_between_efforts_and_serves_only_its_own_efforts_criteria() {
        use dovetail_api::effort::{EffortStatus, UNSORTED_EFFORT};

        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&paths(dir.path()), Some("p")).unwrap();
        let checkout = db
            .create_effort(new_effort("guest-checkout"))
            .wait()
            .unwrap();
        let checkout = db
            .add_criterion(checkout.id, "pay as guest".into())
            .wait()
            .unwrap();
        let criterion = checkout.criteria[0].id;
        let other = db.create_effort(new_effort("providers")).wait().unwrap();
        let task = db.create_task(new_task("form"), false).wait().unwrap()[0].id;

        let moved = db
            .move_task(task, checkout.id, Some(criterion))
            .wait()
            .unwrap();
        assert_eq!(moved[0].effort, checkout.id);
        assert_eq!(moved[0].criterion, Some(criterion));
        assert_eq!(db.load_tasks().wait().unwrap()[0], moved[0]);

        // A criterion of another effort is refused, and the task stays where it was.
        assert!(matches!(
            db.move_task(task, other.id, Some(criterion)).wait(),
            Err(DbError::Refused(_))
        ));
        assert_eq!(db.load_tasks().wait().unwrap()[0].effort, checkout.id);

        // Moving on with no criterion clears it.
        let moved = db.move_task(task, other.id, None).wait().unwrap();
        assert_eq!((moved[0].effort, moved[0].criterion), (other.id, None));

        // A closed effort takes no tasks, by move or at creation.
        db.update_effort(
            checkout.id,
            vec![EffortChange::Status(EffortStatus::Closed)],
        )
        .wait()
        .unwrap();
        assert!(matches!(
            db.move_task(task, checkout.id, None).wait(),
            Err(DbError::Refused(_))
        ));
        let mut into_closed = new_task("late");
        into_closed.effort = Some(checkout.id);
        assert!(matches!(
            db.create_task(into_closed, false).wait(),
            Err(DbError::Refused(_))
        ));
        assert!(matches!(
            db.move_task(TaskId(99), UNSORTED_EFFORT, None).wait(),
            Err(DbError::NotFound(_))
        ));

        // A task can be created straight into an effort, serving a criterion.
        let mut placed = new_task("tax");
        placed.effort = Some(other.id);
        let created = db.create_task(placed, false).wait().unwrap();
        assert_eq!(created[0].effort, other.id);
    }

    #[test]
    fn an_entrys_effort_tag_is_stored_and_read_back() {
        use dovetail_api::{agent::AgentKind, conversation::EntryKind};

        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&paths(dir.path()), Some("p")).unwrap();
        let effort = db.create_effort(new_effort("providers")).wait().unwrap();
        let agent = AgentId("coordinator".into());
        db.upsert_agent(AgentRow {
            agent: crate::service::new_agent(
                agent.clone(),
                "coordinator".into(),
                AgentKind::Coordinator,
                None,
            ),
            session_id: None,
            transcript_path: None,
            task_list_id: None,
            created_ms: 0,
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        })
        .wait()
        .unwrap();
        let entry = |n: u64, effort| ConversationEntry {
            id: EntryId(n),
            at_ms: 0,
            kind: EntryKind::User,
            text: "hi".into(),
            attachments: Vec::new(),
            effort,
        };
        let tagged = entry(1, Some(effort.id));
        let plain = entry(2, None);
        db.upsert_entry(agent.clone(), tagged.clone())
            .wait()
            .unwrap();
        db.upsert_entry(agent.clone(), plain.clone())
            .wait()
            .unwrap();
        let (loaded, _) = db.load_entries(agent, None, 10).wait().unwrap();
        assert_eq!(loaded, vec![tagged, plain]);
    }

    #[test]
    fn maps_migrate_up_from_the_previous_version_and_keep_versions() {
        use dovetail_api::map::{MapContent, MapKind, Node, NodeKind};

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        // A state.db as the build before maps left it, with a task in it.
        let path = paths.state_db("p");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut old = Connection::open(&path).unwrap();
        let before_maps = STATE_MIGRATIONS.len() - 1;
        migrate(&mut old, &path, &STATE_MIGRATIONS[..before_maps]).unwrap();
        old.execute(
            "INSERT INTO tasks (id, title, status) VALUES (4, 'kept', 'todo')",
            [],
        )
        .unwrap();
        drop(old);

        let db = Db::open(&paths, Some("p")).unwrap();
        assert!(db.load_maps().wait().unwrap().is_empty());
        let content = |title: &str, node: &str| MapContent {
            title: title.into(),
            kind: MapKind::Structure,
            nodes: vec![Node {
                id: NodeId(node.into()),
                kind: NodeKind::Part,
                label: "l".into(),
                description: "d".into(),
                file: None,
                pseudocode: None,
            }],
            groups: vec![],
            connections: vec![],
            flows: vec![],
            lanes: vec![],
        };
        let draw = |title: &str, node: &str, id: Option<&str>| NewMapVersion {
            id: id.map(|i| MapId(i.into())),
            content: content(title, node),
            task: Some(TaskId(4)),
            drawn_by: AgentId("slice-a".into()),
        };
        let Drawn::Saved(first) = db.save_map(draw("Shop: core!", "a", None)).wait().unwrap()
        else {
            panic!("saved")
        };
        assert_eq!((first.id.0.as_str(), first.version), ("shop-core", 1));
        assert_eq!(first.task, Some(TaskId(4)));
        db.save_map(draw("Shop", "b", Some("shop-core")))
            .wait()
            .unwrap();
        drop(db);

        let reopened = Db::open(&paths, Some("p")).unwrap();
        let shelf = reopened.load_maps().wait().unwrap();
        assert_eq!(shelf.len(), 1);
        assert_eq!((shelf[0].version, shelf[0].title.as_str()), (2, "Shop"));
        let v1 = reopened
            .load_map(MapId("shop-core".into()), Some(1))
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(v1.content, content("Shop: core!", "a"));
        // Lanes may link only the latest version's nodes.
        assert_eq!(
            reopened.structure_nodes().wait().unwrap(),
            [NodeId("b".into())].into()
        );
    }

    #[test]
    fn unregistered_state_is_written_on_registration() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open_unregistered(&paths).unwrap();
        let new = NewTask {
            title: "kept".into(),
            p_level: None,
            bucket: None,
            notes: String::new(),
            follow_up_of: None,
            pr: None,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        };
        db.create_task(new, false).wait().unwrap();
        assert!(!paths.state_db("p").exists());
        db.persist_state(paths.state_db("p")).wait().unwrap();
        drop(db);
        let reopened = Db::open(&paths, Some("p")).unwrap();
        let tasks = reopened.load_tasks().wait().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "kept");
    }

    #[test]
    fn registry_migrates_to_optional_git_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open(&paths, None).unwrap();
        let folder = db
            .register_project("notes".into(), "notes".into(), folder_repo("/x/notes"))
            .wait()
            .unwrap();
        let again = db
            .register_project("notes".into(), "notes".into(), folder_repo("/x/notes"))
            .wait()
            .unwrap();
        assert_eq!(folder, again);
        let repo = db
            .register_project(
                "notes".into(),
                "notes".into(),
                NewRepo {
                    name: "notes".into(),
                    path: "/y/notes".into(),
                    git_common_dir: Some("/y/notes/.git".into()),
                },
            )
            .wait()
            .unwrap();
        assert_eq!(repo.id, "notes-2");
    }

    fn folder_repo(path: &str) -> NewRepo {
        NewRepo {
            name: "notes".into(),
            path: path.into(),
            git_common_dir: None,
        }
    }

    /// ADR 0021: registry migration 0004 from version 3. Every project is kept; a code project
    /// and a folder project each get one repo named after the project, a research project none.
    #[test]
    fn an_existing_registry_migrates_to_repos() {
        use dovetail_api::project::{ProjectKind, RepoId};

        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let path = paths.registry_db();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut old = Connection::open(&path).unwrap();
        let before = REGISTRY_MIGRATIONS.len() - 1;
        migrate(&mut old, &path, &REGISTRY_MIGRATIONS[..before]).unwrap();
        old.execute_batch(
            "INSERT INTO projects (id, name, kind, repo_path, git_common_dir, last_opened_ms) \
               VALUES ('dovetail', 'Dovetail', 'code', '/src/dovetail', '/src/dovetail/.git', 5);
             INSERT INTO projects (id, name, kind, repo_path, git_common_dir, last_opened_ms) \
               VALUES ('notes', 'notes', 'code', '/x/notes', NULL, 4);
             INSERT INTO projects (id, name, kind, repo_path, git_common_dir, last_opened_ms) \
               VALUES ('market', 'Market', 'research', NULL, NULL, 3);",
        )
        .unwrap();
        drop(old);

        let db = Db::open(&paths, None).unwrap();
        let listed = db.list_projects().wait().unwrap();
        let ids: Vec<&str> = listed.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["dovetail", "notes", "market"]);
        let dovetail = &listed[0];
        assert_eq!(
            (dovetail.name.as_str(), dovetail.account.as_deref()),
            ("Dovetail", None)
        );
        assert_eq!(dovetail.last_opened_ms, Some(5));
        assert_eq!(dovetail.repos.len(), 1);
        assert_eq!(dovetail.repos[0].name, "dovetail");
        assert_eq!(dovetail.repos[0].path, Path::new("/src/dovetail"));
        assert_eq!(
            dovetail.repos[0].git_common_dir.as_deref(),
            Some(Path::new("/src/dovetail/.git"))
        );
        assert_eq!(dovetail.kind(), ProjectKind::Code);
        assert_eq!(listed[1].repos[0].git_common_dir, None);
        assert_eq!(listed[1].kind(), ProjectKind::Code);
        assert!(listed[2].repos.is_empty());
        assert_eq!(listed[2].kind(), ProjectKind::Research);
        // Lookups go through repos.
        let found = db.find_project("/src/dovetail/.git".into()).wait().unwrap();
        assert_eq!(found.map(|p| p.id), Some("dovetail".into()));
        let found = db.find_folder("/x/notes".into()).wait().unwrap();
        assert_eq!(found.map(|p| p.id), Some("notes".into()));
        // A git dir stays unique across projects.
        let again = db
            .add_repo(
                "notes".into(),
                NewRepo {
                    name: "dt".into(),
                    path: "/elsewhere".into(),
                    git_common_dir: Some("/src/dovetail/.git".into()),
                },
            )
            .wait();
        assert!(matches!(again, Err(DbError::Refused(_))), "{again:?}");
        let removed = db
            .remove_repo("dovetail".into(), dovetail.repos[0].id)
            .wait()
            .unwrap();
        assert!(removed.repos.is_empty());
        assert!(
            db.remove_repo("dovetail".into(), RepoId(999))
                .wait()
                .is_err()
        );
        drop(db);
        // Opening again changes nothing.
        let db = Db::open(&paths, None).unwrap();
        assert_eq!(db.list_projects().wait().unwrap().len(), 3);
    }

    /// ADR 0021: projects are created with an account and repos, renamed, and given another
    /// account; a research project's name is unique among projects with no repos.
    #[test]
    fn projects_are_created_renamed_and_given_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let db = Db::open(&paths, None).unwrap();
        let api = NewRepo {
            name: "shop-api".into(),
            path: "/code/shop-api".into(),
            git_common_dir: Some("/code/shop-api/.git".into()),
        };
        let shop = db
            .create_project("shop".into(), "Shop".into(), Some("work".into()), Some(api))
            .wait()
            .unwrap();
        assert_eq!(shop.account.as_deref(), Some("work"));
        let web = NewRepo {
            name: "shop-web".into(),
            path: "/code/shop-web".into(),
            git_common_dir: Some("/code/shop-web/.git".into()),
        };
        let shop = db.add_repo("shop".into(), web.clone()).wait().unwrap();
        assert_eq!(shop.repos.len(), 2);
        // The same name twice in one project is refused.
        let clash = NewRepo {
            path: "/code/other".into(),
            git_common_dir: Some("/code/other/.git".into()),
            ..web
        };
        assert!(db.add_repo("shop".into(), clash).wait().is_err());
        let renamed = db
            .rename_project("shop".into(), "Shop front".into())
            .wait()
            .unwrap();
        assert_eq!(
            (renamed.id.as_str(), renamed.name.as_str()),
            ("shop", "Shop front")
        );
        let cleared = db.set_project_account("shop".into(), None).wait().unwrap();
        assert_eq!(cleared.account, None);
        let again = db
            .create_project("shop".into(), "Shop".into(), None, None)
            .wait()
            .unwrap();
        assert_eq!(again.id, "shop-2");
        assert!(again.is_research());
        assert!(
            db.register_research("x".into(), "Shop".into())
                .wait()
                .unwrap()
                .is_none()
        );
    }
}

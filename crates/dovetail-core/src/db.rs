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

mod maps;
mod registry;
mod sessions;
mod tasks;

pub use maps::{Drawn, NewMapVersion};
pub use sessions::{AgentRow, QueuedSlice};

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};

use dovetail_api::map::{Map, MapId, MapSummary, NodeId};
use dovetail_api::{
    ActivityItem, AgentId, AgentMessage, ConversationEntry, Decision, DecisionAuthor, EntryId,
    InboxId, InboxItem, NewDecision, NewTask, Task, TaskChange, TaskId,
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

    /// Adds a project to the registry, or returns the one with this git common dir (or, without
    /// git, this folder). The id is `slug`, numbered (`slug-2`, ...) when taken.
    pub fn register_project(
        &self,
        slug: String,
        name: String,
        repo_path: PathBuf,
        git_common_dir: Option<PathBuf>,
    ) -> Pending<Project> {
        self.run(move |c, now| {
            registry::register(
                &mut c.registry,
                &slug,
                &name,
                &repo_path,
                git_common_dir.as_deref(),
                now,
            )
        })
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

    /// Records the git common dir of a folder project that has become a repo.
    pub fn set_git_dir(&self, id: String, git_common_dir: PathBuf) -> Pending<Project> {
        self.run(move |c, _| registry::set_git_dir(&c.registry, &id, &git_common_dir))
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
        self.run(move |c, now| tasks::log_decision(c.state()?, new, by, now))
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

    /// Queues a slice start; resolves to its row id.
    pub fn enqueue_slice(
        &self,
        task: Option<TaskId>,
        name: String,
        brief: String,
        by_coordinator: bool,
    ) -> Pending<i64> {
        self.run(move |c, now| {
            sessions::enqueue_slice(c.state()?, task, &name, &brief, by_coordinator, now)
        })
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
            .register_project("notes".into(), "notes".into(), "/x/notes".into(), None)
            .wait()
            .unwrap();
        let again = db
            .register_project("notes".into(), "notes".into(), "/x/notes".into(), None)
            .wait()
            .unwrap();
        assert_eq!(folder, again);
        let repo = db
            .register_project(
                "notes".into(),
                "notes".into(),
                "/y/notes".into(),
                Some("/y/notes/.git".into()),
            )
            .wait()
            .unwrap();
        assert_eq!(repo.id, "notes-2");
    }
}

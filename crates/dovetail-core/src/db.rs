//! The single DB thread: typed requests, migrations, `registry.db` and per-project `state.db`.
//!
//! Every SQLite call in the app goes through one thread that owns the connections. Callers hold a
//! cheap, cloneable [`Db`] and get a [`Pending`] back from each request, which they either
//! `.await` (in the core) or [`Pending::wait`] on (in the binary's start-up and `dovetail add`).
//! Migrations are forward-only and tracked with `PRAGMA user_version`: step `n` of a list moves a
//! database from version `n` to `n + 1`.

mod registry;
mod tasks;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll};

use dovetail_api::{Decision, DecisionAuthor, NewDecision, NewTask, Task, TaskChange, TaskId};
use rusqlite::Connection;
use tokio::sync::oneshot;

use crate::paths::Paths;
use crate::projects::Project;

/// Migrations of `registry.db`, in order.
const REGISTRY_MIGRATIONS: &[&str] = &[include_str!("db/migrations/registry/0001_init.sql")];
/// Migrations of a project's `state.db`, in order.
const STATE_MIGRATIONS: &[&str] = &[include_str!("db/migrations/state/0001_init.sql")];

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

type Reply<T> = oneshot::Sender<Result<T, DbError>>;

/// What the DB thread can be asked to do.
enum Request {
    RegisterProject {
        slug: String,
        name: String,
        repo_path: PathBuf,
        git_common_dir: PathBuf,
        reply: Reply<Project>,
    },
    ListProjects {
        reply: Reply<Vec<Project>>,
    },
    FindProject {
        git_common_dir: PathBuf,
        reply: Reply<Option<Project>>,
    },
    TouchProject {
        id: String,
        reply: Reply<()>,
    },
    LoadTasks {
        reply: Reply<Vec<Task>>,
    },
    LoadDecisions {
        reply: Reply<Vec<Decision>>,
    },
    CreateTask {
        new: NewTask,
        coordinator_created: bool,
        /// The new task first, then every task whose relations changed.
        reply: Reply<Vec<Task>>,
    },
    UpdateTask {
        id: TaskId,
        patch: Vec<TaskChange>,
        /// Every task the patch changed.
        reply: Reply<Vec<Task>>,
    },
    ReorderTask {
        id: TaskId,
        delta: i32,
        /// Every task whose position changed.
        reply: Reply<Vec<Task>>,
    },
    LogDecision {
        new: NewDecision,
        by: DecisionAuthor,
        reply: Reply<Decision>,
    },
}

/// A handle to the DB thread. Clone it freely; the thread ends when the last handle is dropped.
#[derive(Debug, Clone)]
pub struct Db {
    requests: mpsc::Sender<Request>,
}

impl Db {
    /// Opens `registry.db` and, when `project` is given, that project's `state.db`, migrating both,
    /// then starts the DB thread.
    pub fn open(paths: &Paths, project: Option<&str>) -> Result<Self, DbError> {
        let registry = open_connection(&paths.registry_db(), REGISTRY_MIGRATIONS)?;
        let state = project
            .map(|id| open_connection(&paths.state_db(id), STATE_MIGRATIONS))
            .transpose()?;
        let (requests, inbox) = mpsc::channel();
        std::thread::Builder::new()
            .name("dovetail-db".into())
            .spawn(move || run(registry, state, inbox))
            .map_err(|source| DbError::Io {
                path: PathBuf::from("dovetail-db"),
                source,
            })?;
        Ok(Self { requests })
    }

    fn send<T>(&self, make: impl FnOnce(Reply<T>) -> Request) -> Pending<T> {
        let (reply, pending) = oneshot::channel();
        if let Err(mpsc::SendError(request)) = self.requests.send(make(reply)) {
            // The thread is gone; resolve the request with an error at once.
            request.fail(DbError::Closed);
        }
        Pending(pending)
    }

    /// Adds a project to the registry, or returns the one with this git common dir. The id is
    /// `slug`, numbered (`slug-2`, ...) when taken by another repo.
    pub fn register_project(
        &self,
        slug: String,
        name: String,
        repo_path: PathBuf,
        git_common_dir: PathBuf,
    ) -> Pending<Project> {
        self.send(|reply| Request::RegisterProject {
            slug,
            name,
            repo_path,
            git_common_dir,
            reply,
        })
    }

    /// Every registered project, most recently opened first.
    pub fn list_projects(&self) -> Pending<Vec<Project>> {
        self.send(|reply| Request::ListProjects { reply })
    }

    /// The project with this git common dir, if registered.
    pub fn find_project(&self, git_common_dir: PathBuf) -> Pending<Option<Project>> {
        self.send(|reply| Request::FindProject {
            git_common_dir,
            reply,
        })
    }

    /// Marks a project as opened now.
    pub fn touch_project(&self, id: String) -> Pending<()> {
        self.send(|reply| Request::TouchProject { id, reply })
    }

    /// Every task of the open project, archived ones included.
    pub fn load_tasks(&self) -> Pending<Vec<Task>> {
        self.send(|reply| Request::LoadTasks { reply })
    }

    /// Every decision of the open project, oldest first.
    pub fn load_decisions(&self) -> Pending<Vec<Decision>> {
        self.send(|reply| Request::LoadDecisions { reply })
    }

    /// Creates a task. Resolves to the new task followed by tasks whose relations changed.
    pub fn create_task(&self, new: NewTask, coordinator_created: bool) -> Pending<Vec<Task>> {
        self.send(|reply| Request::CreateTask {
            new,
            coordinator_created,
            reply,
        })
    }

    /// Applies a patch. Resolves to every task it changed.
    pub fn update_task(&self, id: TaskId, patch: Vec<TaskChange>) -> Pending<Vec<Task>> {
        self.send(|reply| Request::UpdateTask { id, patch, reply })
    }

    /// Moves a task within its bucket. Resolves to every task whose position changed.
    pub fn reorder_task(&self, id: TaskId, delta: i32) -> Pending<Vec<Task>> {
        self.send(|reply| Request::ReorderTask { id, delta, reply })
    }

    /// Appends to a task's decision log.
    pub fn log_decision(&self, new: NewDecision, by: DecisionAuthor) -> Pending<Decision> {
        self.send(|reply| Request::LogDecision { new, by, reply })
    }
}

impl Request {
    /// Answers the request with an error without running it.
    fn fail(self, error: DbError) {
        match self {
            Request::RegisterProject { reply, .. } => drop(reply.send(Err(error))),
            Request::ListProjects { reply } => drop(reply.send(Err(error))),
            Request::FindProject { reply, .. } => drop(reply.send(Err(error))),
            Request::TouchProject { reply, .. } => drop(reply.send(Err(error))),
            Request::LoadTasks { reply } => drop(reply.send(Err(error))),
            Request::LoadDecisions { reply } => drop(reply.send(Err(error))),
            Request::CreateTask { reply, .. } => drop(reply.send(Err(error))),
            Request::UpdateTask { reply, .. } => drop(reply.send(Err(error))),
            Request::ReorderTask { reply, .. } => drop(reply.send(Err(error))),
            Request::LogDecision { reply, .. } => drop(reply.send(Err(error))),
        }
    }
}

/// The DB thread's loop.
fn run(mut registry: Connection, mut state: Option<Connection>, inbox: mpsc::Receiver<Request>) {
    fn project(state: &mut Option<Connection>) -> Result<&mut Connection, DbError> {
        state.as_mut().ok_or(DbError::NoProject)
    }
    for request in inbox {
        let now = now_ms();
        match request {
            Request::RegisterProject {
                slug,
                name,
                repo_path,
                git_common_dir,
                reply,
            } => drop(reply.send(registry::register(
                &mut registry,
                &slug,
                &name,
                &repo_path,
                &git_common_dir,
                now,
            ))),
            Request::ListProjects { reply } => drop(reply.send(registry::list(&registry))),
            Request::FindProject {
                git_common_dir,
                reply,
            } => drop(reply.send(registry::find(&registry, &git_common_dir))),
            Request::TouchProject { id, reply } => {
                drop(reply.send(registry::touch(&registry, &id, now)));
            }
            Request::LoadTasks { reply } => {
                drop(reply.send(project(&mut state).and_then(tasks::load_tasks)));
            }
            Request::LoadDecisions { reply } => {
                drop(reply.send(project(&mut state).and_then(tasks::load_decisions)));
            }
            Request::CreateTask {
                new,
                coordinator_created,
                reply,
            } => drop(
                reply.send(
                    project(&mut state)
                        .and_then(|c| tasks::create_task(c, new, coordinator_created, now)),
                ),
            ),
            Request::UpdateTask { id, patch, reply } => drop(
                reply.send(project(&mut state).and_then(|c| tasks::update_task(c, id, patch, now))),
            ),
            Request::ReorderTask { id, delta, reply } => {
                drop(reply.send(project(&mut state).and_then(|c| tasks::reorder(c, id, delta))));
            }
            Request::LogDecision { new, by, reply } => drop(
                reply.send(project(&mut state).and_then(|c| tasks::log_decision(c, new, by, now))),
            ),
        }
    }
}

/// Unix milliseconds now.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn open_connection(path: &Path, migrations: &[&str]) -> Result<Connection, DbError> {
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
fn migrate(conn: &mut Connection, path: &Path, steps: &[&str]) -> Result<(), DbError> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let supported = steps.len() as u32;
    if current > supported {
        return Err(DbError::SchemaTooNew {
            path: path.to_owned(),
            found: current,
            supported,
        });
    }
    for (index, sql) in steps.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as u32 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

//! The real core: one thread that owns the event sequence, talks to the DB thread and the socket,
//! and hands the UI the same [`CoreLink`] as `MockCore`.
//!
//! M5 covers tasks and decisions. Session commands answer `CommandFailed` until their milestones.

use std::sync::Arc;
use std::sync::mpsc;

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::{
    Command, CoreLink, DecisionAuthor, Event, NewTask, PROTOCOL_VERSION, ServerMsg, Snapshot, Task,
    TaskChange, TaskId, TaskStatus,
};
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

use crate::config::Config;
use crate::db::{Db, DbError, now_ms};
use crate::paths::Paths;
use crate::projects::Project;
use crate::socket::{
    self, BoxFuture, SocketError, SocketHandler, ToolCall, ToolHandler, UnservedTools,
};

/// Errors starting the core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The databases could not be opened.
    #[error(transparent)]
    Db(#[from] DbError),
    /// The socket could not be created.
    #[error(transparent)]
    Socket(#[from] SocketError),
    /// The core thread could not be started.
    #[error("starting the core thread: {0}")]
    Thread(std::io::Error),
}

/// Starts the real core for `project`, with tool calls unserved. See [`spawn_with`].
pub fn spawn(project: Project, paths: Paths, config: Config) -> Result<CoreLink, CoreError> {
    spawn_with(project, paths, config, Arc::new(UnservedTools))
}

/// Starts the real core for `project` on its own thread and returns the UI's end of the link.
///
/// The first messages are `Hello` and a `Snapshot` of the project's tasks and decisions. The core
/// also listens on the project's socket; `tools` answers the MCP bridges' tool calls.
pub fn spawn_with(
    project: Project,
    paths: Paths,
    config: Config,
    tools: Arc<dyn ToolHandler>,
) -> Result<CoreLink, CoreError> {
    let db = Db::open(&paths, Some(&project.id))?;
    let socket_path = paths.socket(&project.id);
    let listener = socket::bind(&socket_path)?;

    let (command_tx, command_rx) = mpsc::channel::<Command>();
    let (message_tx, message_rx) = mpsc::channel::<ServerMsg>();
    let (input_tx, input_rx) = tokio_mpsc::unbounded_channel::<Input>();

    // Commands arrive on a std channel (the UI is synchronous); forward them into the loop.
    let forward = input_tx.clone();
    std::thread::Builder::new()
        .name("dovetail-commands".into())
        .spawn(move || {
            for command in command_rx {
                if forward.send(Input::Command(command)).is_err() {
                    return;
                }
            }
            // The UI dropped its end.
            let _ = forward.send(Input::Command(Command::Quit {
                end_sessions: false,
            }));
        })
        .map_err(CoreError::Thread)?;

    let handler: Arc<dyn SocketHandler> = Arc::new(CoreHandler {
        inputs: input_tx,
        tools,
    });
    std::thread::Builder::new()
        .name("dovetail-core".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    tracing::error!("starting the core runtime: {e}");
                    return;
                }
            };
            runtime.block_on(async move {
                tokio::spawn(async move {
                    if let Err(e) = socket::serve(listener, handler).await {
                        tracing::error!("socket server stopped: {e}");
                    }
                });
                let core = Core {
                    db,
                    project,
                    config,
                    out: message_tx,
                    seq: 0,
                };
                core.run(input_rx).await;
            });
            let _ = std::fs::remove_file(&socket_path);
        })
        .map_err(CoreError::Thread)?;

    Ok(CoreLink {
        commands: command_tx,
        messages: message_rx,
    })
}

/// What the core loop reacts to.
enum Input {
    Command(Command),
    /// A task from `dovetail add` over the socket.
    AddTask {
        task: NewTask,
        reply: oneshot::Sender<Result<TaskId, String>>,
    },
}

/// Hands socket requests to the loop and tool calls to the pluggable handler.
struct CoreHandler {
    inputs: tokio_mpsc::UnboundedSender<Input>,
    tools: Arc<dyn ToolHandler>,
}

impl SocketHandler for CoreHandler {
    fn add_task(&self, task: NewTask) -> BoxFuture<Result<TaskId, String>> {
        let (reply, answer) = oneshot::channel();
        let sent = self.inputs.send(Input::AddTask { task, reply });
        Box::pin(async move {
            sent.map_err(|_| "Dovetail is shutting down".to_owned())?;
            answer
                .await
                .map_err(|_| "Dovetail is shutting down".to_owned())?
        })
    }

    fn tool_call(&self, call: ToolCall) -> BoxFuture<SocketResponseBody> {
        self.tools.call(call)
    }
}

struct Core {
    db: Db,
    project: Project,
    /// Not read yet; sessions (M6+) use the caps, `claude` path and `max_slices`.
    #[allow(dead_code)]
    config: Config,
    out: mpsc::Sender<ServerMsg>,
    seq: u64,
}

impl Core {
    async fn run(mut self, mut inputs: tokio_mpsc::UnboundedReceiver<Input>) {
        if self.send_snapshot(true).await.is_err() {
            return;
        }
        while let Some(input) = inputs.recv().await {
            let alive = match input {
                Input::Command(Command::Quit { .. }) => break,
                Input::Command(command) => self.command(command).await,
                Input::AddTask { task, reply } => {
                    let result = self.create_task(task).await;
                    let _ = reply.send(result);
                    true
                }
            };
            if !alive {
                break;
            }
        }
    }

    /// Sends `Hello` (when `hello`) and a fresh snapshot. `Err` when the UI is gone.
    async fn send_snapshot(&mut self, hello: bool) -> Result<(), ()> {
        let snapshot = match self.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(e) => {
                tracing::error!("loading state: {e}");
                Snapshot {
                    seq: self.seq,
                    now_ms: now_ms(),
                    project: Some(self.project.summary()),
                    ..Snapshot::default()
                }
            }
        };
        if hello {
            self.out
                .send(ServerMsg::Hello {
                    protocol: PROTOCOL_VERSION,
                })
                .map_err(|_| ())?;
        }
        self.out.send(ServerMsg::Snapshot(snapshot)).map_err(|_| ())
    }

    async fn snapshot(&self) -> Result<Snapshot, DbError> {
        let projects = self.db.list_projects().await?;
        Ok(Snapshot {
            seq: self.seq,
            now_ms: now_ms(),
            project: Some(self.project.summary()),
            projects: projects.iter().map(Project::summary).collect(),
            tasks: self.db.load_tasks().await?,
            decisions: self.db.load_decisions().await?,
            ..Snapshot::default()
        })
    }

    /// Numbers and sends one event. `false` when the UI is gone.
    fn emit(&mut self, event: Event) -> bool {
        self.seq += 1;
        self.out
            .send(ServerMsg::Event {
                seq: self.seq,
                event,
            })
            .is_ok()
    }

    fn emit_tasks(&mut self, tasks: Vec<Task>) -> bool {
        tasks
            .into_iter()
            .all(|task| self.emit(Event::TaskUpserted(task)))
    }

    fn fail(&mut self, message: impl Into<String>) -> bool {
        self.emit(Event::CommandFailed {
            message: message.into(),
        })
    }

    /// Creates a task and tells the UI. Used by the `CreateTask` command and the socket.
    async fn create_task(&mut self, new: NewTask) -> Result<TaskId, String> {
        let tasks = self
            .db
            .create_task(new, false)
            .await
            .map_err(|e| e.to_string())?;
        let id = tasks.first().map(|t| t.id).ok_or("no task was created")?;
        self.emit_tasks(tasks);
        Ok(id)
    }

    /// Handles one command. `false` when the UI is gone.
    async fn command(&mut self, command: Command) -> bool {
        match command {
            Command::CreateTask(new) => match self.create_task(new).await {
                Ok(_) => true,
                Err(message) => self.fail(message),
            },
            Command::UpdateTask { id, patch } => self.update(id, patch).await,
            Command::ArchiveTask { id } => {
                self.update(id, vec![TaskChange::Status(TaskStatus::Archived)])
                    .await
            }
            Command::ReorderTask { id, delta } => match self.db.reorder_task(id, delta).await {
                Ok(tasks) => self.emit_tasks(tasks),
                Err(e) => self.fail(e.to_string()),
            },
            Command::LogDecision(new) => {
                match self.db.log_decision(new, DecisionAuthor::User).await {
                    Ok(decision) => self.emit(Event::DecisionAdded(decision)),
                    Err(e) => self.fail(e.to_string()),
                }
            }
            Command::Resync => self.send_snapshot(false).await.is_ok(),
            // Quit is handled by the loop.
            Command::Quit { .. } => true,
            _ => self.fail("not built yet"),
        }
    }

    async fn update(&mut self, id: TaskId, patch: Vec<TaskChange>) -> bool {
        match self.db.update_task(id, patch).await {
            Ok(tasks) => self.emit_tasks(tasks),
            Err(e) => self.fail(e.to_string()),
        }
    }
}

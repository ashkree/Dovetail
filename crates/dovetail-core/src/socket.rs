//! The app's unix socket server: serves `dovetail add` and the `dovetail mcp` bridges (see
//! `dovetail_api::socket`).
//!
//! Framing is NDJSON. Each request runs on its own task and its response goes out when it is
//! ready, so a held `ask_user` never blocks other requests on the same connection.
//!
//! Wave 2 plugs tool handlers in by implementing [`ToolHandler`] and passing it to
//! [`crate::spawn_with`]; until then [`UnservedTools`] answers every call with an error.

use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdListener, UnixStream as StdStream};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::{AgentId, NewTask, TaskId};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

/// A boxed future, so handlers can be trait objects without extra crates.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// One MCP `tools/call` forwarded by a session's bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The calling session.
    pub agent: AgentId,
    /// Tool name without the MCP server prefix, e.g. `ask_user`.
    pub tool: String,
    /// The call's tool-use id, for attributing subagent calls.
    pub tool_use_id: Option<String>,
    /// The tool's arguments.
    pub input: serde_json::Value,
}

/// Serves MCP tool calls. A call may take as long as it needs (a blocking `ask_user`); the
/// socket keeps serving other requests meanwhile.
pub trait ToolHandler: Send + Sync + 'static {
    /// Answers one call. Return `ToolResult` or `Error`.
    fn call(&self, call: ToolCall) -> BoxFuture<SocketResponseBody>;
}

/// The handler used until real tools exist.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnservedTools;

impl ToolHandler for UnservedTools {
    fn call(&self, _call: ToolCall) -> BoxFuture<SocketResponseBody> {
        Box::pin(async {
            SocketResponseBody::Error {
                message: "tool calls are not served yet".into(),
            }
        })
    }
}

/// What the socket server asks of the core.
pub trait SocketHandler: Send + Sync + 'static {
    /// Creates a task from `dovetail add`.
    fn add_task(&self, task: NewTask) -> BoxFuture<Result<TaskId, String>>;
    /// Answers a forwarded tool call.
    fn tool_call(&self, call: ToolCall) -> BoxFuture<SocketResponseBody>;
}

/// Errors starting the socket server.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
    /// Another Dovetail already listens on this socket.
    #[error("another Dovetail is already running for this project ({0})")]
    AlreadyRunning(std::path::PathBuf),
    /// The socket could not be created.
    #[error("socket {path}: {source}")]
    Io {
        /// The socket path.
        path: std::path::PathBuf,
        /// The cause.
        source: std::io::Error,
    },
}

/// Binds the socket at `path` (mode 0600, parent directory 0700). A stale socket file left by a
/// crashed run is removed; a live one means another app runs, which is an error.
pub fn bind(path: &Path) -> Result<StdListener, SocketError> {
    let io = |source| SocketError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(io)?;
    }
    if path.exists() {
        if StdStream::connect(path).is_ok() {
            return Err(SocketError::AlreadyRunning(path.to_owned()));
        }
        std::fs::remove_file(path).map_err(io)?;
    }
    let listener = StdListener::bind(path).map_err(io)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    listener.set_nonblocking(true).map_err(io)?;
    Ok(listener)
}

/// Accepts connections until the task is dropped. Call inside the tokio runtime.
pub async fn serve(listener: StdListener, handler: Arc<dyn SocketHandler>) -> std::io::Result<()> {
    let listener = UnixListener::from_std(listener)?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(connection(stream, handler.clone()));
    }
}

async fn connection(stream: UnixStream, handler: Arc<dyn SocketHandler>) {
    let (read, mut write) = stream.into_split();
    let (out, mut responses) = mpsc::unbounded_channel::<SocketResponse>();
    tokio::spawn(async move {
        while let Some(response) = responses.recv().await {
            let Ok(mut line) = serde_json::to_string(&response) else {
                continue;
            };
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let mut lines = AsyncBufReader::new(read).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                tracing::debug!("socket read failed: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: SocketRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) => {
                tracing::warn!("bad socket request: {e}");
                let _ = out.send(SocketResponse {
                    id: 0,
                    body: SocketResponseBody::Error {
                        message: format!("bad request: {e}"),
                    },
                });
                continue;
            }
        };
        let handler = handler.clone();
        let out = out.clone();
        tokio::spawn(async move {
            let body = match request.body {
                SocketRequestBody::AddTask { task } => match handler.add_task(task).await {
                    Ok(id) => SocketResponseBody::TaskAdded { id },
                    Err(message) => SocketResponseBody::Error { message },
                },
                SocketRequestBody::ToolCall {
                    agent,
                    tool,
                    tool_use_id,
                    input,
                } => {
                    handler
                        .tool_call(ToolCall {
                            agent,
                            tool,
                            tool_use_id,
                            input,
                        })
                        .await
                }
            };
            // The client may have gone; nothing to do then.
            let _ = out.send(SocketResponse {
                id: request.id,
                body,
            });
        });
    }
}

/// Sends `dovetail add`'s task to a running app. `Ok(None)` means no app listens on `path`.
/// Blocking; for the binary's short-lived subcommands.
pub fn try_add_task(path: &Path, task: NewTask) -> std::io::Result<Option<TaskId>> {
    use std::io::ErrorKind;
    let stream = match StdStream::connect(path) {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request = SocketRequest {
        id: 1,
        body: SocketRequestBody::AddTask { task },
    };
    let mut line = serde_json::to_string(&request).map_err(std::io::Error::other)?;
    line.push('\n');
    (&stream).write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    let response: SocketResponse = serde_json::from_str(&reply).map_err(std::io::Error::other)?;
    match response.body {
        SocketResponseBody::TaskAdded { id } => Ok(Some(id)),
        SocketResponseBody::Error { message } => Err(std::io::Error::other(message)),
        other => Err(std::io::Error::other(format!(
            "unexpected reply: {other:?}"
        ))),
    }
}

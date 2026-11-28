//! `dovetail mcp`: the per-session stdio MCP bridge, forwarding tool calls to the app socket.
//!
//! One bridge runs per Claude Code session (subagents share their parent's connection), so it is
//! deliberately small: synchronous, std plus `serde_json`, no MCP crate, no async runtime. The main
//! thread reads JSON-RPC lines from stdin. Each `tools/call` runs on its own thread, so a slow
//! call never blocks `ping`, `tools/list` or other calls. One reader thread
//! per socket connection matches [`SocketResponse`]s to waiting calls by `id`.
//!
//! # Methods
//!
//! `server/discover` (empty result; Claude Code probes it before `initialize`), `initialize`,
//! `notifications/initialized` (and any other notification: ignored), `tools/list`, `tools/call`,
//! `ping`. Everything else gets JSON-RPC `-32601` (method not found). Nothing is written to stdout
//! except JSON-RPC.
//!
//! # Environment (plan B18)
//!
//! * `DOVETAIL_AGENT_ID`: the session's agent id.
//! * `DOVETAIL_SOCKET`: path of the app's unix socket.
//! * `DOVETAIL_ROLE`: `coordinator`, `slice` or `side-chat`; the rules `validate` applies.
//! * `DOVETAIL_SUITES`: the session's tool suites, comma-separated (ADR 0016); decides which
//!   tools `tools/list` shows. Without it, the role's defaults.
//!
//! # What the core passes to Claude Code (wave 2)
//!
//! Per session, in the file given to `--mcp-config` (without `--strict-mcp-config`, so the user's
//! own MCP servers still load; plan B28):
//!
//! ```json
//! {"mcpServers": {"dovetail": {
//!   "command": "/abs/path/to/dovetail",
//!   "args": ["mcp"],
//!   "env": {
//!     "DOVETAIL_AGENT_ID": "<agent id>",
//!     "DOVETAIL_SOCKET": "<$XDG_RUNTIME_DIR/dovetail/<project-id>.sock>",
//!     "DOVETAIL_ROLE": "coordinator",
//!     "DOVETAIL_SUITES": "core,tasks,project,coordination,research,maps"
//!   }
//! }}}
//! ```
//!
//! With the server named `dovetail`, Claude Code sees the tools as `mcp__dovetail__<tool>`, each
//! named `<suite>_<verb>` (`core_ask_user`, `tasks_create`, `maps_draw`; the table is
//! `dovetail_tools::TOOLS`). By default the coordinator gets every suite, slices Core,
//! Coordination and Maps, and side chats Core.
//!
//! For one release each renamed tool is also listed under its old name (`ask_user`), with an
//! open schema and a description naming the new tool, so a resumed conversation that calls it
//! still works: the call is forwarded with `called_as`, and the app logs it and adds a note
//! naming the new tool to the result.
//!
//! The core passes the same names to `--allowedTools` (`dovetail_core::sessions::launch`). The
//! `tool` field forwarded to the app is the bare new name (`core_ask_user`). Subagents reach the
//! tools through `ToolSearch`. A connection the app closed (it quit or restarted) is replaced
//! once before a call fails.

mod schema;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use anyhow::Result;
use dovetail_api::agents::agent::AgentId;
use dovetail_tools::{
    Role, SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody, Suite, Tool,
    deprecated_alias, env,
};
use serde_json::{Value, json};

const PROTOCOL_FALLBACK: &str = "2025-06-18";

type Reply = Result<SocketResponseBody, String>;
type Pending = HashMap<u64, Sender<Reply>>;

/// A live socket connection: the write half, and the pending calls its reader thread serves.
struct Conn {
    writer: UnixStream,
    pending: Arc<Mutex<Pending>>,
    /// Cleared by the reader when the app closes the connection (it quit or restarted).
    alive: Arc<AtomicBool>,
}

struct Bridge {
    agent: AgentId,
    socket: Option<String>,
    role: Role,
    suites: Vec<Suite>,
    next_id: AtomicU64,
    conn: Mutex<Option<Conn>>,
    out: Mutex<std::io::Stdout>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another call thread panicked; the data is still usable.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run the `dovetail mcp` stdio bridge.
pub fn run() -> Result<()> {
    let role = Role::from_env_value(&std::env::var(env::ROLE).unwrap_or_default());
    let suites = match std::env::var(env::SUITES) {
        Ok(list) => Suite::parse_list(&list),
        Err(_) => role.default_suites().to_vec(),
    };
    let bridge = Arc::new(Bridge {
        agent: AgentId(std::env::var(env::AGENT_ID).unwrap_or_default()),
        socket: std::env::var(env::SOCKET).ok(),
        role,
        suites,
        next_id: AtomicU64::new(1),
        conn: Mutex::new(None),
        out: Mutex::new(std::io::stdout()),
    });

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                bridge.send(&rpc_error(
                    Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                ));
                continue;
            }
        };
        bridge.dispatch(msg);
    }
    Ok(())
}

impl Bridge {
    fn send(&self, msg: &Value) {
        let mut out = lock(&self.out);
        // If stdout is gone, Claude Code is gone; the stdin loop ends on its own.
        let _ = writeln!(out, "{msg}");
        let _ = out.flush();
    }

    fn dispatch(self: &Arc<Self>, msg: Value) {
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return; // a response to something we never asked, or garbage
        };
        let Some(id) = msg.get("id").cloned() else {
            return; // notification (initialized, cancelled, ...): nothing to answer
        };
        match method {
            "server/discover" | "ping" => self.send(&rpc_result(id, json!({}))),
            "initialize" => {
                let version = msg
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_FALLBACK);
                self.send(&rpc_result(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "dovetail", "version": env!("CARGO_PKG_VERSION")},
                    }),
                ));
            }
            "tools/list" => {
                let tools = schema::tools_list(&self.suites);
                self.send(&rpc_result(id, json!({"tools": tools})));
            }
            "tools/call" => {
                let this = Arc::clone(self);
                thread::spawn(move || {
                    let result = this.call_tool(&msg);
                    this.send(&rpc_result(id, result));
                });
            }
            other => self.send(&rpc_error(
                id,
                -32601,
                &format!("method not found: {other}"),
            )),
        }
    }

    fn call_tool(&self, msg: &Value) -> Value {
        let Some(name) = msg.pointer("/params/name").and_then(Value::as_str) else {
            return tool_error("tools/call needs a tool name");
        };
        if !Tool::allowed(name, &self.suites) {
            return tool_error(&format!("unknown tool `{name}` for this session"));
        }
        let arguments = msg
            .pointer("/params/arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        // Local checks, so the agent gets a precise error without a round trip.
        let call = match Tool::parse(name, arguments) {
            Ok(call) => call,
            Err(problem) => return tool_error(&problem.to_string()),
        };
        if let Err(problem) = call.validate(self.role) {
            return tool_error(&problem.to_string());
        }
        let tool_use_id = msg
            .pointer("/params/_meta/claudecode~1toolUseId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let body = SocketRequestBody::ToolCall {
            agent: self.agent.clone(),
            tool_use_id,
            call,
            called_as: deprecated_alias(name).map(|_| name.to_owned()),
        };
        match self.roundtrip(body) {
            Ok(SocketResponseBody::ToolResult { text, is_error }) => text_result(&text, is_error),
            Ok(SocketResponseBody::Error { message }) => text_result(
                &format!("Dovetail could not handle this call: {message}"),
                true,
            ),
            Ok(SocketResponseBody::TaskAdded { .. } | SocketResponseBody::UpgradeStaged) => {
                text_result("Dovetail sent an unexpected reply to this call.", true)
            }
            Err(e) => text_result(&e, true),
        }
    }

    fn roundtrip(&self, body: SocketRequestBody) -> Reply {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = serde_json::to_string(&SocketRequest { id, body })
            .map_err(|e| format!("could not encode the request: {e}"))?;
        // After an app restart the old connection is dead: reconnect once before failing.
        let rx = match self.send_line(id, &line) {
            Ok(rx) => rx,
            Err(_) => self.send_line(id, &line)?,
        };
        // A call waits here for as long as the app takes to answer it.
        match rx.recv() {
            Ok(reply) => reply,
            Err(_) => Err(not_running("the connection closed")),
        }
    }

    /// Writes one request on the connection, connecting first if there is none or the app
    /// closed the last one. A failed write drops the connection, so the next try reconnects.
    fn send_line(&self, id: u64, line: &str) -> Result<Receiver<Reply>, String> {
        let (tx, rx) = channel();
        let mut guard = lock(&self.conn);
        if guard
            .as_ref()
            .is_some_and(|c| !c.alive.load(Ordering::SeqCst))
        {
            *guard = None;
        }
        if guard.is_none() {
            *guard = Some(self.connect()?);
        }
        let Some(conn) = guard.as_mut() else {
            return Err(not_running("no connection"));
        };
        lock(&conn.pending).insert(id, tx);
        let written = writeln!(conn.writer, "{line}").and_then(|()| conn.writer.flush());
        if let Err(e) = written {
            lock(&conn.pending).remove(&id);
            *guard = None;
            return Err(not_running(&e.to_string()));
        }
        Ok(rx)
    }

    fn connect(&self) -> Result<Conn, String> {
        let path = self
            .socket
            .as_deref()
            .ok_or_else(|| not_running(&format!("{} is not set", env::SOCKET)))?;
        let stream = UnixStream::connect(path).map_err(|e| not_running(&e.to_string()))?;
        let reader = stream
            .try_clone()
            .map_err(|e| not_running(&e.to_string()))?;
        let pending: Arc<Mutex<Pending>> = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let for_reader = Arc::clone(&pending);
        let reader_alive = Arc::clone(&alive);
        thread::spawn(move || read_responses(reader, for_reader, reader_alive));
        Ok(Conn {
            writer: stream,
            pending,
            alive,
        })
    }
}

/// Route responses to waiting calls; when the socket closes, fail every call still waiting.
fn read_responses(stream: UnixStream, pending: Arc<Mutex<Pending>>, alive: Arc<AtomicBool>) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        match serde_json::from_str::<SocketResponse>(&line) {
            Ok(resp) => {
                if let Some(tx) = lock(&pending).remove(&resp.id) {
                    let _ = tx.send(Ok(resp.body));
                }
            }
            Err(e) => tracing::warn!("bad socket response: {e}"),
        }
    }
    alive.store(false, Ordering::SeqCst);
    let reason = not_running("the connection closed");
    for (_, tx) in lock(&pending).drain() {
        let _ = tx.send(Err(reason.clone()));
    }
}

fn not_running(detail: &str) -> String {
    format!(
        "Dovetail is not running or not reachable ({detail}). Your request was not delivered. \
         Tell the user in your reply; do not retry in a loop."
    )
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn tool_error(text: &str) -> Value {
    text_result(text, true)
}

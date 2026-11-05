//! `dovetail mcp`: the per-session stdio MCP bridge, forwarding tool calls to the app socket.
//!
//! One bridge runs per Claude Code session (subagents share their parent's connection), so it is
//! deliberately small: synchronous, std plus `serde_json`, no MCP crate, no async runtime. The main
//! thread reads JSON-RPC lines from stdin. Each `tools/call` runs on its own thread, so a held
//! `ask_user` (15+ minutes) never blocks `ping`, `tools/list` or other calls. One reader thread
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
//! * `DOVETAIL_ROLE`: `coordinator` or `slice`; decides which tools `tools/list` shows.
//!
//! # What the core passes to Claude Code (wave 2)
//!
//! Per session, in the file given to `--mcp-config` (without `--strict-mcp-config`, so the user's own MCP servers still load; plan B28):
//!
//! ```json
//! {"mcpServers": {"dovetail": {
//!   "command": "/abs/path/to/dovetail",
//!   "args": ["mcp"],
//!   "env": {
//!     "DOVETAIL_AGENT_ID": "<agent id>",
//!     "DOVETAIL_SOCKET": "<$XDG_RUNTIME_DIR/dovetail/<project-id>.sock>",
//!     "DOVETAIL_ROLE": "coordinator"
//!   }
//! }}}
//! ```
//!
//! With the server named `dovetail`, Claude Code sees the tools as `mcp__dovetail__<tool>`:
//!
//! * both roles: `mcp__dovetail__ask_user`, `mcp__dovetail__report_status`,
//!   `mcp__dovetail__log_decision`
//! * coordinator only: `mcp__dovetail__create_task`, `mcp__dovetail__start_slice`
//!
//! Pass these to `--allowedTools` (or use the prefix `mcp__dovetail`). The `tool` field forwarded
//! to the app is the bare name (`ask_user`). Subagents reach the tools through `ToolSearch`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use anyhow::Result;
use dovetail_api::AgentId;
use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use serde_json::{Value, json};

const PROTOCOL_FALLBACK: &str = "2025-06-18";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Coordinator,
    Slice,
}

type Reply = Result<SocketResponseBody, String>;
type Pending = HashMap<u64, Sender<Reply>>;

/// A live socket connection: the write half, and the pending calls its reader thread serves.
struct Conn {
    writer: UnixStream,
    pending: Arc<Mutex<Pending>>,
}

struct Bridge {
    agent: AgentId,
    socket: Option<String>,
    role: Role,
    next_id: AtomicU64,
    conn: Mutex<Option<Conn>>,
    out: Mutex<std::io::Stdout>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another call thread panicked; the data is still usable.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run the subcommand with the arguments after its name.
pub fn run(_args: Vec<String>) -> Result<()> {
    let role = match std::env::var("DOVETAIL_ROLE").as_deref() {
        Ok("coordinator") => Role::Coordinator,
        _ => Role::Slice,
    };
    let bridge = Arc::new(Bridge {
        agent: AgentId(std::env::var("DOVETAIL_AGENT_ID").unwrap_or_default()),
        socket: std::env::var("DOVETAIL_SOCKET").ok(),
        role,
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
                let tools: Vec<Value> = tool_defs()
                    .into_iter()
                    .filter(|t| self.allowed(t.name))
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": (t.schema)(),
                        })
                    })
                    .collect();
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

    fn allowed(&self, tool: &str) -> bool {
        match tool {
            "create_task" | "start_slice" => self.role == Role::Coordinator,
            _ => tool_defs().iter().any(|t| t.name == tool),
        }
    }

    fn call_tool(&self, msg: &Value) -> Value {
        let Some(name) = msg.pointer("/params/name").and_then(Value::as_str) else {
            return tool_error("tools/call needs a tool name");
        };
        if !self.allowed(name) {
            return tool_error(&format!("unknown tool `{name}` for this session"));
        }
        let input = msg
            .pointer("/params/arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(problem) = self.validate(name, &input) {
            return tool_error(&problem);
        }
        let tool_use_id = msg
            .pointer("/params/_meta/claudecode~1toolUseId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let body = SocketRequestBody::ToolCall {
            agent: self.agent.clone(),
            tool: name.to_owned(),
            tool_use_id,
            input,
        };
        match self.roundtrip(body) {
            Ok(SocketResponseBody::ToolResult { text, is_error }) => text_result(&text, is_error),
            Ok(SocketResponseBody::Error { message }) => text_result(
                &format!("Dovetail could not handle this call: {message}"),
                true,
            ),
            Ok(SocketResponseBody::TaskAdded { .. }) => {
                text_result("Dovetail sent an unexpected reply to this call.", true)
            }
            Err(e) => text_result(&e, true),
        }
    }

    /// Cheap local checks so the agent gets a precise error without a round trip.
    fn validate(&self, tool: &str, input: &Value) -> Option<String> {
        let need_id = |field: &str| {
            let ok = input
                .get(field)
                .and_then(Value::as_i64)
                .is_some_and(|id| id > 0);
            (!ok).then(|| {
                format!("`{field}` is required and must be a task id (a positive integer)")
            })
        };
        let need = |field: &str| {
            let ok = input
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            (!ok).then(|| format!("`{field}` is required and must be a non-empty string"))
        };
        match tool {
            "ask_user" => need("question"),
            "report_status" => need("status"),
            "create_task" => need("title"),
            "start_slice" => need_id("task")
                .or_else(|| need("name"))
                .or_else(|| need("brief")),
            "log_decision" => {
                let task = if self.role == Role::Coordinator {
                    need_id("task").map(|_| {
                        "the coordinator must name the `task` this decision belongs to".to_owned()
                    })
                } else {
                    None
                };
                task.or_else(|| need("decision"))
            }
            _ => None,
        }
    }

    fn roundtrip(&self, body: SocketRequestBody) -> Reply {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        let line = serde_json::to_string(&SocketRequest { id, body })
            .map_err(|e| format!("could not encode the request: {e}"))?;
        {
            let mut guard = lock(&self.conn);
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
        }
        // Held calls (blocking `ask_user`) wait here for as long as the app takes.
        match rx.recv() {
            Ok(reply) => reply,
            Err(_) => Err(not_running("the connection closed")),
        }
    }

    fn connect(&self) -> Result<Conn, String> {
        let path = self
            .socket
            .as_deref()
            .ok_or_else(|| not_running("DOVETAIL_SOCKET is not set"))?;
        let stream = UnixStream::connect(path).map_err(|e| not_running(&e.to_string()))?;
        let reader = stream
            .try_clone()
            .map_err(|e| not_running(&e.to_string()))?;
        let pending: Arc<Mutex<Pending>> = Arc::default();
        let for_reader = Arc::clone(&pending);
        thread::spawn(move || read_responses(reader, for_reader));
        Ok(Conn {
            writer: stream,
            pending,
        })
    }
}

/// Route responses to waiting calls; when the socket closes, fail every call still waiting.
fn read_responses(stream: UnixStream, pending: Arc<Mutex<Pending>>) {
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

struct ToolDef {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "ask_user",
            description: "Ask the user a question and get the answer through Dovetail's inbox. \
                Use this whenever you need a decision, a missing value, credentials or access; \
                never end your turn with a question in plain text, because the user may not see \
                it. Set `blocking: true` if you cannot continue without the answer: the call then \
                waits (possibly many minutes) and returns the answer. Otherwise it returns at \
                once and the answer arrives later as a new user message, so keep working on \
                anything that does not depend on it. Make the question self-contained, with the \
                options you see.",
            schema: || {
                json!({"type": "object", "properties": {
                    "question": {"type": "string", "description": "The question, self-contained, with the options if there are any."},
                    "blocking": {"type": "boolean", "description": "Wait for the answer (true) or return at once and get the answer as a later message (false, the default)."}
                }, "required": ["question"]})
            },
        },
        ToolDef {
            name: "report_status",
            description: "Set your one-line status, shown next to you in Dovetail's task tree. \
                Call it when you start, when your focus changes, and when you finish. Plain \
                words, present tense, at most 120 characters (longer text is cut).",
            schema: || {
                json!({"type": "object", "properties": {
                    "status": {"type": "string", "maxLength": 120, "description": "One line, 120 characters at most."}
                }, "required": ["status"]})
            },
        },
        ToolDef {
            name: "create_task",
            description: "Create a task in the user's task list. Use it to file work you have \
                split out of the feature, or follow-ups you notice, instead of only mentioning \
                them in chat. The task lands in Unsorted (or the bucket you give), flagged as \
                coordinator-created until the user has seen it; it never goes straight into \
                Now. Returns the new task's id; pass that id to `start_slice` or \
                `log_decision`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "title": {"type": "string", "description": "Short imperative title."},
                    "notes": {"type": "string", "description": "Context, acceptance criteria, links."},
                    "p_level": {"type": "string", "enum": ["P0", "P1", "P2", "P3"], "description": "Priority, P0 highest. Omit if unsure."},
                    "bucket": {"type": "string", "enum": ["next", "later"], "description": "Omit to land in Unsorted. Never Now: the user decides what is current."},
                    "follow_up_of": {"type": "integer", "description": "Id of the task this follows up."},
                    "related": {"type": "array", "items": {"type": "integer"}, "description": "Ids of related tasks."}
                }, "required": ["title"]})
            },
        },
        ToolDef {
            name: "log_decision",
            description: "Record a decision in the task's decision log, so it outlives this \
                conversation. Use it when you or the user settle a design choice, scope cut or \
                approach, in one line, with the reason and the alternatives that lost. Entries \
                are append-only; to change an earlier decision, log a new one that supersedes \
                it. Slices log on their own task by default; the coordinator must name `task`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Task id. Required for the coordinator; slices default to their own task."},
                    "decision": {"type": "string", "description": "The decision, one line."},
                    "why": {"type": "string", "description": "The reason."},
                    "alternatives": {"type": "array", "description": "Options considered and rejected.", "items": {
                        "type": "object", "properties": {
                            "option": {"type": "string"},
                            "why_not": {"type": "string", "description": "Why it lost."}
                        }, "required": ["option"]}},
                    "supersedes": {"type": "integer", "description": "Id of the earlier decision this replaces."}
                }, "required": ["decision"]})
            },
        },
        ToolDef {
            name: "start_slice",
            description: "Start a slice session (its own git worktree) on an existing task. \
                For new work call `create_task` first. Depending on the permission mode the \
                user may be asked to approve. Returns at once with `started`, `queued` (the \
                maximum number of slices is running; it starts when one is archived) or \
                `denied`. Do not message the slice until Dovetail sends you \"slice <name> \
                ready\"; a setup failure arrives the same way. The brief becomes the slice's \
                first prompt, so make it complete: goal, scope, files, checks to run.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of an existing task."},
                    "name": {"type": "string", "description": "Short slice name, used for the branch and worktree."},
                    "brief": {"type": "string", "description": "The slice's first prompt."}
                }, "required": ["task", "name", "brief"]})
            },
        },
    ]
}

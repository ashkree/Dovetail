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
//! * coordinator only: `mcp__dovetail__create_task`, `mcp__dovetail__start_slice`,
//!   `mcp__dovetail__add_project`, `mcp__dovetail__archive_slice`, `mcp__dovetail__archive_task`,
//!   `mcp__dovetail__list_tasks`, `mcp__dovetail__get_task`, `mcp__dovetail__update_task`
//!
//! The core passes these to `--allowedTools` (`dovetail_core::session::launch`). The `tool`
//! field forwarded to the app is the bare name (`ask_user`). Subagents reach the tools through
//! `ToolSearch`. A connection the app closed (it quit or restarted) is replaced once before a
//! call fails.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use anyhow::Result;
use dovetail_api::AgentId;
use dovetail_api::socket::{SocketRequest, SocketRequestBody, SocketResponse, SocketResponseBody};
use dovetail_api::tools::{Role, Tool, env};
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
    let role = Role::from_env_value(&std::env::var(env::ROLE).unwrap_or_default());
    let bridge = Arc::new(Bridge {
        agent: AgentId(std::env::var(env::AGENT_ID).unwrap_or_default()),
        socket: std::env::var(env::SOCKET).ok(),
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
                    .filter(|t| Tool::allowed(t.name, self.role))
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

    fn call_tool(&self, msg: &Value) -> Value {
        let Some(name) = msg.pointer("/params/name").and_then(Value::as_str) else {
            return tool_error("tools/call needs a tool name");
        };
        if !Tool::allowed(name, self.role) {
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
                Use this for questions that must wait while the user is away, or for what only \
                the user can give, such as credentials or access; questions for a user who is in \
                the conversation go in your reply. The call returns at once: the question goes to \
                the user's inbox and conversation, and the answer arrives later as a new user \
                message. Keep working \
                on anything that does not depend on it, or end your turn if nothing is left. \
                Make the question self-contained, with the options you see.",
            schema: || {
                json!({"type": "object", "properties": {
                    "question": {"type": "string", "description": "The question, self-contained, with the options if there are any."}
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
                For new work call `create_task` first. Returns at once with `started`, \
                `queued` (the maximum number of slices is running; it starts when one is \
                archived), `denied`, or `pending` (the user is asked to approve; the answer \
                arrives later as a message, \"start_slice <name> approved\" or \"... \
                denied\"). Do not message the slice until Dovetail sends you \"slice <name> \
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
        ToolDef {
            name: "add_project",
            description: "Make the folder you run in a Dovetail project, so its tasks, inbox \
                and this conversation are kept. Call it only when the user asks for that. Until \
                then everything lives in memory and is lost when Dovetail quits. Takes no \
                arguments; calling it on a folder that is already a project does nothing.",
            schema: || json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "archive_slice",
            description: "Archive a slice once its work is done, normally right after its PR \
                is merged and you have pulled. It stops the slice's session and removes its \
                worktree and `dovetail/*` branch, freeing a place for a queued slice. Returns \
                `archived` when it is gone, or `waiting_for_user` when the worktree has \
                uncommitted or unpushed work: the user is asked before anything is lost, and \
                their answer arrives later as a message (\"slice <name> archived\" or \"... \
                kept\"). Fails for an unknown or already archived name.",
            schema: || {
                json!({"type": "object", "properties": {
                    "name": {"type": "string", "description": "The slice's name, as given to `start_slice`."}
                }, "required": ["name"]})
            },
        },
        ToolDef {
            name: "archive_task",
            description: "Archive a finished task. Its live slices are archived first, with \
                `archive_slice`'s rules (unsaved work waits for the user), and its queued \
                slice starts are dropped. Returns `archived`, or `waiting_for_user` if a slice \
                still waits for the user.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "list_tasks",
            description: "Find tasks: one line each with id, status, bucket, priority, title \
                and live slice, ordered Now, Next, Unsorted, Later, then by priority. Archived \
                tasks are left out unless asked for. Use `get_task` for one task's notes and \
                decisions.",
            schema: || {
                json!({"type": "object", "properties": {
                    "bucket": {"type": "string", "enum": ["now", "next", "unsorted", "later"], "description": "Only tasks in this bucket."},
                    "status": {"type": "string", "enum": ["todo", "in_progress", "done", "archived"], "description": "Only tasks with this status."},
                    "include_archived": {"type": "boolean", "default": false, "description": "Also list archived tasks."}
                }})
            },
        },
        ToolDef {
            name: "get_task",
            description: "Read one task in full: status, bucket, priority, notes, \
                follow-up and related ids, its live slices, and its decision log oldest \
                first (text, why, rejected alternatives, what superseded what, user or agent). \
                Use it to brief a slice or hand a reviewer the decisions as spec.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "update_task",
            description: "Change a task's status, bucket, priority, title or notes; only the \
                fields you give change. Use it to mark a task `done` once its slice's PR is \
                merged, or to re-sort and re-prioritise the list. A task moves into Now only \
                when a slice starts on it, so `bucket` cannot be `now`; moving a task out of \
                Now is fine. To archive a task use `archive_task`. Returns the task's new \
                `list_tasks` line. Fails for an unknown or archived task.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."},
                    "status": {"type": "string", "enum": ["todo", "in_progress", "done"], "description": "New status."},
                    "bucket": {"type": "string", "enum": ["next", "later", "unsorted"], "description": "New bucket; `unsorted` clears it. Never `now`."},
                    "p_level": {"type": ["string", "null"], "enum": ["P0", "P1", "P2", "P3", null], "description": "New priority, P0 highest; null clears it."},
                    "title": {"type": "string", "description": "New short imperative title."},
                    "notes": {"type": "string", "description": "New notes; they replace the old ones, so include what should stay."}
                }, "required": ["id"]})
            },
        },
    ]
}

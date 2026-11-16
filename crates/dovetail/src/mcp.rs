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
//! `dovetail_api::tools::TOOLS`). By default the coordinator gets every suite, slices Core,
//! Coordination and Maps, and side chats Core.
//!
//! For one release each renamed tool is also listed under its old name (`ask_user`), with an
//! open schema and a description naming the new tool, so a resumed conversation that calls it
//! still works: the call is forwarded with `called_as`, and the app logs it and adds a note
//! naming the new tool to the result.
//!
//! The core passes the same names to `--allowedTools` (`dovetail_core::session::launch`). The
//! `tool` field forwarded to the app is the bare new name (`core_ask_user`). Subagents reach the
//! tools through `ToolSearch`. A connection the app closed (it quit or restarted) is replaced
//! once before a call fails.

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
use dovetail_api::tools::{Role, Suite, Tool, ToolSpec, deprecated_alias, env, tools_in};
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

/// Run the subcommand with the arguments after its name.
pub fn run(_args: Vec<String>) -> Result<()> {
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
                let tools = tools_list(&self.suites);
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

/// The `tools/list` entries of `suites`: each tool in table order, then, for one release, each
/// renamed tool's old name with an open schema and a description naming the new tool.
fn tools_list(suites: &[Suite]) -> Vec<Value> {
    let defs = tool_defs();
    let current = tools_in(suites).filter_map(|spec| {
        let def = defs.iter().find(|d| d.name == spec.name)?;
        Some(json!({
            "name": def.name,
            "description": def.description,
            "inputSchema": (def.schema)(),
        }))
    });
    let old = tools_in(suites).filter_map(|spec: &ToolSpec| {
        let old = spec.old_name?;
        Some(json!({
            "name": old,
            "description": format!(
                "Deprecated: the old name of `{}`, kept for conversations from before the \
                 rename. Call `{}` instead; it takes the same arguments.",
                spec.name, spec.name
            ),
            "inputSchema": {"type": "object"},
        }))
    });
    current.chain(old).collect()
}

struct ToolDef {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "core_ask_user",
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
            name: "core_report_status",
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
            name: "tasks_create",
            description: "Create a task in the user's task list. Use it to file work you have \
                split out of the feature, or follow-ups you notice, instead of only mentioning \
                them in chat. The task lands in Unsorted (or the bucket you give), flagged as \
                coordinator-created until the user has seen it; it never goes straight into \
                Now. Returns the new task's id; pass that id to `project_start_slice` or \
                `core_log_decision`.",
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
            name: "core_log_decision",
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
            name: "project_start_slice",
            description: "Start a slice session (its own git worktree) on an existing task. \
                For new work call `tasks_create` first. Returns at once with `started`, \
                `queued` (the maximum number of slices is running; it starts when one is \
                archived), `denied`, or `pending` (the user is asked to approve; the answer \
                arrives later as a message, \"project_start_slice <name> approved\" or \"... \
                denied\"). Do not message the slice until Dovetail sends you \"slice <name> \
                ready\"; a setup failure arrives the same way. The brief becomes the slice's \
                first prompt, so make it complete: goal, scope, files, checks to run. The \
                slice works in one of the project's repos: name it with `repo` when the \
                project has several (refused without it); with one it may be left out.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of an existing task."},
                    "name": {"type": "string", "description": "Short slice name, used for the branch and worktree."},
                    "brief": {"type": "string", "description": "The slice's first prompt."},
                    "repo": {"type": "string", "description": "The repo it works in, by name, as Dovetail listed it to you. Required when the project has several repos."}
                }, "required": ["task", "name", "brief"]})
            },
        },
        ToolDef {
            name: "project_add",
            description: "Make the folder you run in a Dovetail project, so its tasks, inbox \
                and this conversation are kept. Call it only when the user asks for that. Until \
                then everything lives in memory and is lost when Dovetail quits. Takes no \
                arguments; calling it on a folder that is already a project does nothing.",
            schema: || json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "project_archive_slice",
            description: "Archive a slice once its work is done, normally right after its PR \
                is merged and you have pulled. It stops the slice's session and removes its \
                worktree and `dovetail/*` branch, freeing a place for a queued slice. Returns \
                `archived` when it is gone, or `waiting_for_user` when the worktree has \
                uncommitted or unpushed work: the user is asked before anything is lost, and \
                their answer arrives later as a message (\"slice <name> archived\" or \"... \
                kept\"). Fails for an unknown or already archived name.",
            schema: || {
                json!({"type": "object", "properties": {
                    "name": {"type": "string", "description": "The slice's name, as given to `project_start_slice`."}
                }, "required": ["name"]})
            },
        },
        ToolDef {
            name: "tasks_archive",
            description: "Archive a finished task. Its live slices are archived first, with \
                `project_archive_slice`'s rules (unsaved work waits for the user), and its queued \
                slice starts are dropped. Returns `archived`, or `waiting_for_user` if a slice \
                still waits for the user.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "tasks_list",
            description: "Find tasks: one line each with id, status, bucket, priority, title \
                and live slice, ordered Now, Next, Unsorted, Later, then by priority. Archived \
                tasks are left out unless asked for. Use `tasks_get` for one task's notes and \
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
            name: "tasks_get",
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
            name: "tasks_update",
            description: "Change a task's bucket, priority, title or notes; only the \
                fields you give change. Use it to re-sort and re-prioritise the list. Status \
                is not yours to set: Dovetail moves a task to in progress, review and done \
                from what its slice and pull request do. Moving a task \
                into Now is allowed; the user sees it flagged as new until they look at it. To archive a task use `tasks_archive`. Returns the task's new \
                `tasks_list` line. Fails for an unknown or archived task.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."},
                    "bucket": {"type": "string", "enum": ["now", "next", "later", "unsorted"], "description": "New bucket; `unsorted` clears it."},
                    "p_level": {"type": ["string", "null"], "enum": ["P0", "P1", "P2", "P3", null], "description": "New priority, P0 highest; null clears it."},
                    "title": {"type": "string", "description": "New short imperative title."},
                    "notes": {"type": "string", "description": "New notes; they replace the old ones, so include what should stay."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "coordination_report",
            description: "Report to the coordinator about your task. Use it instead of \
                SendMessage for reports: when something worth knowing happened (`progress`), \
                when your work is ready for review, normally with its pull request open \
                (`done`), and when you cannot go on without the coordinator (`blocked`). The \
                coordinator gets the report as a message and the user sees it as one line. \
                `done` moves your task to review.",
            schema: || {
                json!({"type": "object", "properties": {
                    "state": {"type": "string", "enum": ["progress", "done", "blocked"], "description": "Where the work stands."},
                    "summary": {"type": "string", "description": "What happened, in a line or two: what changed, what you ran, what you need."},
                    "pr": {"type": "integer", "description": "The pull request number, if the report is about one."}
                }, "required": ["state", "summary"]})
            },
        },
        ToolDef {
            name: "coordination_claim_criterion",
            description: "Say an acceptance criterion of an effort is met, with the evidence: \
                a pull request, a test that passes, a finding. The criterion becomes claimed \
                and waits in the user's inbox; only the user can approve it as met, or send it \
                back. Claim only what you can show. Fails for an unknown criterion, one already \
                met, or empty evidence.",
            schema: || {
                json!({"type": "object", "properties": {
                    "criterion": {"type": "integer", "description": "The criterion's id."},
                    "evidence": {"type": "string", "description": "What shows it is met, specific enough for the user to check: PR number, test name, finding id."}
                }, "required": ["criterion", "evidence"]})
            },
        },
        ToolDef {
            name: "maps_draw",
            description: "Draw a map the user can explore in Dovetail, to explain structure, \
                flow or architecture instead of drawing ASCII diagrams. A `structure` map shows \
                what exists and how it connects: nodes of nine kinds with plain-English \
                descriptions, groups to drill into, labelled connections, and guided flows. A \
                `swimlane` map shows what happens in order: one lane per participant and one \
                flow whose steps name their lane; a lane can link a node of a structure map. \
                Send the whole map each time. Every problem comes back at once as a list to \
                fix. On success it returns `{\"id\", \"version\"}`: call again with that `id` \
                to redraw the map as a new version; a new subject is a new map. Maps are kept \
                per project and linked to your task.",
            schema: dovetail_maps::input_schema,
        },
        ToolDef {
            name: "canvas_add_card",
            description: "Put a card into an effort's incoming strip on its canvas, so the user \
                sees what you found or thought. The user decides whether it goes on the canvas; \
                you cannot place, move or delete a card. An `idea` or `question` carries its own \
                `text`. A `finding`, `source`, `decision`, `map` or `task` card points at \
                something that already exists: give its id as `reference` (a finding's or \
                source's number, a decision's number, a map's id, a task's number); the card \
                shows its title and nothing is copied. Fails for a reference that does not \
                exist. `effort` defaults to the effort the user last tagged. Returns the \
                card's id, for `canvas_link_cards`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "kind": {"type": "string", "enum": ["idea", "question", "finding", "source", "decision", "map", "task"], "description": "What the card is."},
                    "text": {"type": "string", "description": "The words of an idea or question card; leave out for the other kinds."},
                    "reference": {"type": "string", "description": "The id of the finding, source, decision, map or task the card points at; leave out for an idea or question."},
                    "effort": {"type": "integer", "description": "The effort's id; defaults to the one the user last tagged."}
                }, "required": ["kind"]})
            },
        },
        ToolDef {
            name: "canvas_link_cards",
            description: "Link two cards of one effort, such as a question to the finding that \
                answers it. Linking them again changes the label. Cards are named by the ids \
                `canvas_add_card` returned.",
            schema: || {
                json!({"type": "object", "properties": {
                    "from": {"type": "integer", "description": "The card the link starts at (the question, the parent)."},
                    "to": {"type": "integer", "description": "The card it leads to."},
                    "label": {"type": "string", "description": "What the link says, if anything."}
                }, "required": ["from", "to"]})
            },
        },
        ToolDef {
            name: "research_start_round",
            description: "Open the next round of a research task: a fan-out of subagents under \
                the user's criteria. Call it before you launch research subagents; while it is \
                open, Dovetail files each subagent's full report and the sources it actually \
                fetched or read, outside your context. Put the user's criteria in `brief`, \
                verbatim where you can. Set `kind` to `decision` when the user wants a recommendation \
                to choose by, `question` for a plain question. One round is open at a time: a \
                round requested while another is open is queued, and you are told when it \
                opens. `research_write_report` closes the open one. Returns \
                `{round, dir, status}` with status `open`, or `{round, status: \"queued\", \
                position}`: then wait for the message \"Round N for #T is now open\" before \
                you launch its subagents. Errors: unknown or archived task, an empty brief.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of an existing task (the question)."},
                    "brief": {"type": "string", "description": "The user's criteria for this round, verbatim where possible."},
                    "kind": {"type": "string", "enum": ["decision", "question"], "description": "Whether the task makes a decision (report follows the decision template) or asks a question. Optional."}
                }, "required": ["task", "brief"]})
            },
        },
        ToolDef {
            name: "research_write_report",
            description: "Store the report of a research task and close its open round. Send the \
                whole report each time; it replaces the last one. Cite findings as [F3] and \
                sources as [S12], using the ids Dovetail shows; a citation that does not exist \
                refuses the write and the error lists the ids that do. Dovetail also flags \
                factual claims without a citation and key figures without a label (Sourced, \
                Derived or Estimate) in the result, and writes the \"Evidence: N sources, M \
                independent\" line itself. A task marked as a decision needs the template \
                sections (Key figures, Options, What would change this, Open questions). \
                Returns `{written, uncited, unlabelled_figures, citations_resolved}`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of the research task."},
                    "body": {"type": "string", "description": "The whole report in markdown, citing findings as [F3] and sources as [S12]. Rewrites the previous report."}
                }, "required": ["task", "body"]})
            },
        },
        ToolDef {
            name: "research_propose",
            description: "Propose an idea or a decision from research to the user. It never \
                writes anything itself: the proposal goes to the user's inbox, and the call \
                returns `pending` at once. If the user accepts, an idea becomes a follow-up \
                task in Unsorted (or a new round), and a decision is logged with its evidence; \
                either way the answer arrives later as a message. A `decision` needs \
                `evidence`: the finding and source ids it rests on, like [\"F3\", \"S12\"]. Use \
                this instead of settling a research question in prose or with `core_log_decision`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of the research task."},
                    "kind": {"type": "string", "enum": ["idea", "decision"]},
                    "text": {"type": "string", "description": "The idea, or the decision in one line."},
                    "evidence": {"type": "array", "items": {"type": "string"}, "description": "Finding and source ids, e.g. [\"F3\", \"S12\"]. Required for a decision."},
                    "why": {"type": "string"},
                    "alternatives": {"type": "array", "items": {
                        "type": "object", "properties": {
                            "option": {"type": "string"},
                            "why_not": {"type": "string"}
                        }, "required": ["option"]}}
                }, "required": ["task", "kind", "text"]})
            },
        },
        ToolDef {
            name: "research_get",
            description: "Read one stored research document, so you can cite it without keeping \
                it in context: the latest `report`, a round's brief (`round:N`), a `finding:N` \
                or a `source:N` (the numbers in [F3] and [S12]). Long documents are cut, with \
                the file's path at the end.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of the research task."},
                    "what": {"type": "string", "description": "`report`, `round:N`, `finding:N` or `source:N`."}
                }, "required": ["task", "what"]})
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use dovetail_api::tools::TOOLS;

    use super::*;

    #[test]
    fn every_tool_has_one_definition_with_an_object_schema() {
        let defs = tool_defs();
        let mut names: Vec<&str> = defs.iter().map(|d| d.name).collect();
        let mut shared: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        names.sort_unstable();
        shared.sort_unstable();
        assert_eq!(
            names, shared,
            "tool_defs() must follow dovetail_api::tools::TOOLS"
        );
        for def in defs {
            assert_eq!((def.schema)()["type"], "object", "{}", def.name);
        }
    }

    #[test]
    fn tools_list_shows_the_suites_tools_then_their_old_names() {
        let names = |suites: &[Suite]| -> Vec<String> {
            tools_list(suites)
                .iter()
                .map(|t| t["name"].as_str().unwrap_or_default().to_owned())
                .collect()
        };
        assert_eq!(
            names(Role::SideChat.default_suites()),
            [
                "core_ask_user",
                "core_report_status",
                "core_log_decision",
                "ask_user",
                "report_status",
                "log_decision"
            ]
        );
        let slice = names(Role::Slice.default_suites());
        assert!(slice.contains(&"maps_draw".to_owned()) && slice.contains(&"draw_map".to_owned()));
        assert!(
            !slice
                .iter()
                .any(|n| n.contains("slice") || n.starts_with("tasks_"))
        );
        let coordinator = names(Role::Coordinator.default_suites());
        assert_eq!(coordinator.len(), TOOLS.len() + 12, "{coordinator:?}");
        let alias = tools_list(&[Suite::Project])
            .into_iter()
            .find(|t| t["name"] == "start_slice")
            .expect("the old name is listed");
        assert!(
            alias["description"]
                .as_str()
                .is_some_and(|d| d.contains("`project_start_slice`"))
        );
    }
}

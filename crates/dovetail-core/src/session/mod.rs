//! Sessions: launching and resuming them, sending to them, and turning their stream into the
//! API's events (plan §4's table). The stream rules of digest §3 are law here:
//!
//! - a subagent's events carry `parent_tool_use_id` and are routed by it, never into the
//!   parent's conversation (their tree nodes arrive at M11);
//! - a `result` ends a turn, not the session: a finished-turn item is raised only when no
//!   background tasks remain;
//! - permission requests become inbox items at once and are answered in any order;
//! - session ids and transcript paths are recorded by Dovetail itself.

pub mod launch;
pub mod state;

use std::collections::HashMap;
use std::path::PathBuf;

use dovetail_api::tools::Role;
use dovetail_api::{
    ActivityKind, AgentId, AgentKind, AgentStatus, EntryKind, InboxKind, Warning, WarningId,
};
use dovetail_driver::{
    ContentBlock, DriverEvent, ExitInfo, InitInfo, PeerMessage, Session, TurnResult, UserInput,
};

use crate::recipe::Recipe;
use crate::service::{COORDINATOR, Core, Input};
use state::{Streaming, Turn};

/// A running session process and what Dovetail tracks about it.
pub(crate) struct Live {
    pub(crate) session: Box<dyn Session>,
    /// Tells this process's events from an earlier one's.
    pub(crate) generation: u64,
    pub(crate) turn: Turn,
    /// The agent's cost before this process; `result` costs count per process.
    pub(crate) base_cost: f64,
    /// Dovetail asked it to stop, so its exit is no error.
    pub(crate) stopping: bool,
    /// A slice waiting for its worktree hook; its brief is sent when the hook succeeds.
    pub(crate) setting_up: bool,
    /// The first prompt, held until setup succeeds.
    pub(crate) brief: Option<String>,
    /// The first `system/init` has arrived.
    pub(crate) inited: bool,
    /// Pending `SendMessage` calls: tool use id → (recipient name, text).
    pub(crate) sends: HashMap<String, (String, String)>,
    /// Archive the slice once the process has exited; `Some(true)` if the user already
    /// confirmed losing unsaved work.
    pub(crate) close_after: Option<bool>,
    /// Dropped to stop the setup watcher.
    pub(crate) setup_cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Core {
    /// The tools an agent gets.
    pub(crate) fn role_of(&self, id: &AgentId) -> Option<Role> {
        self.agents.get(id).map(|r| match r.agent.kind {
            AgentKind::Coordinator => Role::Coordinator,
            _ => Role::Slice,
        })
    }

    /// The agent the user calls `name`, as sessions know it (`--name`).
    pub(crate) fn agent_by_name(&self, name: &str) -> AgentId {
        self.agents
            .values()
            .find(|r| r.agent.name == name)
            .map_or_else(|| AgentId(name.to_owned()), |r| r.agent.id.clone())
    }

    /// Starts (or resumes, if it has run before) the session of agent `id` in `cwd`.
    pub(crate) fn launch(
        &mut self,
        id: &AgentId,
        cwd: PathBuf,
        worktree: Option<String>,
    ) -> Result<(), String> {
        let role = self
            .role_of(id)
            .ok_or_else(|| format!("no agent {}", id.0))?;
        let Some(row) = self.agents.get_mut(id) else {
            return Err(format!("no agent {}", id.0));
        };
        let session_id = row
            .session_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let resume = row.transcript_path.is_some();
        let row = row.clone();
        let registered = self.project.registered().is_some();
        let env = if registered && row.agent.kind == AgentKind::Slice {
            Recipe::load(&self.paths.recipe(self.project.key()))
                .map(|r| r.env.into_iter().collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let launch = launch::Launch {
            project_key: self.project.key().to_owned(),
            registered,
            agent_id: id.0.clone(),
            name: row.agent.name.clone(),
            role,
            session_id,
            resume,
            cwd,
            worktree,
            mode: state::to_cli(row.agent.permission_mode),
            task_list_id: row
                .task_list_id
                .clone()
                .unwrap_or_else(|| self.task_list_id(row.agent.task, &row.agent.name)),
            socket: self.socket.clone(),
            exe: self.options.exe.clone(),
            model: self.config.default_model.clone(),
            env,
        };
        let spec = launch::spec(&self.paths, &launch)
            .map_err(|e| format!("writing launch settings: {e}"))?;
        tracing::info!(agent = %id.0, resume, "starting session");
        let mut session = self
            .options
            .driver
            .start(spec)
            .map_err(|e| format!("starting {}: {e}", row.agent.name))?;
        let mut events = session.take_events();
        self.generation += 1;
        let generation = self.generation;
        let inputs = self.inputs.clone();
        let agent = id.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let input = Input::Driver {
                    agent: agent.clone(),
                    generation,
                    event,
                };
                if inputs.send(input).is_err() {
                    break;
                }
            }
        });
        self.live.insert(
            id.clone(),
            Live {
                session,
                generation,
                turn: Turn::default(),
                base_cost: row.agent.cost_usd,
                stopping: false,
                setting_up: false,
                brief: None,
                inited: false,
                sends: HashMap::new(),
                close_after: None,
                setup_cancel: None,
            },
        );
        self.save_agent(id);
        Ok(())
    }

    /// Makes sure agent `id` has a running session, resuming it if needed.
    pub(crate) fn ensure_live(&mut self, id: &AgentId) -> Result<(), String> {
        if self.live.contains_key(id) {
            return Ok(());
        }
        let row = self
            .agents
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no agent {}", id.0))?;
        match row.agent.kind {
            AgentKind::Coordinator => {
                let cwd = self.project.repo_path().to_path_buf();
                self.launch(id, cwd, None)
            }
            AgentKind::Slice => {
                if row.transcript_path.is_none() {
                    return Err(format!(
                        "slice {} never started; archive it and start a new one",
                        row.agent.name
                    ));
                }
                // Resume where it ran: its worktree, or the shared folder (ADR 0005).
                let cwd = row.agent.worktree.as_ref().map_or_else(
                    || self.project.repo_path().to_path_buf(),
                    |w| w.path.clone(),
                );
                self.launch(id, cwd, None)
            }
            AgentKind::Subagent { .. } => Err("subagents have no session of their own".into()),
        }
    }

    /// Sends text to an agent's session and records `shown` in its conversation. Starts a turn.
    pub(crate) fn send_text(
        &mut self,
        id: &AgentId,
        text: &str,
        shown: EntryKind,
        shown_text: String,
    ) -> Result<(), String> {
        self.ensure_live(id)?;
        let Some(live) = self.live.get_mut(id) else {
            return Err(format!("{} is not running", id.0));
        };
        live.session
            .send(UserInput::text(text))
            .map_err(|e| e.to_string())?;
        live.turn.running = true;
        self.add_entry(id, shown, shown_text);
        self.refresh_status(id);
        Ok(())
    }

    /// The user's prompt.
    pub(crate) async fn prompt(&mut self, id: &AgentId, text: String) {
        if self.live.get(id).is_some_and(|l| l.setting_up) {
            self.fail("this slice is still setting up");
            return;
        }
        // A failed session gets another go when the user writes to it.
        self.update_agent(id, |row| {
            if row.agent.status == AgentStatus::Error {
                row.agent.status = AgentStatus::Idle;
            }
        });
        if let Err(message) = self.send_text(id, &text, EntryKind::User, text.clone()) {
            self.fail(message);
        }
    }

    /// A message from Dovetail itself (a slice's ready signal, an answer that arrives later),
    /// shown as a notice.
    pub(crate) fn send_note(&mut self, id: &AgentId, text: &str) {
        if let Err(message) =
            self.send_text(id, text, EntryKind::Notice, format!("Dovetail: {text}"))
        {
            tracing::warn!(agent = %id.0, "could not send a note: {message}");
        }
    }

    /// Tells the coordinator something at once (ADR 0004's signals: slice ready, setup
    /// failed, start approved or denied). A coordinator that is idle, or not running, starts a
    /// turn for it (decided 2026-11-06).
    pub(crate) fn tell_coordinator(&mut self, text: String) {
        self.send_note(&AgentId(COORDINATOR.into()), &text);
    }

    pub(crate) fn interrupt(&mut self, id: &AgentId) {
        let Some(live) = self.live.get_mut(id) else {
            self.fail(format!("{} is not running", id.0));
            return;
        };
        if !live.turn.running {
            self.fail(format!("{} is not mid-turn", id.0));
            return;
        }
        match live.session.interrupt() {
            Ok(()) => live.turn.interrupting = true,
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// `<leader>sm`: the next mode, switched on the live process with no restart. A session not
    /// running starts in it next time.
    pub(crate) fn cycle_mode(&mut self, id: &AgentId) {
        let Some(row) = self.agents.get(id) else {
            self.fail(format!("no agent {}", id.0));
            return;
        };
        // Subagents share their parent's session (B20).
        let target = match (&row.agent.kind, &row.agent.parent) {
            (AgentKind::Subagent { .. }, Some(parent)) => parent.clone(),
            _ => id.clone(),
        };
        let Some(current) = self.agents.get(&target).map(|r| r.agent.permission_mode) else {
            return;
        };
        let next = state::next_mode(current);
        if let Some(live) = self.live.get(&target)
            && let Err(e) = live.session.set_permission_mode(state::to_cli(next))
        {
            self.fail(e.to_string());
            return;
        }
        // The badge follows at once; `system/status` confirms it.
        self.update_agent(&target, |row| row.agent.permission_mode = next);
    }

    /// One event from a session.
    pub(crate) async fn on_driver(&mut self, agent: AgentId, generation: u64, event: DriverEvent) {
        if self.live.get(&agent).map(|l| l.generation) != Some(generation) {
            return; // an earlier process of this agent
        }
        // Rule 1: a subagent's events are its own (M11 gives them tree nodes).
        if event.origin().is_some_and(|o| !o.is_main()) {
            tracing::trace!(agent = %agent.0, "subagent event");
            return;
        }
        match event {
            DriverEvent::Init(init) => self.on_init(&agent, init),
            DriverEvent::Status {
                permission_mode: Some(mode),
                ..
            } => {
                if let Some(mode) = state::from_cli(&mode) {
                    self.update_agent(&agent, |row| row.agent.permission_mode = mode);
                }
            }
            DriverEvent::TextDelta {
                message_id, text, ..
            } => {
                let entry = self.streaming_entry(&agent, message_id);
                self.queue_delta(&agent, entry, &text);
            }
            DriverEvent::Assistant(message) => self.on_assistant(&agent, message),
            DriverEvent::ToolResult(result) => {
                let send = self
                    .live
                    .get_mut(&agent)
                    .and_then(|l| l.sends.remove(&result.tool_use_id));
                if let Some((to, text)) = send
                    && !result.is_error
                {
                    let msg_id = result
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("msg_id"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                    let to_id = self.agent_by_name(&to);
                    let from = self.agents.get(&agent).map(|r| r.agent.name.clone());
                    self.activity(
                        ActivityKind::Message,
                        Some(&agent),
                        format!("{} → {to}", from.unwrap_or_default()),
                    );
                    self.message(agent.clone(), to_id, text, msg_id);
                }
            }
            DriverEvent::UserMessage {
                message_origin: Some(origin),
                ..
            } => {
                if let Some(peer) = PeerMessage::from_origin(&origin) {
                    let from = self.agent_by_name(&peer.name);
                    self.notice(&agent, format!("Message from {}: {}", peer.name, peer.body));
                    self.message(from, agent.clone(), peer.body, peer.msg_id);
                }
            }
            DriverEvent::BackgroundTasksChanged(tasks) => {
                if let Some(live) = self.live.get_mut(&agent) {
                    live.turn.background = tasks.len();
                }
                self.refresh_status(&agent);
            }
            DriverEvent::TurnResult(result) => self.on_result(&agent, result),
            DriverEvent::PermissionRequest(request) => self.permission_request(&agent, request),
            DriverEvent::PermissionCancelled(control) => {
                self.permission_cancelled(&agent, &control);
            }
            DriverEvent::PermissionDenied {
                tool_name, message, ..
            } => {
                let tool = tool_name.unwrap_or_else(|| "a tool".into());
                let why = message.map(|m| format!(": {m}")).unwrap_or_default();
                self.notice(&agent, format!("{tool} was denied automatically{why}"));
            }
            DriverEvent::ControlResponse {
                success: false,
                response,
                ..
            } => {
                self.fail(format!("{} refused a request: {response}", agent.0));
            }
            DriverEvent::ControlRequest { raw, .. } => {
                tracing::warn!(agent = %agent.0, "unhandled control request: {raw}");
            }
            DriverEvent::Exited(info) => self.on_exit(&agent, info),
            _ => {}
        }
    }

    fn on_init(&mut self, agent: &AgentId, init: InitInfo) {
        let Some(live) = self.live.get_mut(agent) else {
            return;
        };
        // Claude Code sends an init at the start of every turn, including one it starts itself.
        live.turn.running = true;
        let first = !live.inited;
        live.inited = true;
        let setting_up = live.setting_up;
        if first {
            let cwd = init
                .cwd
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.project.repo_path().to_path_buf());
            let session_id = self
                .agents
                .get(agent)
                .and_then(|r| r.session_id.clone())
                .unwrap_or_else(|| init.session_id.clone());
            if session_id != init.session_id {
                tracing::warn!(agent = %agent.0, ours = %session_id, theirs = %init.session_id, "session id differs");
            }
            let transcript = launch::transcript_path(&cwd, &session_id);
            let mode = init.permission_mode.as_deref().and_then(state::from_cli);
            let mut warn = None;
            self.update_agent(agent, |row| {
                row.transcript_path = Some(transcript);
                // Decision 15: if init reports anything but auto when auto was asked, run as
                // manual and say so.
                if row.agent.permission_mode == dovetail_api::PermissionMode::Auto
                    && mode != Some(dovetail_api::PermissionMode::Auto)
                {
                    warn = Some(format!(
                        "{} started in {} mode, not auto: running as manual",
                        row.agent.name,
                        init.permission_mode.as_deref().unwrap_or("an unknown")
                    ));
                    row.agent.permission_mode = dovetail_api::PermissionMode::Manual;
                }
            });
            if let Some(text) = warn {
                let warning = Warning {
                    id: WarningId(format!("mode-{}", agent.0)),
                    text,
                };
                self.warnings.retain(|w| w.id != warning.id);
                self.warnings.push(warning.clone());
                self.emit(dovetail_api::Event::WarningSet(warning));
            }
            // A slice whose init already runs in its worktree is set up.
            if setting_up {
                let expected = self
                    .agents
                    .get(agent)
                    .and_then(|r| r.agent.worktree.as_ref().map(|w| w.path.clone()));
                if expected.is_some_and(|w| {
                    Some(w.as_path()) == init.cwd.as_deref().map(std::path::Path::new)
                }) {
                    self.slice_ready(agent);
                }
            }
        }
        self.refresh_status(agent);
    }

    /// The assistant entry streaming text for `message_id`, created if this is new text.
    fn streaming_entry(
        &mut self,
        agent: &AgentId,
        message_id: Option<String>,
    ) -> dovetail_api::EntryId {
        if let Some(s) = self
            .live
            .get(agent)
            .and_then(|l| l.turn.streaming.as_ref())
            .filter(|s| s.message_id == message_id)
        {
            return s.entry;
        }
        let entry = self.add_entry(agent, EntryKind::Assistant, String::new());
        if let Some(live) = self.live.get_mut(agent) {
            live.turn.streaming = Some(Streaming {
                message_id,
                entry,
                confirmed: String::new(),
            });
        }
        entry
    }

    fn on_assistant(&mut self, agent: &AgentId, message: dovetail_driver::AssistantMessage) {
        let mut texts = Vec::new();
        let mut tools = Vec::new();
        for block in &message.blocks {
            match block {
                ContentBlock::Text(text) if !text.is_empty() => texts.push(text.clone()),
                ContentBlock::ToolUse { id, name, input } => {
                    tools.push((id.clone(), name.clone(), input.clone()));
                }
                _ => {}
            }
        }
        if !texts.is_empty() {
            let text = texts.join("\n\n");
            let entry = self.streaming_entry(agent, message.message_id.clone());
            let confirmed = match self
                .live
                .get_mut(agent)
                .and_then(|l| l.turn.streaming.as_mut())
            {
                Some(s) => {
                    if !s.confirmed.is_empty() {
                        s.confirmed.push_str("\n\n");
                    }
                    s.confirmed.push_str(&text);
                    s.confirmed.clone()
                }
                None => text,
            };
            // The complete message wins over the deltas; it is also what gets stored.
            self.set_entry_text(agent, entry, confirmed.clone());
            if let Some(live) = self.live.get_mut(agent) {
                live.turn.last_text = confirmed;
            }
        }
        for (id, name, input) in tools {
            if name == "SendMessage" {
                let field = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_owned);
                if let (Some(to), Some(text)) = (
                    field("to").or_else(|| field("recipient")),
                    field("message").or_else(|| field("content")),
                ) && let Some(live) = self.live.get_mut(agent)
                {
                    live.sends.insert(id, (to, text));
                }
            }
            let (entry, line) = match self.live.get_mut(agent) {
                Some(live) => {
                    // A tool call ends the text before it: later text is a new entry.
                    live.turn.streaming = None;
                    live.turn.tools.add(&name);
                    (live.turn.tools.entry, live.turn.tools.line())
                }
                None => continue,
            };
            match entry {
                Some(entry) => self.set_entry_text(agent, entry, line),
                None => {
                    let entry = self.add_entry(agent, EntryKind::ToolSummary, line);
                    if let Some(live) = self.live.get_mut(agent) {
                        live.turn.tools.entry = Some(entry);
                    }
                }
            }
        }
    }

    fn on_result(&mut self, agent: &AgentId, result: TurnResult) {
        let Some(live) = self.live.get_mut(agent) else {
            return;
        };
        let interrupted = live.turn.interrupting || result.subtype == "error_during_execution";
        let base = live.base_cost;
        let preview: String = live.turn.last_text.chars().take(200).collect();
        live.turn.end();
        let background = live.turn.background;
        if let Some(cost) = result.total_cost_usd {
            self.update_agent(agent, |row| row.agent.cost_usd = base + cost);
        }
        for denial in &result.permission_denials {
            let what = denial
                .tool_input
                .get("command")
                .or_else(|| denial.tool_input.get("file_path"))
                .and_then(|v| v.as_str())
                .map(|s| format!(" `{s}`"))
                .unwrap_or_default();
            self.notice(agent, format!("Not allowed: {}{what}", denial.tool_name));
        }
        if interrupted {
            self.notice(agent, "Interrupted");
        } else if result.is_error {
            let detail = result.result.unwrap_or(result.subtype);
            self.notice(agent, format!("The turn ended with an error: {detail}"));
        }
        // Rule 2: with background work left, this is not the end (ADR 0001: no item yet).
        if background == 0 {
            self.finished_turn(agent, preview);
        }
        self.refresh_status(agent);
    }

    fn on_exit(&mut self, agent: &AgentId, info: ExitInfo) {
        let Some(live) = self.live.remove(agent) else {
            return;
        };
        self.drop_session_items(agent);
        if live.setting_up {
            let detail = info
                .stderr_tail
                .last()
                .cloned()
                .unwrap_or_else(|| exit_text(&info));
            self.setup_failed(agent, format!("the session exited during setup: {detail}"));
        } else if !live.stopping && !info.success {
            let mut text = format!("session ended unexpectedly ({})", exit_text(&info));
            if !info.stderr_tail.is_empty() {
                text.push_str(":\n");
                text.push_str(&info.stderr_tail.join("\n"));
            }
            self.raise(agent, InboxKind::Error { text }, crate::inbox::Waiter::None);
            self.update_agent(agent, |row| row.agent.status = AgentStatus::Error);
            self.activity(
                ActivityKind::Lifecycle,
                Some(agent),
                format!("{} crashed", agent.0),
            );
        }
        if let Some(confirmed) = live.close_after {
            self.finish_close(agent, confirmed);
        }
        self.refresh_status(agent);
    }
}

fn exit_text(info: &ExitInfo) -> String {
    match (info.code, info.signal) {
        (Some(code), _) => format!("exit code {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "unknown status".into(),
    }
}

//! `ClaudeDriver`: Claude Code over its NDJSON stream protocol. Hand-written NDJSON layer on
//! tokio and serde_json (spike S1 decision): the shared process plumbing ([`crate::process`]) plus
//! this module's stdout reader, and the Claude Code pieces beside it: [`parse`] (lines to
//! events), `switch` (the live model switch), `mcp` (the MCP menu), `args` (argv), `usage`
//! (`get_usage`) and [`transcript`] (the native JSONL transcript after a take-over).

mod args;
mod mcp;
pub mod parse;
mod reader;
mod switch;
pub mod transcript;
mod usage;

pub use args::ClaudeOptions;

use crate::event::*;
use crate::process::{self, Ctl, Log, WriterMsg};
use crate::{
    ControlAnswer, DriverError, PermissionMode, Session, SessionDriver, SessionSpec, UserInput,
};
use dovetail_api::agents::{
    mcp::McpRequest,
    model::{DriverKind, ReasoningEffort},
};
use mcp::McpCalls;
use reader::Reader;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use switch::Switches;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// How long an MCP menu request may wait for its answer.
const MCP_TIMEOUT: Duration = Duration::from_secs(30);

/// Spawns real `claude` processes.
#[derive(Debug, Clone)]
pub struct ClaudeDriver {
    program: OsString,
}

impl Default for ClaudeDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeDriver {
    /// Uses `claude` from `PATH`.
    pub fn new() -> Self {
        Self {
            program: "claude".into(),
        }
    }

    /// Uses a specific binary (tests can point this at a fake).
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The full command line for a spec, `systemd-run` prefix included. Public so the core can
    /// log it and tests can assert it.
    pub fn command_line(&self, spec: &SessionSpec) -> (OsString, Vec<OsString>) {
        spec.wrap(&self.program, spec.claude_args())
    }
}

impl SessionDriver for ClaudeDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Claude
    }

    fn usage(&self, query: crate::UsageQuery) -> crate::UsageFuture {
        Box::pin(usage::usage(self.program.clone(), query))
    }

    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| DriverError::NoRuntime)?;
        let _guard = handle.enter();
        let (program, args) = self.command_line(&spec);
        let log = Log::for_spec(&spec)?;
        let child = process::spawn(program, args, &spec)?;

        let (events_tx, events_rx) = unbounded_channel();
        let (writer_tx, writer_rx) = unbounded_channel();
        let (ctl_tx, ctl_rx) = unbounded_channel();
        let pending: Arc<Mutex<HashMap<String, Value>>> = Arc::default();
        let switches: Arc<Mutex<Switches>> = Arc::default();
        let mcp: Arc<Mutex<McpCalls>> = Arc::default();

        tokio::spawn(process::write_stdin(child.stdin, writer_rx, log.clone()));
        let reader = tokio::spawn(reader::read_stdout(
            child.stdout,
            Reader {
                events: events_tx.clone(),
                writer: writer_tx.clone(),
                pending: pending.clone(),
                switches: switches.clone(),
                mcp: mcp.clone(),
                log: log.clone(),
            },
        ));
        let stderr_task = tokio::spawn(process::read_stderr(child.stderr, log));
        tokio::spawn(process::supervise(
            child.child,
            ctl_rx,
            writer_tx.clone(),
            reader,
            stderr_task,
            events_tx.clone(),
        ));

        // Asks for the slash command list before any prompt; the answer comes back as
        // `DriverEvent::Commands`. No model call is made for it.
        let _ = writer_tx.send(WriterMsg::Line(json!({
            "type": "control_request",
            "request_id": parse::INITIALIZE_ID,
            "request": {"subtype": "initialize"},
        })));
        Ok(Box::new(ClaudeSession {
            session_id: spec.session_id,
            pid: child.pid,
            writer: writer_tx,
            ctl: ctl_tx,
            events: Some(events_rx),
            pending,
            switches,
            mcp,
            events_tx,
            runtime: handle,
            counter: AtomicU64::new(0),
        }))
    }
}

/// A live `claude` process.
struct ClaudeSession {
    session_id: String,
    pid: Option<u32>,
    writer: UnboundedSender<WriterMsg>,
    ctl: UnboundedSender<Ctl>,
    events: Option<UnboundedReceiver<DriverEvent>>,
    /// Inputs of pending `can_use_tool` requests, to echo back when the answer has none.
    pending: Arc<Mutex<HashMap<String, Value>>>,
    /// Model switches in flight; the stdout reader finishes them.
    switches: Arc<Mutex<Switches>>,
    /// MCP menu requests in flight.
    mcp: Arc<Mutex<McpCalls>>,
    /// For failing an MCP request that timed out.
    events_tx: UnboundedSender<DriverEvent>,
    /// The runtime the timeouts run on.
    runtime: tokio::runtime::Handle,
    counter: AtomicU64,
}

impl ClaudeSession {
    fn write(&self, v: Value) -> Result<(), DriverError> {
        self.writer
            .send(WriterMsg::Line(v))
            .map_err(|_| DriverError::SessionClosed)
    }

    fn control(&self, request: Value) -> Result<(), DriverError> {
        let id = format!("dt_{}", self.counter.fetch_add(1, Ordering::SeqCst));
        self.write(json!({"type": "control_request", "request_id": id, "request": request}))
    }
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        // Dropping the handle must not leak a process: kill; the supervisor reaps.
        let _ = self.ctl.send(Ctl::Kill);
    }
}

impl Session for ClaudeSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn send(&self, input: UserInput) -> Result<(), DriverError> {
        self.write(input.to_stream_json())
    }

    fn interrupt(&self) -> Result<(), DriverError> {
        self.control(json!({"subtype": "interrupt"}))
    }

    fn set_permission_mode(&self, mode: PermissionMode) -> Result<(), DriverError> {
        self.control(json!({"subtype": "set_permission_mode", "mode": mode.as_cli()}))
    }

    fn set_model(
        &self,
        model: Option<&str>,
        effort: Option<ReasoningEffort>,
    ) -> Result<SwitchId, DriverError> {
        let (switch, line) = self
            .switches
            .lock()
            .map_err(|_| DriverError::SessionClosed)?
            .begin(model, effort);
        self.write(line)?;
        Ok(switch)
    }

    fn mcp(&self, request: McpRequest) -> Result<(), DriverError> {
        let (id, line) = self
            .mcp
            .lock()
            .map_err(|_| DriverError::SessionClosed)?
            .begin(request);
        if let Err(e) = self.write(line) {
            // Nothing will answer: forget it, and tell the caller through the error.
            if let Ok(mut calls) = self.mcp.lock() {
                let _ = calls.fail(&id, "the session is closed");
            }
            return Err(e);
        }
        // Reconnecting and logging in can be slow, but not forever.
        let (calls, events) = (self.mcp.clone(), self.events_tx.clone());
        self.runtime.spawn(async move {
            tokio::time::sleep(MCP_TIMEOUT).await;
            let event = calls
                .lock()
                .ok()
                .and_then(|mut c| c.fail(&id, "no answer from Claude Code in 30 s"));
            if let Some(event) = event {
                let _ = events.send(event);
            }
        });
        Ok(())
    }

    fn answer_control(&self, id: ControlId, answer: ControlAnswer) -> Result<(), DriverError> {
        let original = self
            .pending
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&id.0))
            .ok_or_else(|| DriverError::UnknownControl(id.clone()))?;
        let response = match answer {
            ControlAnswer::Allow {
                updated_input,
                updated_permissions,
            } => {
                let mut r =
                    json!({"behavior": "allow", "updatedInput": updated_input.unwrap_or(original)});
                if !updated_permissions.is_empty() {
                    r["updatedPermissions"] = Value::Array(updated_permissions);
                }
                r
            }
            ControlAnswer::Deny { message, interrupt } => {
                json!({"behavior": "deny", "message": message, "interrupt": interrupt})
            }
        };
        self.write(json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": id.0, "response": response},
        }))
    }

    fn take_events(&mut self) -> UnboundedReceiver<DriverEvent> {
        self.events.take().unwrap_or_else(|| unbounded_channel().1)
    }

    fn shutdown(&self, grace: Duration) {
        let _ = self.ctl.send(Ctl::Shutdown(grace));
    }

    fn kill(&self) {
        let _ = self.ctl.send(Ctl::Kill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemdScope;

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_capped_session_runs_under_a_collected_scope() {
        let mut spec = SessionSpec::new("s1", "/tmp");
        spec.name = Some("cost $$ and $HOME".into());
        spec.systemd_scope = Some(SystemdScope::capped(
            "dovetail-p-coordinator",
            4 << 30,
            true,
        ));
        let (program, args) = ClaudeDriver::with_program("/opt/$bin/claude").command_line(&spec);
        let args = strings(&args);
        assert_eq!(program, "systemd-run");
        assert_eq!(
            args[..12],
            [
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "--unit=dovetail-p-coordinator",
                "-p",
                "MemoryMax=4294967296",
                "-p",
                "MemorySwapMax=0",
                "-p",
                "OOMPolicy=continue",
                "--",
            ]
        );
        // systemd-run turns `$$` into `$` (S4), so every `$` after `--` is doubled.
        assert_eq!(args[12], "/opt/$$bin/claude");
        let name = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name + 1], "cost $$$$ and $$HOME");
    }

    #[test]
    fn without_escaping_arguments_pass_unchanged() {
        let mut spec = SessionSpec::new("s1", "/tmp");
        spec.name = Some("a$b".into());
        spec.systemd_scope = Some(SystemdScope::capped("u", 1, false));
        let (_, args) = ClaudeDriver::new().command_line(&spec);
        assert!(strings(&args).contains(&"a$b".to_string()));
    }

    #[test]
    fn an_uncapped_session_runs_claude_directly() {
        let spec = SessionSpec::new("s1", "/tmp");
        let (program, args) = ClaudeDriver::new().command_line(&spec);
        assert_eq!(program, "claude");
        assert_eq!(strings(&args), spec.claude_args());
    }

    /// Take-over: native `claude` gets everything headless gets but the stream flags, always
    /// resumes, and never asks for a worktree; under caps it runs in the same scope unit.
    #[test]
    fn native_runs_resume_with_the_same_flags_and_no_stream_ones() {
        let mut spec = SessionSpec::new("s1", "/tmp");
        spec.name = Some("slice-4".into());
        spec.model = Some("opus".into());
        spec.claude.settings_path = Some("/run/settings.json".into());
        spec.claude.mcp_config_path = Some("/run/mcp.json".into());
        spec.append_system_prompt_file = Some("/roles/slice.md".into());
        spec.claude.allowed_tools = vec!["mcp__dovetail__core_ask_user".into()];
        spec.claude.worktree = Some("slice-4".into());
        spec.permission_mode = PermissionMode::AcceptEdits;
        let native = spec.native_args();
        assert_eq!(
            native,
            [
                "--permission-mode",
                "acceptEdits",
                "--resume",
                "s1",
                "--name",
                "slice-4",
                "--model",
                "opus",
                "--settings",
                "/run/settings.json",
                "--mcp-config",
                "/run/mcp.json",
                "--append-system-prompt-file",
                "/roles/slice.md",
                "--allowedTools",
                "mcp__dovetail__core_ask_user",
            ]
        );
        // Every native flag is one headless passes too (bar the resume and worktree).
        let headless = spec.claude_args();
        for flag in native.iter().filter(|a| a.starts_with("--")) {
            assert!(
                flag == "--resume" || headless.contains(flag),
                "{flag} is not a headless flag"
            );
        }
        spec.systemd_scope = Some(SystemdScope::capped("dovetail-p-slice-4", 1, true));
        let (program, args) = spec.wrap(std::ffi::OsStr::new("claude"), spec.native_args());
        assert_eq!(program, "systemd-run");
        let args = strings(&args);
        assert!(args.contains(&"--unit=dovetail-p-slice-4".to_string()));
        let end = args.iter().position(|a| a == "--").expect("scope args end");
        assert_eq!(args[end + 1], "claude");
        assert!(!args[end + 2..].contains(&"-p".to_string()), "{args:?}");
    }
}

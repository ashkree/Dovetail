//! `AgyDriver`: Antigravity's CLI over the Agent Client Protocol (`agy acp`, JSON-RPC 2.0 on
//! stdio; spike `docs/spikes/agy-headless.md`). The shared process plumbing
//! ([`crate::process`]) runs the child; [`conversation`] holds the protocol state with no I/O,
//! and this module only moves lines between the two. The pieces: `args` (argv and the
//! `session/new` params), `rpc` (request ids), `turn` (updates to events), `models` (agy's ids
//! as model and effort), `permission` (permission requests and answers).

mod args;
pub(crate) mod conversation;
mod models;
mod permission;
mod rpc;
mod turn;
mod usage;

pub use args::{AgyOptions, McpServerSpec};

use crate::event::*;
use crate::process::{self, Ctl, Log, WriterMsg};
use crate::{
    ControlAnswer, DriverError, PermissionMode, Session, SessionDriver, SessionSpec, UserInput,
};
use conversation::{Conversation, Plan, Step};
use dovetail_api::agents::{
    mcp::{McpOutcome, McpRequest},
    model::{DriverKind, ReasoningEffort},
};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Spawns real `agy acp` processes, one per session.
#[derive(Debug, Clone)]
pub struct AgyDriver {
    program: OsString,
}

impl Default for AgyDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl AgyDriver {
    /// Uses `agy` from `PATH`.
    pub fn new() -> Self {
        Self {
            program: "agy".into(),
        }
    }

    /// `agy` from `PATH`, or `None` when no executable of that name is there: a machine without
    /// agy then has no agy driver at all.
    pub fn if_installed() -> Option<Self> {
        let path = std::env::var_os("PATH")?;
        on_path("agy", &path).then(Self::new)
    }

    /// Uses a specific binary.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// The full command line for a spec, `systemd-run` prefix included.
    pub fn command_line(&self, spec: &SessionSpec) -> (OsString, Vec<OsString>) {
        spec.wrap(&self.program, spec.agy_args())
    }
}

/// What the reader and the handle share: the conversation, and where its output goes.
struct Link {
    conversation: Mutex<Conversation>,
    events: UnboundedSender<DriverEvent>,
    writer: UnboundedSender<WriterMsg>,
}

impl Link {
    /// Sends a step's events and lines on.
    fn emit(&self, step: Step) -> Result<(), DriverError> {
        for event in step.events {
            let _ = self.events.send(event);
        }
        for line in step.lines {
            self.writer
                .send(WriterMsg::Line(line))
                .map_err(|_| DriverError::SessionClosed)?;
        }
        if step.close {
            let _ = self.writer.send(WriterMsg::Close);
        }
        Ok(())
    }

    /// Runs `f` on the conversation and sends its step on.
    fn with<T>(&self, f: impl FnOnce(&mut Conversation) -> (T, Step)) -> Result<T, DriverError> {
        let (value, step) = {
            let mut c = self
                .conversation
                .lock()
                .map_err(|_| DriverError::SessionClosed)?;
            f(&mut c)
        };
        self.emit(step)?;
        Ok(value)
    }

    /// One stdout line; `false` once nobody listens any more.
    fn on_line(&self, line: &str) -> bool {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return self
                .events
                .send(DriverEvent::Unknown(json!({"_non_json": line})))
                .is_ok();
        };
        let _ = self.with(|c| ((), c.received(&msg)));
        !self.events.is_closed()
    }
}

impl SessionDriver for AgyDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Agy
    }

    fn usage(&self, query: crate::UsageQuery) -> crate::UsageFuture {
        Box::pin(usage::usage(self.program.clone(), query))
    }

    fn models(&self, query: crate::UsageQuery) -> crate::ModelsFuture {
        Box::pin(models::list(self.program.clone(), query))
    }

    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| DriverError::NoRuntime)?;
        let _guard = handle.enter();
        let role_prompt = spec
            .append_system_prompt_file
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok());
        let (program, args) = self.command_line(&spec);
        let log = Log::for_spec(&spec)?;
        let child = process::spawn(program, args, &spec)?;

        let (events_tx, events_rx) = unbounded_channel();
        let (writer_tx, writer_rx) = unbounded_channel();
        let (ctl_tx, ctl_rx) = unbounded_channel();
        let link = Arc::new(Link {
            conversation: Mutex::new(Conversation::new(Plan::of(&spec, role_prompt.as_deref()))),
            events: events_tx.clone(),
            writer: writer_tx.clone(),
        });

        tokio::spawn(process::write_stdin(child.stdin, writer_rx, log.clone()));
        let reader_link = link.clone();
        let reader_log = log.clone();
        let reader = tokio::spawn(async move {
            process::read_lines(child.stdout, reader_log, |line| reader_link.on_line(line)).await;
        });
        let stderr_task = tokio::spawn(process::read_stderr(child.stderr, log));
        tokio::spawn(process::supervise(
            child.child,
            ctl_rx,
            writer_tx,
            reader,
            stderr_task,
            events_tx,
        ));
        link.with(|c| ((), Step::line(c.start())))?;
        Ok(Box::new(AgySession {
            session_id: spec.session_id,
            pid: child.pid,
            link,
            ctl: ctl_tx,
            events: Some(events_rx),
        }))
    }
}

/// A live `agy acp` process.
struct AgySession {
    /// Dovetail's id for the session; agy's own id arrives in the first `Init`.
    session_id: String,
    pid: Option<u32>,
    link: Arc<Link>,
    ctl: UnboundedSender<Ctl>,
    events: Option<UnboundedReceiver<DriverEvent>>,
}

impl Drop for AgySession {
    fn drop(&mut self) {
        // Dropping the handle must not leak a process: kill; the supervisor reaps.
        let _ = self.ctl.send(Ctl::Kill);
    }
}

impl Session for AgySession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn send(&self, input: UserInput) -> Result<(), DriverError> {
        self.link.with(|c| ((), c.send(&input)))
    }

    fn interrupt(&self) -> Result<(), DriverError> {
        self.link.with(|c| ((), c.interrupt()))
    }

    fn set_permission_mode(&self, mode: PermissionMode) -> Result<(), DriverError> {
        self.link.with(|c| ((), c.set_mode(mode)))
    }

    fn set_model(
        &self,
        model: Option<&str>,
        effort: Option<ReasoningEffort>,
    ) -> Result<SwitchId, DriverError> {
        self.link.with(|c| c.set_model(model, effort))
    }

    fn mcp(&self, request: McpRequest) -> Result<(), DriverError> {
        // ACP has no MCP menu: answer at once, as a refusal the menu shows.
        let _ = self.link.events.send(DriverEvent::McpReply {
            request,
            outcome: McpOutcome::Failed("agy has no MCP menu".into()),
        });
        Ok(())
    }

    fn answer_control(&self, id: ControlId, answer: ControlAnswer) -> Result<(), DriverError> {
        let step = self
            .link
            .conversation
            .lock()
            .map_err(|_| DriverError::SessionClosed)?
            .answer(&id, &answer)?;
        self.link.emit(step)
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

/// Whether `program` is an executable file in one of the `path` folders.
fn on_path(program: &str, path: &std::ffi::OsStr) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(program))
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_is_found_only_as_an_executable_file_on_the_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dvt-agy-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = std::env::join_paths([&dir]).expect("path");
        assert!(!on_path("agy", &path), "nothing there yet");
        let file = dir.join("agy");
        std::fs::write(&file, "#!/bin/sh\n").expect("file");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("mode");
        assert!(!on_path("agy", &path), "not executable");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("mode");
        assert!(on_path("agy", &path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_capped_session_runs_agy_acp_under_its_scope() {
        let mut spec = SessionSpec::new("s1", "/tmp");
        spec.agy.settings_dir = Some("/run/a".into());
        spec.systemd_scope = Some(crate::SystemdScope::capped("dovetail-p-slice", 1, false));
        let (program, args) = AgyDriver::new().command_line(&spec);
        assert_eq!(program, "systemd-run");
        let args: Vec<_> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let end = args.iter().position(|a| a == "--").expect("scope args end");
        assert_eq!(args[end + 1..], ["agy", "acp", "--cli_dir_name", "/run/a"]);
    }
}

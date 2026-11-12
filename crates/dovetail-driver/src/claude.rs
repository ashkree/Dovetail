//! `ClaudeDriver`: the real thing. Hand-written NDJSON layer on tokio and serde_json
//! (spike S1 decision), one supervisor, one reader, one writer and one stderr task per session.

use crate::event::*;
use crate::parse;
use crate::switch::Switches;
use crate::{
    ControlAnswer, DriverError, PermissionMode, Session, SessionDriver, SessionSpec, UserInput,
};
use dovetail_api::Effort;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::Write as _;
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const STDERR_TAIL: usize = 20;
/// How long to wait for the pipes to drain after the process is gone (a background grandchild
/// can keep stdout open).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

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
    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| DriverError::NoRuntime)?;
        let _guard = handle.enter();
        let (program, args) = self.command_line(&spec);
        let log = match &spec.log_path {
            Some(p) => Some(Arc::new(Log::open(p).map_err(DriverError::Log)?)),
            None => None,
        };
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(DriverError::Spawn)?;
        let pid = child.id();
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.start_kill();
            return Err(DriverError::Spawn(std::io::Error::other(
                "child pipes missing",
            )));
        };

        let (events_tx, events_rx) = unbounded_channel();
        let (writer_tx, writer_rx) = unbounded_channel();
        let (ctl_tx, ctl_rx) = unbounded_channel();
        let pending: Arc<Mutex<HashMap<String, Value>>> = Arc::default();
        let switches: Arc<Mutex<Switches>> = Arc::default();

        tokio::spawn(write_stdin(stdin, writer_rx, log.clone()));
        let reader = tokio::spawn(read_stdout(
            stdout,
            Reader {
                events: events_tx.clone(),
                writer: writer_tx.clone(),
                pending: pending.clone(),
                switches: switches.clone(),
                log: log.clone(),
            },
        ));
        let stderr_task = tokio::spawn(read_stderr(stderr, log));
        tokio::spawn(supervise(
            child,
            ctl_rx,
            writer_tx.clone(),
            reader,
            stderr_task,
            events_tx,
        ));

        // Asks for the slash command list before any prompt; the answer comes back as
        // `DriverEvent::Commands`. No model call is made for it.
        let _ = writer_tx.send(WriterMsg::Line(json!({
            "type": "control_request",
            "request_id": crate::parse::INITIALIZE_ID,
            "request": {"subtype": "initialize"},
        })));
        // The launch model and effort, for the usage row before any switch, and what each model
        // accepts, for the model selector (task #39). Neither makes a model call.
        if let Ok(mut s) = switches.lock() {
            let _ = writer_tx.send(WriterMsg::Line(s.read_settings()));
        }
        let _ = writer_tx.send(WriterMsg::Line(crate::switch::list_models()));
        Ok(Box::new(ClaudeSession {
            session_id: spec.session_id,
            pid,
            writer: writer_tx,
            ctl: ctl_tx,
            events: Some(events_rx),
            pending,
            switches,
            counter: AtomicU64::new(0),
        }))
    }
}

/// NDJSON log in the fixture format.
struct Log {
    start: Instant,
    file: Mutex<std::fs::File>,
}

impl Log {
    fn open(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self {
            start: Instant::now(),
            file: Mutex::new(std::fs::File::create(path)?),
        })
    }

    fn write(&self, dir: &str, msg: Value) {
        let t = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = json!({"t": t, "dir": dir, "msg": msg});
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

enum WriterMsg {
    Line(Value),
    /// Close stdin so the process exits by itself.
    Close,
}

enum Ctl {
    Shutdown(Duration),
    Kill,
}

async fn write_stdin(
    mut stdin: ChildStdin,
    mut rx: UnboundedReceiver<WriterMsg>,
    log: Option<Arc<Log>>,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            WriterMsg::Line(v) => {
                if let Some(l) = &log {
                    l.write("in", v.clone());
                }
                let bytes = format!("{v}\n");
                if let Err(e) = stdin.write_all(bytes.as_bytes()).await {
                    tracing::debug!("stdin write failed: {e}");
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    tracing::debug!("stdin flush failed: {e}");
                    break;
                }
            }
            WriterMsg::Close => break,
        }
    }
    let _ = stdin.shutdown().await;
}

/// What the stdout reader shares with the session handle.
struct Reader {
    events: UnboundedSender<DriverEvent>,
    /// For the next request of a model switch, sent when the previous one is answered.
    writer: UnboundedSender<WriterMsg>,
    pending: Arc<Mutex<HashMap<String, Value>>>,
    switches: Arc<Mutex<Switches>>,
    log: Option<Arc<Log>>,
}

impl Reader {
    /// A switch step's answer becomes the next request or the switch's result; any other event
    /// passes through.
    fn route(&self, ev: DriverEvent) -> Option<DriverEvent> {
        let DriverEvent::ControlResponse {
            request_id,
            success,
            response,
        } = &ev
        else {
            return Some(ev);
        };
        let outcome = self
            .switches
            .lock()
            .ok()
            .and_then(|mut s| s.answer(request_id, *success, response));
        let Some(outcome) = outcome else {
            return Some(ev);
        };
        if let Some(line) = outcome.send {
            let _ = self.writer.send(WriterMsg::Line(line));
        }
        outcome.event
    }
}

/// Never awaits anything but the next line and an unbounded send, so a held permission request
/// cannot stall the stream.
async fn read_stdout(stdout: tokio::process::ChildStdout, reader: Reader) {
    let Reader {
        events: ref tx,
        ref pending,
        ref log,
        ..
    } = reader;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("stdout read error: {e}");
                break;
            }
        };
        if let Some(l) = &log {
            let msg = serde_json::from_str(&line).unwrap_or_else(|_| json!({"_non_json": line}));
            l.write("out", msg);
        }
        for ev in parse::map_line(&line) {
            let Some(ev) = reader.route(ev) else {
                continue;
            };
            if let DriverEvent::PermissionRequest(req) = &ev
                && let Ok(mut map) = pending.lock()
            {
                map.insert(req.id.0.clone(), req.input.clone());
            }
            if tx.send(ev).is_err() {
                return; // nobody listens any more
            }
        }
    }
}

async fn read_stderr(stderr: tokio::process::ChildStderr, log: Option<Arc<Log>>) -> Vec<String> {
    let mut tail: VecDeque<String> = VecDeque::new();
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(l) = &log {
            l.write("err", Value::String(line.clone()));
        }
        if tail.len() == STDERR_TAIL {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    tail.into()
}

/// Owns the child: waits for it, enforces shutdown deadlines and kills, then reaps and reports.
async fn supervise(
    mut child: Child,
    mut ctl: UnboundedReceiver<Ctl>,
    writer: UnboundedSender<WriterMsg>,
    reader: tokio::task::JoinHandle<()>,
    stderr_task: tokio::task::JoinHandle<Vec<String>>,
    events: UnboundedSender<DriverEvent>,
) {
    let mut deadline: Option<tokio::time::Instant> = None;
    let mut ctl_open = true;
    let status = loop {
        let dl = deadline;
        tokio::select! {
            st = child.wait() => break st,
            msg = ctl.recv(), if ctl_open => match msg {
                Some(Ctl::Shutdown(grace)) => {
                    let _ = writer.send(WriterMsg::Close);
                    deadline = Some(tokio::time::Instant::now() + grace);
                }
                Some(Ctl::Kill) | None => {
                    ctl_open = false;
                    let _ = child.start_kill();
                }
            },
            _ = tokio::time::sleep_until(dl.unwrap_or_else(tokio::time::Instant::now)), if dl.is_some() => {
                deadline = None;
                tracing::warn!("session did not exit within its grace period, killing");
                let _ = child.start_kill();
            }
        }
    };
    let _ = writer.send(WriterMsg::Close);
    let mut reader = reader;
    if tokio::time::timeout(DRAIN_TIMEOUT, &mut reader)
        .await
        .is_err()
    {
        reader.abort();
    }
    let stderr_tail = match tokio::time::timeout(DRAIN_TIMEOUT, stderr_task).await {
        Ok(Ok(tail)) => tail,
        _ => Vec::new(),
    };
    let info = match status {
        Ok(st) => ExitInfo {
            code: st.code(),
            signal: st.signal(),
            success: st.success(),
            stderr_tail,
        },
        Err(e) => {
            tracing::warn!("waiting for session process failed: {e}");
            ExitInfo {
                code: None,
                signal: None,
                success: false,
                stderr_tail,
            }
        }
    };
    let _ = events.send(DriverEvent::Exited(info));
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

    fn set_model(&self, model: &str, effort: Option<Effort>) -> Result<(), DriverError> {
        let line = self
            .switches
            .lock()
            .map_err(|_| DriverError::SessionClosed)?
            .begin(model, effort);
        self.write(line)
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
        spec.settings_path = Some("/run/settings.json".into());
        spec.mcp_config_path = Some("/run/mcp.json".into());
        spec.append_system_prompt_file = Some("/roles/slice.md".into());
        spec.allowed_tools = vec!["mcp__dovetail__core_ask_user".into()];
        spec.worktree = Some("slice-4".into());
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

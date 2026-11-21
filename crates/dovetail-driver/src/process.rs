//! The child process under a session, whichever CLI it is: spawning it with piped stdio, the
//! stdin writer, the stderr tail, the supervisor that owns the child until it is reaped, and the
//! optional NDJSON log in the fixture format. Each driver adds only its own stdout reader.

use crate::event::{DriverEvent, ExitInfo};
use crate::{DriverError, SessionSpec};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::Write as _;
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

const STDERR_TAIL: usize = 20;
/// How long to wait for the pipes to drain after the process is gone (a background grandchild
/// can keep stdout open).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// NDJSON log in the fixture format (`{"t","dir","msg"}`).
pub(crate) struct Log {
    start: Instant,
    file: Mutex<std::fs::File>,
}

impl Log {
    /// The spec's log, if it asks for one.
    pub(crate) fn for_spec(spec: &SessionSpec) -> Result<Option<Arc<Self>>, DriverError> {
        match &spec.log_path {
            Some(p) => Ok(Some(Arc::new(Self::open(p).map_err(DriverError::Log)?))),
            None => Ok(None),
        }
    }

    fn open(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self {
            start: Instant::now(),
            file: Mutex::new(std::fs::File::create(path)?),
        })
    }

    /// Appends one line; `dir` is `in`, `out` or `err`.
    pub(crate) fn write(&self, dir: &str, msg: Value) {
        let t = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let line = json!({"t": t, "dir": dir, "msg": msg});
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }

    /// Logs a stdout line: as JSON when it parses, else wrapped as `_non_json`.
    pub(crate) fn write_out(&self, line: &str) {
        let msg = serde_json::from_str(line).unwrap_or_else(|_| json!({"_non_json": line}));
        self.write("out", msg);
    }
}

/// What the stdin writer is told.
pub(crate) enum WriterMsg {
    /// One JSON line.
    Line(Value),
    /// Close stdin so the process exits by itself.
    Close,
}

/// What the supervisor is told.
pub(crate) enum Ctl {
    /// Close stdin, then kill after the grace period.
    Shutdown(Duration),
    /// Kill now.
    Kill,
}

/// A spawned child with its pipes taken.
pub(crate) struct Spawned {
    pub(crate) child: Child,
    pub(crate) pid: Option<u32>,
    pub(crate) stdin: ChildStdin,
    pub(crate) stdout: ChildStdout,
    pub(crate) stderr: ChildStderr,
}

/// Spawns `program args` in the spec's cwd with its env, every stdio piped.
pub(crate) fn spawn(
    program: OsString,
    args: Vec<OsString>,
    spec: &SessionSpec,
) -> Result<Spawned, DriverError> {
    spawn_in(program, args, &spec.cwd, &spec.env)
}

/// Spawns `program args` in `cwd` with `env` added, every stdio piped.
pub(crate) fn spawn_in(
    program: OsString,
    args: Vec<OsString>,
    cwd: &std::path::Path,
    env: &[(String, String)],
) -> Result<Spawned, DriverError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in env {
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
    Ok(Spawned {
        child,
        pid,
        stdin,
        stdout,
        stderr,
    })
}

/// Writes each line to stdin, flushed, until told to close or a write fails.
pub(crate) async fn write_stdin(
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

/// Reads stdout line by line and hands each to `on_line`, which returns `false` once nobody
/// listens any more. Never awaits anything but the next line, so a held permission request
/// cannot stall the stream.
pub(crate) async fn read_lines(
    stdout: ChildStdout,
    log: Option<Arc<Log>>,
    mut on_line: impl FnMut(&str) -> bool,
) {
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
            l.write_out(&line);
        }
        if !on_line(&line) {
            return;
        }
    }
}

/// Collects the last stderr lines (and logs each).
pub(crate) async fn read_stderr(stderr: ChildStderr, log: Option<Arc<Log>>) -> Vec<String> {
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

/// Owns the child: waits for it, enforces shutdown deadlines and kills, then reaps and reports
/// [`DriverEvent::Exited`], always last.
pub(crate) async fn supervise(
    mut child: Child,
    mut ctl: UnboundedReceiver<Ctl>,
    writer: UnboundedSender<WriterMsg>,
    reader: JoinHandle<()>,
    stderr_task: JoinHandle<Vec<String>>,
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

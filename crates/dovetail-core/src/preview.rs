//! Slice previews (ADR 0026): the recipe's `preview` command, run per slice.
//!
//! The viewer asks for a preview ([`dovetail_api::design::DesignCommand::StartPreview`]) the first time it
//! opens on a slice. The core picks a free port, runs the command in the slice's worktree with
//! the recipe's `[env]`, inside a capped scope like the slice's session when caps are on, and
//! reports its state: starting until the port accepts a connection (checked every
//! [`READY_EVERY`] for at most [`READY_WITHIN`], never afterwards), then running, or failed with
//! the last lines it printed. Previews stop when their slice is archived and when the app quits.
//! They are not restored after a restart: a process does not outlive the app.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dovetail_api::{
    Event,
    agent::{AgentId, AgentKind},
    design::{DesignEvent, OUTPUT_LINES, Preview, PreviewState},
};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::recipe::Recipe;
use crate::service::{Core, Input};

/// How often the port is tried while the command starts.
const READY_EVERY: Duration = Duration::from_millis(200);
/// How long the command gets to listen before the start counts as failed.
const READY_WITHIN: Duration = Duration::from_secs(60);

/// What a watcher tells the loop.
#[derive(Debug)]
pub(crate) enum PreviewReport {
    /// The port accepts connections.
    Ready,
    /// The process is gone (or never listened); `why` says how, then the output tail.
    Ended { why: String },
}

/// Why a preview could not start; the text is shown to the user in the viewer.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct PreviewError(String);

/// One slice's preview as the core tracks it.
struct Run {
    state: PreviewState,
    /// Which start this is, so a late report of an earlier start is ignored.
    run: u64,
    /// Asks the watcher to kill the process and reap it.
    stop: Option<oneshot::Sender<()>>,
    /// The process, which leads its own process group.
    pid: Option<u32>,
    /// The capped scope's unit, when it runs in one.
    unit: Option<String>,
    /// Where it will answer: `http://127.0.0.1:<port>/`.
    url: String,
}

/// Every slice's preview, in memory only.
#[derive(Default)]
pub(crate) struct Previews {
    runs: HashMap<AgentId, Run>,
    next_run: u64,
}

impl Previews {
    /// The states to put in a snapshot, ordered by agent.
    pub(crate) fn list(&self) -> Vec<Preview> {
        let mut out: Vec<Preview> = self
            .runs
            .iter()
            .map(|(agent, r)| Preview {
                agent: agent.clone(),
                state: r.state.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.agent.cmp(&b.agent));
        out
    }

    fn state(&self, agent: &AgentId) -> Option<&PreviewState> {
        self.runs.get(agent).map(|r| &r.state)
    }
}

impl Core {
    fn announce_preview(&mut self, agent: &AgentId) {
        let state = self
            .previews
            .state(agent)
            .cloned()
            .unwrap_or(PreviewState::NotStarted);
        self.emit(Event::Design(DesignEvent::PreviewChanged(Preview {
            agent: agent.clone(),
            state,
        })));
    }

    fn set_preview_state(&mut self, agent: &AgentId, state: PreviewState) {
        self.previews.runs.insert(
            agent.clone(),
            Run {
                state,
                run: self.previews.next_run,
                stop: None,
                pid: None,
                unit: None,
                url: String::new(),
            },
        );
        self.announce_preview(agent);
    }

    /// `DesignCommand::StartPreview`: starts the slice's preview unless it starts or runs already.
    pub(crate) fn start_preview(&mut self, agent: AgentId) {
        if matches!(
            self.previews.state(&agent),
            Some(PreviewState::Starting | PreviewState::Running { .. })
        ) {
            return;
        }
        self.previews.next_run += 1;
        let run = self.previews.next_run;
        match self.spawn_preview(&agent, run) {
            Ok(Some(started)) => {
                self.previews.runs.insert(agent.clone(), started);
                self.announce_preview(&agent);
            }
            Ok(None) => self.set_preview_state(&agent, PreviewState::NoCommand),
            Err(why) => self.set_preview_state(&agent, PreviewState::Failed { output: why.0 }),
        }
    }

    /// `DesignCommand::RestartPreview`.
    pub(crate) fn restart_preview(&mut self, agent: AgentId) {
        self.stop_preview(&agent, false);
        self.start_preview(agent);
    }

    /// Stops a slice's preview and forgets it. With `announce`, tells the UI it is not started.
    pub(crate) fn stop_preview(&mut self, agent: &AgentId, announce: bool) {
        let Some(run) = self.previews.runs.remove(agent) else {
            return;
        };
        kill(&run);
        if announce {
            self.announce_preview(agent);
        }
    }

    /// Stops every preview, for the app to quit. Returns once the processes have been signalled.
    pub(crate) fn stop_all_previews(&mut self) {
        for (_, run) in self.previews.runs.drain() {
            kill(&run);
        }
    }

    /// `Ok(None)`: the recipe has no preview command for the slice's repo.
    fn spawn_preview(&mut self, agent: &AgentId, run: u64) -> Result<Option<Run>, PreviewError> {
        let Some(row) = self.agents.get(agent) else {
            return Err(PreviewError(format!("no agent {}", agent.0)));
        };
        if row.agent.kind != AgentKind::Slice {
            return Err(PreviewError("only a slice has a preview".into()));
        }
        let Some(worktree) = row.agent.worktree.as_ref().map(|w| w.path.clone()) else {
            return Err(PreviewError("this slice has no worktree yet".into()));
        };
        let recipe = Recipe::load(&self.paths.recipe(self.project.key()))
            .map_err(|e| PreviewError(format!("reading the recipe: {e}")))?;
        let recipe = match self.repo_of(agent) {
            Some(repo) => recipe.for_repo(&repo.name),
            None => recipe,
        };
        let Some(template) = recipe.preview.clone().filter(|p| !p.trim().is_empty()) else {
            return Ok(None);
        };
        let port = free_port().map_err(|e| PreviewError(format!("finding a free port: {e}")))?;
        let command = template.replace("{port}", &port.to_string());
        let unit = format!(
            "{}-preview",
            crate::memory::unit_name(self.project.key(), &agent.0)
        );
        let scope = self
            .scopes
            .scope(&unit, crate::memory::cap_bytes(self.cap_gb(agent)));
        let mut cmd = match &scope {
            Some(scope) => {
                if let Some(cgroup) = self.scopes.cgroup(&unit) {
                    crate::memory::clear_stale_scope(&cgroup, &unit);
                }
                let mut c = tokio::process::Command::new("systemd-run");
                c.args(scope.args())
                    .arg("sh")
                    .arg("-c")
                    .arg(scope.escape(&command));
                c
            }
            None => {
                let mut c = tokio::process::Command::new("sh");
                c.arg("-c").arg(&command);
                c
            }
        };
        cmd.current_dir(&worktree)
            .envs(recipe.env_for(&worktree))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| PreviewError(format!("starting `{command}`: {e}")))?;
        let pid = child.id();
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(out) = child.stdout.take() {
            tokio::spawn(read_lines(out, tail.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(read_lines(err, tail.clone()));
        }
        let (stop, stopped) = oneshot::channel();
        let (report, mut reports) = mpsc::unbounded_channel();
        tokio::spawn(watch(child, port, stopped, tail, report));
        let inputs = self.inputs.clone();
        let who = agent.clone();
        tokio::spawn(async move {
            while let Some(report) = reports.recv().await {
                let input = Input::Preview {
                    agent: who.clone(),
                    run,
                    report,
                };
                if inputs.send(input).is_err() {
                    break;
                }
            }
        });
        tracing::info!(agent = %agent.0, port, capped = scope.is_some(), "starting preview");
        Ok(Some(Run {
            state: PreviewState::Starting,
            run,
            stop: Some(stop),
            pid,
            unit: scope.map(|_| unit),
            url: format!("http://127.0.0.1:{port}/"),
        }))
    }

    /// A watcher reported: the port answers, or the process ended.
    pub(crate) fn on_preview(&mut self, agent: &AgentId, run: u64, report: PreviewReport) {
        let current = self.previews.runs.get(agent).is_some_and(|r| r.run == run);
        if !current {
            return;
        }
        match report {
            PreviewReport::Ready => {
                if let Some(r) = self.previews.runs.get_mut(agent) {
                    r.state = PreviewState::Running { url: r.url.clone() };
                }
            }
            PreviewReport::Ended { why } => {
                if let Some(r) = self.previews.runs.get_mut(agent) {
                    r.state = PreviewState::Failed { output: why };
                    r.stop = None;
                    kill_group(r.pid.take());
                }
            }
        }
        self.announce_preview(agent);
    }
}

/// A port nothing listens on now. The window before the command binds it is accepted.
fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

/// Signals the process group the preview leads, and stops its scope.
fn kill(run: &Run) {
    kill_group(run.pid);
    if let Some(unit) = run.unit.clone() {
        // Best effort: the group kill already ended what ran; this clears a scope kept alive by
        // a process that left the group.
        std::thread::spawn(move || {
            for args in crate::memory::clear_scope_commands(&unit) {
                let _ = std::process::Command::new("systemctl")
                    .args(&args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        });
    }
}

fn kill_group(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    // `kill -KILL -- -PID` signals the whole group; the preview leads its own (unsafe is
    // forbidden here, so the `kill` program does it).
    let _ = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

async fn read_lines(from: impl AsyncRead + Unpin, tail: Arc<Mutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(from).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // A poisoned lock means another reader panicked; the tail is only for the user to read.
        if let Ok(mut tail) = tail.lock() {
            if tail.len() == OUTPUT_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    }
}

fn with_tail(why: String, tail: &Mutex<VecDeque<String>>) -> String {
    let lines = tail
        .lock()
        .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    if lines.is_empty() {
        why
    } else {
        format!("{why}\n{lines}")
    }
}

/// Owns the process: waits for the port, then for the end, reporting both. A stop request kills
/// and reaps it without a report.
async fn watch(
    mut child: tokio::process::Child,
    port: u16,
    mut stopped: oneshot::Receiver<()>,
    tail: Arc<Mutex<VecDeque<String>>>,
    report: mpsc::UnboundedSender<PreviewReport>,
) {
    let listening = wait_for_port(port);
    tokio::pin!(listening);
    let mut ready = false;
    loop {
        tokio::select! {
            up = &mut listening, if !ready => {
                if up {
                    ready = true;
                    let _ = report.send(PreviewReport::Ready);
                } else {
                    let why = format!(
                        "nothing listened on port {port} within {} s",
                        READY_WITHIN.as_secs()
                    );
                    let _ = report.send(PreviewReport::Ended { why: with_tail(why, &tail) });
                    let _ = child.kill().await;
                    return;
                }
            }
            status = child.wait() => {
                // The readers may still hold the last lines of output.
                tokio::time::sleep(Duration::from_millis(50)).await;
                let why = match status {
                    Ok(s) if ready => format!("the preview exited ({s})"),
                    Ok(s) => format!("the preview exited before it listened ({s})"),
                    Err(e) => format!("waiting for the preview: {e}"),
                };
                let _ = report.send(PreviewReport::Ended { why: with_tail(why, &tail) });
                return;
            }
            _ = &mut stopped => {
                let _ = child.kill().await;
                return;
            }
        }
    }
}

/// `true` once something accepts a connection on `port`; `false` after [`READY_WITHIN`].
async fn wait_for_port(port: u16) -> bool {
    let deadline = tokio::time::Instant::now() + READY_WITHIN;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(READY_EVERY).await;
    }
    false
}

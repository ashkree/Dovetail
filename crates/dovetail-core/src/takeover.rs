//! Take-over (M14, digest §2): `<leader>st` hands a session to native `claude` in the user's
//! terminal and takes it back.
//!
//! **The hand-off.** The UI lets go of the terminal and sends [`TerminalCommand::TakeOver`]. The core
//! stores a [`TakeoverLock`] on the agent (awaited, so it is on disk before anything stops),
//! stops the headless process, and once it has exited runs `claude --resume <id>` built from
//! the same [`dovetail_driver::SessionSpec`] as headless: same settings and hooks, MCP config,
//! role prompt, tools, model, permission mode, profile and recipe environment, in the same
//! working directory, under the same capped scope unit. When native exits, the lines it added to
//! the transcript are shown in the conversation under a note, the lock is cleared, the terminal
//! goes back ([`TerminalEvent::Returned`]) and headless resumes from the same session id.
//! Nothing on this path removes a worktree or archives anything.
//!
//! **The lock.** While a session is locked, headless never starts: [`Core::launch`], which
//! every start goes through (prompt, resume, OOM restart, restore), refuses. Dovetail's own
//! notes to a locked session wait and go once headless is back; user messages are refused with
//! a note saying where to write instead. Archiving a locked slice or removing its worktree is
//! refused.
//!
//! **After a crash** the lock is still stored. At open, and again before any start, it is
//! checked against `/proc` ([`verdict`]): native is gone → the lock is released (and its turns
//! shown); native still runs → it stays, with an error in Needs you saying so; `/proc` cannot
//! tell → Needs you asks whether to release it ([`RELEASE_TAKEOVER_PERMISSION`]).
//!
//! **Quitting during a take-over** cannot come from the UI, which reads no keys while native has
//! the terminal; a signal can end Dovetail then. Native is left running in the terminal, the
//! lock stays stored, and the next open checks it as after a crash.
//!
//! **Tools from native.** Native gets the same MCP config, so its tool calls reach this core
//! under the session's agent id. None waits on the UI: `core_ask_user` raises its question and
//! returns at once (ADR 0007), and the answer goes to the session as a message once headless
//! is back (answering while the lock holds is refused and the question stays open);
//! `project_start_slice` starts or queues the slice, or raises its approval and returns `pending`; the
//! coordinator's signals (slice ready, approvals) are held and sent when headless resumes.
//!
//! **Memory caps.** Native runs in the agent's own scope unit, so its cap holds there too. The
//! OOM watcher only follows headless processes, and an automatic restart goes through
//! [`Core::launch`], so it cannot start headless beside native.
//!
//! [`TerminalCommand::TakeOver`]: dovetail_api::TerminalCommand::TakeOver
//! [`TerminalEvent::Returned`]: dovetail_api::TerminalEvent::Returned
//! [`RELEASE_TAKEOVER_PERMISSION`]: dovetail_api::inbox::RELEASE_TAKEOVER_PERMISSION

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dovetail_api::{
    Event,
    agent::{AgentEvent, AgentId, AgentKind, SessionLock},
    conversation::EntryKind,
    inbox::{InboxKind, RELEASE_TAKEOVER_PERMISSION},
    terminal::TerminalEvent,
};
use dovetail_driver::transcript::{self, NativeEntry};
use serde::{Deserialize, Serialize};

use crate::db::now_ms;
use crate::error::SessionError;
use crate::inbox::Waiter;
use crate::service::{Core, Input};
use crate::session::state::ToolTally;

/// The program run for native take-over when [`crate::CoreOptions::claude`] is not set.
pub const NATIVE_CLAUDE: &str = "claude";

/// How long the headless process gets to exit before it is killed, when it is taken over.
pub const STOP_GRACE: Duration = Duration::from_secs(3);

/// At most this many native entries are copied into the conversation; the note says how many
/// more the transcript holds.
pub const MAX_NATIVE_ENTRIES: usize = 200;

/// A session's take-over lock, stored on its agent row while native `claude` has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeoverLock {
    /// When it was taken over, in Unix milliseconds.
    pub since_ms: i64,
    /// The native process, once started (under caps, `systemd-run`, which becomes `claude`).
    #[serde(default)]
    pub pid: Option<u32>,
    /// Its start time in clock ticks since boot (`/proc/<pid>/stat` field 22), so a reused pid
    /// is not taken for it.
    #[serde(default)]
    pub start_ticks: Option<u64>,
    /// The transcript's length when native started: what it adds comes after.
    #[serde(default)]
    pub transcript_offset: Option<u64>,
}

/// Whether a stored lock's native `claude` still runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It runs: the lock holds.
    Held {
        /// The native process.
        pid: u32,
    },
    /// It is gone: the lock can be released.
    Stale,
    /// It cannot be told; the text says why.
    Unknown(String),
}

/// What [`verdict`] needs to know about processes. [`Proc`] reads `/proc`; tests fake it.
pub trait Processes {
    /// The start time of a running (not zombie) process `pid`, or `None` if there is none.
    fn start_ticks(&self, pid: u32) -> io::Result<Option<u64>>;
    /// A running native `claude` on `session_id` (`--resume <id>` and no `-p`), if any.
    fn find_native(&self, session_id: &str) -> io::Result<Option<u32>>;
}

/// Whether the native run behind `lock` on session `session_id` still runs.
pub fn verdict(lock: &TakeoverLock, session_id: &str, procs: &dyn Processes) -> Verdict {
    match (lock.pid, lock.start_ticks) {
        (Some(pid), Some(ticks)) => match procs.start_ticks(pid) {
            Ok(Some(now)) if now == ticks => Verdict::Held { pid },
            Ok(_) => Verdict::Stale,
            Err(e) => Verdict::Unknown(format!("could not read process {pid}: {e}")),
        },
        // No pid on record: Dovetail stopped between taking the lock and starting native, or
        // could not read the start time. Look for the process by its command line.
        _ => match procs.find_native(session_id) {
            Ok(Some(pid)) => Verdict::Held { pid },
            Ok(None) => Verdict::Stale,
            Err(e) => Verdict::Unknown(format!("could not list processes: {e}")),
        },
    }
}

/// Where a take-over this app runs stands. Headless may start only in [`Phase::Headless`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Dovetail drives the session.
    Headless,
    /// Locked; the headless process is stopping.
    Stopping,
    /// Locked; native `claude` is being started.
    Starting,
    /// Locked; native `claude` runs in the terminal.
    Native,
}

/// What happens to a take-over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `<leader>st`; `live`: a headless process runs and must stop first.
    Begin {
        /// A headless process runs.
        live: bool,
    },
    /// The headless process exited.
    HeadlessExited,
    /// The native process started.
    NativeStarted,
    /// The native process exited, or could not start.
    NativeExited,
}

impl Phase {
    /// The phase after `step`, or `None` if `step` cannot happen now.
    pub fn after(self, step: Step) -> Option<Phase> {
        Some(match (self, step) {
            (Phase::Headless, Step::Begin { live: true }) => Phase::Stopping,
            (Phase::Headless, Step::Begin { live: false }) => Phase::Starting,
            (Phase::Stopping, Step::HeadlessExited) => Phase::Starting,
            (Phase::Starting, Step::NativeStarted) => Phase::Native,
            (Phase::Starting | Phase::Native, Step::NativeExited) => Phase::Headless,
            _ => return None,
        })
    }
}

/// Reads `/proc`.
pub struct Proc;

const PROC_ROOT: &str = "/proc";

/// The state and start time (`/proc/<pid>/stat` fields 3 and 22). Fields are counted from the
/// last `)`, as the command name may hold spaces and parentheses.
pub fn parse_start(stat: &str) -> Option<(char, u64)> {
    let (_, rest) = stat.rsplit_once(')')?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let ticks = fields.get(19)?.parse().ok()?;
    Some((state, ticks))
}

/// Whether a NUL-separated command line is native `claude` on `session_id`.
pub fn is_native_on(cmdline: &[u8], session_id: &str) -> bool {
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    let resumes = args
        .windows(2)
        .any(|w| w[0] == b"--resume" && w[1] == session_id.as_bytes());
    let print = args.iter().any(|a| *a == b"-p" || *a == b"--print");
    resumes && !print
}

impl Processes for Proc {
    fn start_ticks(&self, pid: u32) -> io::Result<Option<u64>> {
        let path = Path::new(PROC_ROOT).join(pid.to_string()).join("stat");
        match std::fs::read_to_string(path) {
            Ok(stat) => match parse_start(&stat) {
                Some(('Z' | 'X', _)) => Ok(None),
                Some((_, ticks)) => Ok(Some(ticks)),
                None => Err(io::Error::other("unreadable stat line")),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn find_native(&self, session_id: &str) -> io::Result<Option<u32>> {
        for entry in std::fs::read_dir(PROC_ROOT)? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            // A process may be gone, or not ours to read: neither is the native run.
            let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            if is_native_on(&cmdline, session_id) && self.start_ticks(pid)?.is_some() {
                return Ok(Some(pid));
            }
        }
        Ok(None)
    }
}

/// How the native run ended, from the blocking thread that ran it.
pub(crate) struct NativeEnd {
    /// Why it failed to start or exited unsuccessfully; `None` for a clean exit.
    pub(crate) error: Option<String>,
    /// What it added to the transcript.
    pub(crate) entries: Vec<NativeEntry>,
}

/// What the blocking thread needs to run native `claude`.
struct NativeRun {
    agent: AgentId,
    program: OsString,
    args: Vec<OsString>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    /// The scope to clear first, if a dead process left it (its unit name and cgroup).
    stale_scope: Option<(String, PathBuf)>,
    transcript: Option<PathBuf>,
    offset: u64,
}

/// Runs native `claude` in the terminal and waits for it; always reports back, and always
/// waits for the child.
fn run_native(run: NativeRun, inputs: &tokio::sync::mpsc::UnboundedSender<Input>) {
    if let Some((unit, cgroup)) = &run.stale_scope {
        // The headless process just exited; `--collect` normally removed its scope already.
        crate::memory::clear_stale_scope(cgroup, unit);
    }
    let shown = run.program.to_string_lossy().into_owned();
    let mut command = std::process::Command::new(&run.program);
    command
        .args(&run.args)
        .current_dir(&run.cwd)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    for (k, v) in &run.env {
        command.env(k, v);
    }
    let error = match command.spawn() {
        Err(e) => Some(format!("could not start {shown}: {e}")),
        Ok(child) => {
            let _ = inputs.send(Input::NativeStarted {
                agent: run.agent.clone(),
                pid: child.id(),
            });
            // A Ctrl-Z in native claude must not leave the terminal hanging (`foreground`).
            match crate::foreground::wait(child) {
                Ok(status) if status.success() => None,
                Ok(status) => Some(match status.code() {
                    Some(127) => format!("{shown} exited with 127 (not found)"),
                    Some(code) => format!("{shown} exited with {code}"),
                    None => format!("{shown} ended: {status}"),
                }),
                Err(e) => Some(format!("waiting for {shown}: {e}")),
            }
        }
    };
    let entries = match &run.transcript {
        Some(path) => transcript::read_since(path, run.offset).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), "reading native turns: {e}");
            Vec::new()
        }),
        None => Vec::new(),
    };
    let _ = inputs.send(Input::NativeExited {
        agent: run.agent,
        end: NativeEnd { error, entries },
    });
}

/// The in-app take-over: which agent, and where it stands.
pub(crate) struct Active {
    pub(crate) agent: AgentId,
    pub(crate) phase: Phase,
}

impl Core {
    /// Whether agent `id` is taken over (by this app, or by a lock left from before).
    pub(crate) fn locked(&self, id: &AgentId) -> bool {
        self.agents.get(id).is_some_and(|r| r.takeover.is_some())
    }

    /// Before any headless start: `Ok` when `id` holds no lock, or held one whose native run
    /// is gone (then released, with its turns shown). Else the start is refused.
    pub(crate) fn check_lock(&mut self, id: &AgentId) -> Result<(), SessionError> {
        let Some(row) = self.agents.get(id) else {
            return Ok(());
        };
        let Some(lock) = row.takeover.clone() else {
            return Ok(());
        };
        let name = row.agent.name.clone();
        if self.takeover.as_ref().is_some_and(|t| &t.agent == id) {
            return Err(SessionError::TakenOver(name));
        }
        let session = row.session_id.clone().unwrap_or_default();
        match verdict(&lock, &session, self.processes()) {
            // `launch` goes on to start headless, and sends the held notes once it runs.
            Verdict::Stale => {
                self.release_stale(id, &lock, false);
                Ok(())
            }
            Verdict::Held { .. } | Verdict::Unknown(_) => Err(SessionError::TakenOver(name)),
        }
    }

    /// Why agent `id` cannot be taken over now, if it cannot.
    fn refuse_take_over(&self, id: &AgentId) -> Option<String> {
        let Some(row) = self.agents.get(id) else {
            return Some(format!("no agent {}", id.0));
        };
        let name = &row.agent.name;
        if !row.agent.kind.is_session() {
            return Some(format!(
                "{name} is a subagent: take over the session it runs in"
            ));
        }
        // It runs in its task's research folder and only reads: nothing to take over to.
        if row.agent.kind == AgentKind::Ask {
            return Some(format!("{name} is reading mode's Ask: it stays headless"));
        }
        if self.takeover.is_some() || self.editor_open || self.editor_pending.is_some() {
            return Some("the terminal is already lent out".into());
        }
        if row.takeover.is_some() {
            return Some(format!("{name} is already taken over in a native terminal"));
        }
        if self.live.get(id).is_some_and(|l| l.setting_up) {
            return Some(format!("{name} is still setting up"));
        }
        // Being archived: its exit removes the worktree, which native would run in.
        if self.live.get(id).is_some_and(|l| l.stopping) {
            return Some(format!("{name} is stopping"));
        }
        if row.transcript_path.is_none() || row.session_id.is_none() {
            return Some(format!(
                "{name} has no session to resume yet: send it a message first"
            ));
        }
        let home = match &self.claude {
            Ok(home) => home,
            Err(e) => return Some(e.to_string()),
        };
        let recorded = row
            .claude_home
            .clone()
            .unwrap_or_else(crate::profile::default_dir);
        if !crate::profile::same_dir(&recorded, &home.dir) {
            return Some(format!(
                "{name}'s session was recorded under another Claude profile: write to it once \
                 to start it under this one"
            ));
        }
        None
    }

    /// `<leader>st`: the UI has let go of the terminal. Takes the lock, stops headless, and
    /// starts native once it has exited. Every refusal hands the terminal back at once.
    pub(crate) async fn take_over(&mut self, id: AgentId) {
        if let Some(why) = self.refuse_take_over(&id) {
            self.emit(Event::Terminal(TerminalEvent::Returned {
                error: Some(why),
            }));
            return;
        }
        let live = self.live.contains_key(&id);
        let Some(phase) = Phase::Headless.after(Step::Begin { live }) else {
            return;
        };
        let lock = TakeoverLock {
            since_ms: now_ms(),
            pid: None,
            start_ticks: None,
            transcript_offset: None,
        };
        let Some(row) = self.agents.get_mut(&id) else {
            return;
        };
        row.takeover = Some(lock);
        row.agent.lock = SessionLock::TakenOver;
        let row = row.clone();
        // On disk before anything stops: a crash from here on leaves the lock for the next open.
        if let Err(e) = self.db.upsert_agent(row.clone()).await {
            self.update_agent(&id, |row| {
                row.takeover = None;
                row.agent.lock = SessionLock::Headless;
            });
            self.emit(Event::Terminal(TerminalEvent::Returned {
                error: Some(format!("could not store the take-over lock: {e}")),
            }));
            return;
        }
        // Stored above; the UI only needs to hear of it.
        self.emit(Event::Agent(AgentEvent::Upserted(row.agent)));
        self.takeover = Some(Active {
            agent: id.clone(),
            phase,
        });
        let name = self.name_of(&id);
        self.activity(
            dovetail_api::activity::ActivityKind::Lifecycle,
            Some(&id),
            format!("{name} taken over in a native terminal"),
        );
        match self.live.get_mut(&id) {
            Some(live) => {
                live.stopping = true;
                live.session.shutdown(STOP_GRACE);
            }
            None => self.start_native(&id),
        }
    }

    /// A headless process exited: if it was stopping for a take-over, native starts now.
    pub(crate) fn takeover_headless_exited(&mut self, id: &AgentId) {
        let Some(active) = self.takeover.as_mut().filter(|t| &t.agent == id) else {
            return;
        };
        let Some(next) = active.phase.after(Step::HeadlessExited) else {
            return;
        };
        active.phase = next;
        self.start_native(id);
    }

    /// Builds the native command line from the headless spec and runs it on a blocking thread.
    fn start_native(&mut self, id: &AgentId) {
        if let Some(active) = self.takeover.as_mut().filter(|t| &t.agent == id) {
            active.phase = Phase::Starting;
        }
        let prepared = self.native_run(id);
        let run = match prepared {
            Ok(run) => run,
            Err(e) => {
                let inputs = self.inputs.clone();
                let _ = inputs.send(Input::NativeExited {
                    agent: id.clone(),
                    end: NativeEnd {
                        error: Some(e.to_string()),
                        entries: Vec::new(),
                    },
                });
                return;
            }
        };
        let offset = run.offset;
        self.update_agent(id, |row| {
            if let Some(lock) = row.takeover.as_mut() {
                lock.transcript_offset = Some(offset);
            }
        });
        tracing::info!(agent = %id.0, program = ?run.program, args = ?run.args, "native take-over");
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || run_native(run, &inputs));
    }

    fn native_run(&mut self, id: &AgentId) -> Result<NativeRun, SessionError> {
        let row = self
            .agents
            .get(id)
            .cloned()
            .ok_or_else(|| SessionError::NoAgent(id.0.clone()))?;
        let cwd = self.session_cwd(&row.agent);
        let session = row
            .session_id
            .clone()
            .ok_or_else(|| SessionError::NeverStarted(row.agent.name.clone()))?;
        let spec = self.session_spec(id, cwd.clone(), None, session, true)?;
        let program = self
            .options
            .claude
            .clone()
            .unwrap_or_else(|| NATIVE_CLAUDE.into());
        let (program, args) = spec.wrap(&program, spec.native_args());
        let unit = crate::memory::unit_name(self.project.key(), &id.0);
        let stale_scope = self.scopes.cgroup(&unit).map(|cgroup| (unit, cgroup));
        let transcript = row.transcript_path.clone();
        let offset = transcript
            .as_deref()
            .map(|p| transcript::len(p).unwrap_or(0))
            .unwrap_or(0);
        Ok(NativeRun {
            agent: id.clone(),
            program,
            args,
            cwd,
            env: spec.env,
            stale_scope,
            transcript,
            offset,
        })
    }

    /// The native process is running: its pid and start time go on the lock.
    pub(crate) fn native_started(&mut self, id: &AgentId, pid: u32) {
        if let Some(active) = self.takeover.as_mut().filter(|t| &t.agent == id)
            && let Some(next) = active.phase.after(Step::NativeStarted)
        {
            active.phase = next;
        }
        let ticks = Proc.start_ticks(pid).ok().flatten();
        self.update_agent(id, |row| {
            if let Some(lock) = row.takeover.as_mut() {
                lock.pid = Some(pid);
                lock.start_ticks = ticks;
            }
        });
    }

    /// Native has exited (or never started): give the terminal back, then release the lock,
    /// showing what happened in native, and resume headless.
    pub(crate) fn native_exited(&mut self, id: &AgentId, end: NativeEnd) {
        self.emit(Event::Terminal(TerminalEvent::Returned {
            error: end.error.clone(),
        }));
        let name = self.name_of(id);
        self.activity(
            dovetail_api::activity::ActivityKind::Lifecycle,
            Some(id),
            format!("{name} back from the native terminal"),
        );
        let mut note = "Back from the native terminal".to_string();
        if let Some(error) = &end.error {
            note.push_str(&format!(" ({error})"));
        }
        // Headless again, from the same session id.
        self.release(id, note, &end.entries, true);
    }

    /// The one way a lock ends, whichever way it was found to be over (native exited here, a
    /// crash lock whose native is gone, the user's `y`): the lock is cleared and stored, the
    /// note and native turns go in the conversation, and with `resume` headless starts again.
    /// Notes held for the session ([`Core::hold_for_takeover`]) are sent by `launch` once
    /// headless runs, so no path loses them; without `resume` they wait for the next start
    /// (`launch` is already on its way when a start finds the lock stale).
    fn release(&mut self, id: &AgentId, note: String, entries: &[NativeEntry], resume: bool) {
        if self.takeover.as_ref().is_some_and(|t| &t.agent == id) {
            self.takeover = None;
        }
        self.update_agent(id, |row| {
            row.takeover = None;
            row.agent.lock = SessionLock::Headless;
        });
        self.show_native_turns(id, note, entries);
        if resume && let Err(e) = self.ensure_live(id) {
            self.fail(e);
        }
        self.refresh_status(id);
    }

    /// Sends the notes held while `id` was taken over; `launch` calls it once headless runs.
    pub(crate) fn send_held(&mut self, id: &AgentId) {
        for text in self.held_for_takeover.remove(id).unwrap_or_default() {
            self.send_note(id, &text);
        }
    }

    /// Whether anything waits for `id`'s headless session: held notes, or for the coordinator
    /// its stored notes.
    fn notes_waiting(&self, id: &AgentId) -> bool {
        self.held_for_takeover
            .get(id)
            .is_some_and(|n| !n.is_empty())
            || (id.0 == crate::service::COORDINATOR && !self.coordinator_notes.is_empty())
    }

    /// How processes are checked: `/proc`, or what a test gave.
    fn processes(&self) -> &dyn Processes {
        match &self.options.processes {
            Some(p) => p.as_ref(),
            None => &Proc,
        }
    }

    /// The note that native turns happened, and the turns themselves when the transcript had
    /// them: the user's messages and assistant text as entries, each turn's tool calls folded
    /// into one summary line, as headless turns show.
    fn show_native_turns(&mut self, id: &AgentId, note: String, entries: &[NativeEntry]) {
        let turns = entries
            .iter()
            .filter(|e| matches!(e, NativeEntry::User(_)))
            .count();
        let skipped = entries.len().saturating_sub(MAX_NATIVE_ENTRIES);
        let text = match (entries.is_empty(), turns) {
            (true, _) => format!("{note}: nothing new in the transcript."),
            (false, 1) => format!("{note}: 1 native turn, shown below."),
            (false, n) => format!("{note}: {n} native turns, shown below."),
        };
        let text = if skipped > 0 {
            format!("{text} The first {skipped} entries are left out; the transcript has them.")
        } else {
            text
        };
        self.notice(id, text);
        let mut tools = ToolTally::default();
        for entry in &entries[skipped..] {
            match entry {
                NativeEntry::ToolUse(name) => {
                    tools.add(name);
                    let line = tools.line();
                    match tools.entry {
                        Some(e) => self.set_entry_text(id, e, line),
                        None => {
                            tools.entry = Some(self.add_entry(id, EntryKind::ToolSummary, line))
                        }
                    }
                }
                NativeEntry::User(text) => {
                    tools = ToolTally::default();
                    self.add_entry(id, EntryKind::User, text.clone());
                }
                // As in a headless turn, one summary line holds all of a turn's tools.
                NativeEntry::Assistant(text) => {
                    self.add_entry(id, EntryKind::Assistant, text.clone());
                }
            }
        }
    }

    /// A lock whose native run is gone (Dovetail stopped while it ran): release it and show
    /// what native added.
    fn release_stale(&mut self, id: &AgentId, lock: &TakeoverLock, resume: bool) {
        let entries = match (
            self.agents.get(id).and_then(|r| r.transcript_path.clone()),
            lock.transcript_offset,
        ) {
            (Some(path), Some(offset)) => transcript::read_since(&path, offset).unwrap_or_default(),
            _ => Vec::new(),
        };
        tracing::info!(agent = %id.0, "released a take-over lock left by a crash");
        self.release(
            id,
            "Dovetail stopped while this session was taken over; the native terminal has \
             exited since and the lock is released"
                .into(),
            &entries,
            resume,
        );
    }

    /// At open: checks each lock left from before. Runs before anything can start a session.
    pub(crate) fn reconcile_locks(&mut self) {
        let locked: Vec<(AgentId, TakeoverLock, String, String)> = self
            .agents
            .values()
            .filter_map(|r| {
                Some((
                    r.agent.id.clone(),
                    r.takeover.clone()?,
                    r.session_id.clone().unwrap_or_default(),
                    r.agent.name.clone(),
                ))
            })
            .collect();
        for (id, lock, session, name) in locked {
            match verdict(&lock, &session, self.processes()) {
                // Nothing is held this early; the coordinator resumes with the rest of the open.
                Verdict::Stale => self.release_stale(&id, &lock, false),
                Verdict::Held { pid } => {
                    self.update_agent(&id, |row| {
                        if let Some(l) = row.takeover.as_mut() {
                            l.pid = Some(pid);
                        }
                    });
                    let text = format!(
                        "{name} is still taken over: a native claude (pid {pid}) from before \
                         Dovetail restarted runs on its session. Exit it there; Dovetail \
                         resumes {name} headless when you next write to it."
                    );
                    self.raise(&id, InboxKind::Error { text }, Waiter::None);
                }
                Verdict::Unknown(why) => {
                    self.raise(
                        &id,
                        InboxKind::Permission {
                            tool: RELEASE_TAKEOVER_PERMISSION.into(),
                            input: serde_json::json!({ "session": name, "reason": why }),
                            reason: Some(format!(
                                "{name} was taken over in a native terminal when Dovetail \
                                 stopped, and whether that claude still runs cannot be told \
                                 ({why}). Release the lock only if it has exited: y releases, \
                                 d keeps it."
                            )),
                            suggestions: Vec::new(),
                        },
                        Waiter::ReleaseLock,
                    );
                }
            }
        }
    }

    /// `y` on a release item: the user says the native run is gone. Headless comes back at
    /// once when something waits for it (held notes), and for the coordinator, which runs
    /// whenever Dovetail is open; a slice otherwise resumes on first touch, as at open.
    pub(crate) fn release_lock(&mut self, id: &AgentId) {
        if !self.locked(id) || self.takeover.as_ref().is_some_and(|t| &t.agent == id) {
            return;
        }
        let resume = self.notes_waiting(id) || id.0 == crate::service::COORDINATOR;
        self.release(
            id,
            "Take-over lock released by the user".into(),
            &[],
            resume,
        );
    }

    /// A note from Dovetail for a locked session: kept, and sent once headless is back.
    pub(crate) fn hold_for_takeover(&mut self, id: &AgentId, text: &str) {
        self.held_for_takeover
            .entry(id.clone())
            .or_default()
            .push(text.to_owned());
    }

    /// The working directory of agent `agent`'s session: a slice's worktree (or, without git,
    /// its repo's folder), else the coordinator's folder (ADR 0021).
    pub(crate) fn session_cwd(&self, agent: &dovetail_api::agent::Agent) -> PathBuf {
        match (agent.kind, &agent.worktree) {
            (AgentKind::Slice, Some(w)) => w.path.clone(),
            (AgentKind::Slice, None) => self
                .repo_of(&agent.id)
                .map_or_else(|| self.project.coordinator_folder(&self.paths), |r| r.path),
            // Whatever project runs it, a tangent works in Tangents' folder (ADR 0017).
            (AgentKind::Tangent, _) => self.tangents_folder(),
            _ => self.project.coordinator_folder(&self.paths),
        }
    }

    /// The repo agent `id` works in (ADR 0021): a slice's stored repo, or the first when it has
    /// none stored (slices from before the migration); a subagent's parent's; for any other
    /// agent, the first repo. `None` in a project with no repos, or when the repo was removed.
    pub(crate) fn repo_of(
        &self,
        id: &dovetail_api::agent::AgentId,
    ) -> Option<crate::projects::Repo> {
        let row = self.agents.get(id)?;
        match (row.agent.kind, &row.agent.parent) {
            (AgentKind::Subagent { .. }, Some(parent)) if parent != id => self.repo_of(parent),
            (AgentKind::Slice, _) => self.project.repo(row.repo),
            _ => self.project.repo(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Processes as a test says they are.
    struct Fake {
        ticks: io::Result<Option<u64>>,
        native: io::Result<Option<u32>>,
        asked: RefCell<Vec<String>>,
    }

    fn clone_result<T: Copy>(r: &io::Result<T>) -> io::Result<T> {
        match r {
            Ok(v) => Ok(*v),
            Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
        }
    }

    impl Processes for Fake {
        fn start_ticks(&self, pid: u32) -> io::Result<Option<u64>> {
            self.asked.borrow_mut().push(format!("ticks {pid}"));
            clone_result(&self.ticks)
        }
        fn find_native(&self, session_id: &str) -> io::Result<Option<u32>> {
            self.asked.borrow_mut().push(format!("find {session_id}"));
            clone_result(&self.native)
        }
    }

    fn fake(ticks: io::Result<Option<u64>>, native: io::Result<Option<u32>>) -> Fake {
        Fake {
            ticks,
            native,
            asked: RefCell::new(Vec::new()),
        }
    }

    fn lock(pid: Option<u32>, ticks: Option<u64>) -> TakeoverLock {
        TakeoverLock {
            since_ms: 1,
            pid,
            start_ticks: ticks,
            transcript_offset: Some(0),
        }
    }

    #[test]
    fn the_phases_go_round_and_refuse_steps_out_of_turn() {
        use Phase::*;
        // Live headless: stop it, start native, run, back.
        let mut p = Headless;
        for (step, want) in [
            (Step::Begin { live: true }, Stopping),
            (Step::HeadlessExited, Starting),
            (Step::NativeStarted, Native),
            (Step::NativeExited, Headless),
        ] {
            p = p.after(step).expect("allowed");
            assert_eq!(p, want);
        }
        // Not running: straight to starting native; a failed start goes back at once.
        assert_eq!(Headless.after(Step::Begin { live: false }), Some(Starting));
        assert_eq!(Starting.after(Step::NativeExited), Some(Headless));
        // Out of turn: a second take-over, native before headless stopped, exits from nowhere.
        assert_eq!(Native.after(Step::Begin { live: false }), None);
        assert_eq!(Stopping.after(Step::Begin { live: true }), None);
        assert_eq!(Stopping.after(Step::NativeStarted), None);
        assert_eq!(Stopping.after(Step::NativeExited), None);
        assert_eq!(Headless.after(Step::NativeExited), None);
        assert_eq!(Headless.after(Step::HeadlessExited), None);
    }

    #[test]
    fn a_recorded_pid_holds_only_while_the_same_process_runs() {
        let l = lock(Some(42), Some(1000));
        assert_eq!(
            verdict(&l, "s", &fake(Ok(Some(1000)), Ok(None))),
            Verdict::Held { pid: 42 }
        );
        // Gone, or the pid reused by another process.
        assert_eq!(verdict(&l, "s", &fake(Ok(None), Ok(None))), Verdict::Stale);
        assert_eq!(
            verdict(&l, "s", &fake(Ok(Some(2000)), Ok(None))),
            Verdict::Stale
        );
        let denied = fake(Err(io::Error::other("denied")), Ok(None));
        assert!(matches!(verdict(&l, "s", &denied), Verdict::Unknown(_)));
    }

    #[test]
    fn with_no_pid_on_record_the_process_is_looked_for_by_its_command_line() {
        let l = lock(None, None);
        let found = fake(Ok(None), Ok(Some(7)));
        assert_eq!(verdict(&l, "abc", &found), Verdict::Held { pid: 7 });
        assert_eq!(found.asked.borrow().as_slice(), ["find abc"]);
        assert_eq!(
            verdict(&l, "abc", &fake(Ok(None), Ok(None))),
            Verdict::Stale
        );
        let blind = fake(Ok(None), Err(io::Error::other("no /proc")));
        assert!(matches!(verdict(&l, "abc", &blind), Verdict::Unknown(_)));
    }

    #[test]
    fn stat_lines_give_state_and_start_time() {
        let stat = "123 (claude (x) y) S 1 123 123 0 -1 4194560 100 0 0 0 5 1 0 0 20 0 9 0 \
                    987654 1000 50";
        assert_eq!(parse_start(stat), Some(('S', 987654)));
        assert_eq!(parse_start("garbage"), None);
    }

    #[test]
    fn native_claude_is_told_from_headless_by_its_flags() {
        let native = b"claude\0--permission-mode\0auto\0--resume\0abc\0--name\0x\0";
        assert!(is_native_on(native, "abc"));
        assert!(!is_native_on(native, "abd"));
        let headless = b"claude\0-p\0--input-format\0stream-json\0--resume\0abc\0";
        assert!(!is_native_on(headless, "abc"));
    }

    #[test]
    fn this_process_is_seen_running_and_a_dead_pid_is_not() {
        let me = std::process::id();
        let ticks = Proc.start_ticks(me).expect("read").expect("running");
        let l = lock(Some(me), Some(ticks));
        assert_eq!(verdict(&l, "s", &Proc), Verdict::Held { pid: me });
        let mut child = std::process::Command::new("true").spawn().expect("true");
        let dead = child.id();
        child.wait().expect("reaped");
        assert_eq!(
            verdict(&lock(Some(dead), Some(ticks)), "s", &Proc),
            Verdict::Stale
        );
    }
}

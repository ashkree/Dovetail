//! Upgrading Dovetail from inside the app (ADR 0014): build the next binary in the background
//! into a staging folder, then swap it over the installed one while nothing runs it.
//!
//! This module is the blocking backbone, shared by the app (`:upgrade`, see [`flow`]) and the
//! `dovetail upgrade` command:
//!
//! - [`build`] fetches `origin/main` into a throwaway worktree (or builds the checkout itself
//!   with [`Source::Local`]), runs `cargo build --release`, and leaves the binary in
//!   `<data>/upgrade/staged/` with a [`Ready`] note. It never touches `~/.local/bin`.
//! - [`install_staged`] renames the staged binary over the installed one, and only when
//!   [`running_blockers`] finds no app and no helper of it (plan R5; the same rule as
//!   `scripts/install.sh`).
//!
//! Every process goes through the [`Exec`] trait, so tests run the steps with a fake.

pub(crate) mod flow;

use std::fs;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// The branch `:upgrade` builds, on `origin`.
pub const SOURCE_BRANCH: &str = "main";
/// How many lines of a failed build's output the Error item shows.
pub const BUILD_TAIL_LINES: usize = 20;
/// How long a quit waits for the app's own helpers to exit before it calls the install blocked.
pub const HELPER_EXIT_WAIT: Duration = Duration::from_secs(2);
/// How often a running step checks whether it was cancelled.
const POLL: Duration = Duration::from_millis(100);

/// The installed binary: `~/.local/bin/dovetail`, where `scripts/install.sh` puts it.
pub fn install_path() -> Result<PathBuf, UpgradeError> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or(UpgradeError::NoHome)?;
    Ok(PathBuf::from(home).join(".local/bin/dovetail"))
}

/// Whether `exe` is the installed binary. A dev build in a slice is not, and must not replace
/// the one the user works in.
pub fn is_installed_binary(exe: &Path, install: &Path) -> bool {
    match (exe.canonicalize(), install.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// What to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `origin/main`, fetched into a throwaway worktree.
    OriginMain,
    /// The checkout itself (`--local`), half-finished edits and all.
    Local,
}

/// A build that finished and waits to be installed, written next to the staged binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    /// Where it came from: a short commit id, or `<id> (local checkout)`.
    pub source: String,
    /// Role prompts and the recipe whose installed copy differs from the new build's, as
    /// `roles/slice.md`. Upgrade never overwrites them (`scripts/bootstrap.sh --force` does).
    pub differing: Vec<String>,
    /// When the build finished, in Unix milliseconds.
    pub built_at_ms: i64,
}

/// The staging folder, `<data>/upgrade/`.
#[derive(Debug, Clone)]
pub struct Staging {
    root: PathBuf,
}

impl Staging {
    /// The staging folder of these paths.
    pub fn new(paths: &Paths) -> Self {
        Self {
            root: paths.upgrade(),
        }
    }

    /// The throwaway worktree of `origin/main`; empty between builds.
    fn src(&self) -> PathBuf {
        self.root.join("src")
    }

    /// Cargo's target directory, kept between builds so they are incremental.
    fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }

    /// The last build's output.
    pub fn log(&self) -> PathBuf {
        self.root.join("build.log")
    }

    fn lock(&self) -> PathBuf {
        self.root.join("build.lock")
    }

    fn staged_dir(&self) -> PathBuf {
        self.root.join("staged")
    }

    /// The staged binary.
    pub fn binary(&self) -> PathBuf {
        self.staged_dir().join("dovetail")
    }

    fn ready_file(&self) -> PathBuf {
        self.staged_dir().join("ready.json")
    }

    /// The staged build, if one is complete: the binary and its note both exist and parse.
    pub fn ready(&self) -> Option<Ready> {
        if !self.binary().is_file() {
            return None;
        }
        let text = fs::read_to_string(self.ready_file()).ok()?;
        serde_json::from_str(&text)
            .inspect_err(|e| {
                tracing::warn!("upgrade: unreadable {}: {e}", self.ready_file().display())
            })
            .ok()
    }

    /// Forgets the staged build. The note goes first, so a half-removed build never reads as
    /// ready.
    pub fn clear(&self) {
        for path in [self.ready_file(), self.binary()] {
            if let Err(e) = fs::remove_file(&path)
                && e.kind() != io::ErrorKind::NotFound
            {
                tracing::warn!("upgrade: removing {}: {e}", path.display());
            }
        }
    }
}

/// What can go wrong outside a build step.
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    /// `$HOME` is not set, so the install path is unknown.
    #[error("HOME is not set, so the install path (~/.local/bin/dovetail) is unknown")]
    NoHome,
    /// The folder is not a Dovetail checkout.
    #[error(
        "{0} is not a Dovetail checkout (no crates/dovetail); run upgrade from a project that is"
    )]
    NotDovetail(PathBuf),
    /// The running binary is not the installed one.
    #[error("this is not the installed Dovetail ({0}); upgrade only replaces the installed binary")]
    NotInstalled(PathBuf),
    /// Another build is running.
    #[error("another upgrade is already building (pid {0})")]
    Busy(u32),
    /// A file operation failed.
    #[error("{what}: {source}")]
    Io {
        /// What was being done.
        what: String,
        /// The cause.
        source: io::Error,
    },
}

fn io_err(what: impl Into<String>) -> impl FnOnce(io::Error) -> UpgradeError {
    let what = what.into();
    move |source| UpgradeError::Io { what, source }
}

/// A process to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The program, looked up on `PATH`.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Where it runs.
    pub cwd: PathBuf,
    /// Extra environment.
    pub env: Vec<(String, String)>,
    /// Append its stdout and stderr here; `None` captures stdout and returns it.
    pub log: Option<PathBuf>,
}

/// Why a process did not succeed.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The program could not be started.
    #[error("could not start {program}: {source}")]
    Spawn {
        /// The program.
        program: String,
        /// The cause.
        source: io::Error,
    },
    /// It exited with a failure.
    #[error("{program} exited with {}", .code.map_or("a signal".to_owned(), |c| format!("status {c}")))]
    Failed {
        /// The program.
        program: String,
        /// Its exit code, if it had one.
        code: Option<i32>,
    },
    /// The app is quitting; the process was killed.
    #[error("cancelled")]
    Cancelled,
    /// Reading its output or its log failed.
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Runs processes. The real one is [`SystemExec`]; tests use a fake.
pub trait Exec: Send + Sync {
    /// Runs `call` to the end, or kills it when `cancel` is set. Returns its stdout when
    /// `call.log` is `None`, else an empty string.
    fn run(&self, call: &Call, cancel: &AtomicBool) -> Result<String, ExecError>;
}

/// Runs real processes, each in its own process group so a cancel kills cargo's compilers too,
/// and always waits for the child.
#[derive(Debug, Default)]
pub struct SystemExec;

impl Exec for SystemExec {
    fn run(&self, call: &Call, cancel: &AtomicBool) -> Result<String, ExecError> {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(&call.program);
        command
            .args(&call.args)
            .current_dir(&call.cwd)
            .envs(call.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .process_group(0);
        match &call.log {
            Some(log) => {
                let file = fs::OpenOptions::new().create(true).append(true).open(log)?;
                command.stdout(file.try_clone()?).stderr(file);
            }
            None => {
                command.stdout(Stdio::piped()).stderr(Stdio::null());
            }
        }
        let mut child = command.spawn().map_err(|source| ExecError::Spawn {
            program: call.program.clone(),
            source,
        })?;
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                kill_group(&mut child);
                // Reaped, whatever the signal did.
                let _ = child.wait();
                return Err(ExecError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(POLL),
                Err(e) => {
                    kill_group(&mut child);
                    let _ = child.wait();
                    return Err(e.into());
                }
            }
        };
        if !status.success() {
            return Err(ExecError::Failed {
                program: call.program.clone(),
                code: status.code(),
            });
        }
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            io::Read::read_to_string(&mut stdout, &mut out)?;
        }
        Ok(out)
    }
}

fn kill_group(child: &mut std::process::Child) {
    use rustix::process::{Pid, Signal, kill_process_group};
    if kill_process_group(Pid::from_child(child), Signal::KILL).is_err() {
        let _ = child.kill();
    }
}

/// A build that did not produce a staged binary.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// Something outside the steps: the checkout, the lock, a file.
    #[error(transparent)]
    Upgrade(#[from] UpgradeError),
    /// A step failed; `tail` is the end of its output.
    #[error("{step} failed ({cause}){}", if .tail.is_empty() { String::new() } else { format!(":\n{tail}") })]
    Step {
        /// What was being done: `fetching origin/main`, `cargo build`.
        step: &'static str,
        /// Why it failed.
        cause: ExecError,
        /// The last [`BUILD_TAIL_LINES`] lines of the build's output.
        tail: String,
    },
    /// The app quit while building.
    #[error("the build was cancelled")]
    Cancelled,
}

/// What [`build`] needs to know.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    /// The Dovetail checkout: the project's main checkout.
    pub repo: PathBuf,
    /// What to build.
    pub source: Source,
    /// The config folder, whose installed role prompts and recipe are compared with the new
    /// build's.
    pub config_dir: PathBuf,
    /// Where the project's installed recipe lives, to compare `config/projects/dovetail.toml`
    /// with.
    pub recipe: Option<PathBuf>,
}

/// Builds the next binary into `staging`. Blocking; run it on a thread. Nothing outside
/// `staging` changes except git's bookkeeping in `request.repo` (a fetch, a short-lived
/// worktree), and the installed binary is never touched.
pub fn build(
    exec: &dyn Exec,
    staging: &Staging,
    request: &BuildRequest,
    cancel: &AtomicBool,
    now_ms: i64,
) -> Result<Ready, BuildError> {
    if !request.repo.join("crates/dovetail/Cargo.toml").is_file() {
        return Err(UpgradeError::NotDovetail(request.repo.clone()).into());
    }
    fs::create_dir_all(&staging.root).map_err(io_err("creating the staging folder"))?;
    let _lock = BuildLock::acquire(&staging.lock())?;
    staging.clear();
    fs::create_dir_all(staging.staged_dir()).map_err(io_err("creating the staging folder"))?;
    fs::write(staging.log(), "").map_err(io_err("clearing the build log"))?;

    let result = build_in_lock(exec, staging, request, cancel, now_ms);
    if request.source == Source::OriginMain {
        remove_worktree(exec, staging, request);
    }
    result
}

fn build_in_lock(
    exec: &dyn Exec,
    staging: &Staging,
    request: &BuildRequest,
    cancel: &AtomicBool,
    now_ms: i64,
) -> Result<Ready, BuildError> {
    let log = staging.log();
    let step = |step: &'static str, call: Call| {
        exec.run(&call, cancel).map_err(|cause| match cause {
            ExecError::Cancelled => BuildError::Cancelled,
            cause => BuildError::Step {
                step,
                cause,
                tail: tail(&log, BUILD_TAIL_LINES),
            },
        })
    };
    let git = |cwd: &Path, args: &[&str], log: bool, env: Vec<(String, String)>| Call {
        program: "git".into(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: cwd.to_owned(),
        env,
        log: log.then(|| staging.log()),
    };

    let (src, label) = match request.source {
        Source::OriginMain => {
            // A worktree left by a build that was killed.
            remove_worktree(exec, staging, request);
            let fetch_env =
                crate::worktree_info::fetch_env(std::env::var("GIT_SSH_COMMAND").ok().as_deref())
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect();
            step(
                "fetching origin/main",
                git(
                    &request.repo,
                    &["fetch", "--quiet", "origin", SOURCE_BRANCH],
                    true,
                    fetch_env,
                ),
            )?;
            let src = staging.src();
            let src_arg = src.to_string_lossy().into_owned();
            let remote = format!("origin/{SOURCE_BRANCH}");
            step(
                "checking out origin/main",
                git(
                    &request.repo,
                    &["worktree", "add", "--detach", &src_arg, &remote],
                    true,
                    Vec::new(),
                ),
            )?;
            (src, None)
        }
        Source::Local => (request.repo.clone(), Some("local checkout")),
    };
    let id = step(
        "reading the commit",
        git(&src, &["rev-parse", "--short", "HEAD"], false, Vec::new()),
    )?;
    let source = match label {
        Some(label) => format!("{} ({label})", id.trim()),
        None => id.trim().to_owned(),
    };

    let target_dir = staging.target_dir();
    step(
        "cargo build",
        Call {
            program: "cargo".into(),
            args: ["build", "--release", "--locked", "--package", "dovetail"]
                .map(String::from)
                .to_vec(),
            cwd: src.clone(),
            env: vec![(
                "CARGO_TARGET_DIR".into(),
                target_dir.to_string_lossy().into_owned(),
            )],
            log: Some(log.clone()),
        },
    )?;

    let built = target_dir.join("release/dovetail");
    let partial = staging.staged_dir().join("dovetail.partial");
    fs::copy(&built, &partial).map_err(io_err(format!("copying {}", built.display())))?;
    fs::rename(&partial, staging.binary()).map_err(io_err("staging the binary"))?;

    let ready = Ready {
        source,
        differing: differing(&src, &request.config_dir, request.recipe.as_deref()),
        built_at_ms: now_ms,
    };
    let text = serde_json::to_string(&ready).map_err(|e| io_err("writing the note")(e.into()))?;
    fs::write(staging.ready_file(), text).map_err(io_err("writing the note"))?;
    Ok(ready)
}

/// Removes the throwaway worktree, best effort: the next build starts clean either way.
fn remove_worktree(exec: &dyn Exec, staging: &Staging, request: &BuildRequest) {
    let src = staging.src();
    if !src.exists() {
        return;
    }
    // Never cancelled: cleanup must still run when the build itself was.
    let never = AtomicBool::new(false);
    let remove = Call {
        program: "git".into(),
        args: vec![
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            src.to_string_lossy().into_owned(),
        ],
        cwd: request.repo.clone(),
        env: Vec::new(),
        log: None,
    };
    if let Err(e) = exec.run(&remove, &never) {
        tracing::warn!("upgrade: removing the throwaway worktree: {e}");
    }
    if src.exists()
        && let Err(e) = fs::remove_dir_all(&src)
    {
        tracing::warn!("upgrade: removing {}: {e}", src.display());
    }
    let prune = Call {
        args: vec!["worktree".into(), "prune".into()],
        ..remove
    };
    if let Err(e) = exec.run(&prune, &never) {
        tracing::warn!("upgrade: pruning worktrees: {e}");
    }
}

/// The last `lines` lines of the file, or nothing when it cannot be read.
fn tail(path: &Path, lines: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Role prompts and the recipe of the new build whose installed copy is missing or different.
fn differing(src: &Path, config_dir: &Path, recipe: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    let differs = |new: &Path, installed: &Path| match (fs::read(new), fs::read(installed)) {
        (Ok(a), Ok(b)) => a != b,
        (Ok(_), Err(_)) => true,
        // Nothing to compare: not a difference to report.
        (Err(_), _) => false,
    };
    if let Ok(entries) = fs::read_dir(src.join("config/roles")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if Path::new(&name).extension().is_some_and(|e| e == "md")
                && differs(&entry.path(), &config_dir.join("roles").join(&name))
            {
                out.push(format!("roles/{}", name.to_string_lossy()));
            }
        }
    }
    if let Some(recipe) = recipe
        && differs(&src.join("config/projects/dovetail.toml"), recipe)
    {
        out.push("projects/dovetail.toml".into());
    }
    out.sort();
    out
}

/// Held while a build runs, so the app and `dovetail upgrade` never build into the staging
/// folder at once.
struct BuildLock {
    path: PathBuf,
}

impl BuildLock {
    fn acquire(path: &Path) -> Result<Self, UpgradeError> {
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    let _ = write!(file, "{}", std::process::id());
                    return Ok(Self {
                        path: path.to_owned(),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let pid = fs::read_to_string(path)
                        .ok()
                        .and_then(|t| t.trim().parse::<u32>().ok());
                    match pid {
                        Some(pid) if Path::new(&format!("/proc/{pid}")).exists() => {
                            return Err(UpgradeError::Busy(pid));
                        }
                        // Left by a build that died.
                        _ => {
                            fs::remove_file(path).map_err(io_err("removing a stale build lock"))?;
                        }
                    }
                }
                Err(e) => return Err(io_err("taking the build lock")(e)),
            }
        }
        Err(UpgradeError::Busy(0))
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// What is running the installed binary: pids of processes whose executable it is (the app, and
/// the `dovetail mcp` and `dovetail hook` helpers its sessions start), and sockets in `runtime`
/// something answers on (an app, which `dovetail add` and the bridges talk to). `own_pid` is
/// left out: the caller is about to hand over. `proc_root` is `/proc`.
pub fn running_blockers(
    install: &Path,
    runtime: &Path,
    proc_root: &Path,
    own_pid: u32,
) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(real) = install.canonicalize() {
        let deleted = format!("{} (deleted)", real.display());
        if let Ok(entries) = fs::read_dir(proc_root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
                    continue;
                };
                if pid == own_pid {
                    continue;
                }
                let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
                    continue;
                };
                // A binary replaced while running reads as `<path> (deleted)`.
                if exe == real || exe.to_string_lossy() == deleted {
                    found.push(format!("pid {pid}"));
                }
            }
        }
    }
    if let Ok(entries) = fs::read_dir(runtime) {
        for entry in entries.flatten() {
            let path = entry.path();
            // A crash can leave a stale file: only a socket something accepts on counts.
            if path.extension().is_some_and(|e| e == "sock") && UnixStream::connect(&path).is_ok() {
                found.push(path.display().to_string());
            }
        }
    }
    found.sort();
    found
}

/// What [`install_staged`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Swap {
    /// The staged binary now is the installed one.
    Installed(Ready),
    /// Nothing was staged.
    NothingStaged,
}

/// Why the staged binary was not installed.
#[derive(Debug, thiserror::Error)]
pub enum SwapError {
    /// Something still runs the installed binary. The build stays staged.
    #[error("not installed, still running: {}", .0.join(", "))]
    Blocked(Vec<String>),
    /// Copying or renaming failed. The installed binary is unchanged.
    #[error(transparent)]
    Upgrade(#[from] UpgradeError),
}

/// Where the swap looks for what still runs.
#[derive(Debug, Clone)]
pub struct SwapGuard {
    /// Where the app sockets live.
    pub runtime: PathBuf,
    /// `/proc`.
    pub proc_root: PathBuf,
    /// The caller's pid, which does not count.
    pub own_pid: u32,
    /// How long to wait for things to exit (the app's own helpers, at a quit) before giving up.
    pub wait: Duration,
}

/// Renames the staged binary over `install`, unless something runs it ([`running_blockers`]). The
/// binary is first copied next to the target, so the rename is atomic and works across
/// filesystems. The staged build is forgotten once installed.
pub fn install_staged(
    staging: &Staging,
    install: &Path,
    guard: &SwapGuard,
) -> Result<Swap, SwapError> {
    let Some(ready) = staging.ready() else {
        return Ok(Swap::NothingStaged);
    };
    let deadline = Instant::now() + guard.wait;
    loop {
        let blockers = running_blockers(install, &guard.runtime, &guard.proc_root, guard.own_pid);
        if blockers.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(SwapError::Blocked(blockers));
        }
        std::thread::sleep(POLL);
    }
    let dir = install.parent().ok_or(UpgradeError::NoHome)?;
    fs::create_dir_all(dir).map_err(io_err(format!("creating {}", dir.display())))?;
    let next = dir.join(format!(".dovetail.upgrade-{}", guard.own_pid));
    fs::copy(staging.binary(), &next).map_err(io_err(format!("copying to {}", next.display())))?;
    if let Err(e) = fs::rename(&next, install) {
        let _ = fs::remove_file(&next);
        return Err(io_err(format!("replacing {}", install.display()))(e).into());
    }
    staging.clear();
    Ok(Swap::Installed(ready))
}

#[cfg(test)]
mod tests;

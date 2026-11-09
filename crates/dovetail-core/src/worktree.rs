//! Slice worktrees: creation for the `WorktreeCreate` hook, recipe setup, inspection and removal.
//!
//! Layout: a worktree named `<name>` lives at `Paths::worktrees(project)/<name>` on branch
//! `dovetail/<name>`. Slices are named after the slice; subagent worktrees are named
//! `agent-<id>` (so their branch is `dovetail/agent-<id>`). Nothing is written into the user's
//! repo except the branch and git's own worktree bookkeeping.
//!
//! Setup state lives in the directory `Paths::setup_state(project, name)`:
//! `state.json` ([`SetupState`]), `setup.log` (output of the setup commands) and `links`
//! (one symlink path per line, relative to the worktree, so inspect and remove know which
//! untracked entries are ours).
//!
//! All git work goes through the `git` CLI.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::recipe::Recipe;

/// Prefix of every branch Dovetail creates.
pub const BRANCH_PREFIX: &str = "dovetail/";
/// Name prefix that marks a subagent worktree (symlinks, no setup).
pub const SUBAGENT_PREFIX: &str = "agent-";

/// Errors from worktree operations.
#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    /// The name cannot be used as a directory and branch name.
    #[error("invalid worktree name {0:?}")]
    InvalidName(String),
    /// A git command failed.
    #[error("git {args} failed: {stderr}")]
    Git {
        /// The arguments, space-joined.
        args: String,
        /// Trimmed stderr.
        stderr: String,
    },
    /// Filesystem or process error.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// The cause.
        source: std::io::Error,
    },
    /// A setup command failed.
    #[error("worktree setup failed ({reason}); log: {}", .log.display())]
    SetupFailed {
        /// Which command failed and how.
        reason: String,
        /// The setup log.
        log: PathBuf,
    },
    /// Removal refused: the worktree has uncommitted or unpushed work and was not confirmed.
    #[error("worktree has unsaved work (uncommitted: {}, unpushed commits: {})", .0.uncommitted.len(), .0.unpushed_commits)]
    Unsafe(Box<Inspection>),
    /// The state file could not be written.
    #[error("setup state {path}: {message}")]
    State {
        /// The state file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> WorktreeError {
    let context = context.into();
    move |source| WorktreeError::Io { context, source }
}

/// What kind of session the worktree is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A slice session: symlinks and recipe setup.
    Slice,
    /// A subagent: symlinks only, to stay fast (B12).
    Subagent,
}

impl Kind {
    /// Subagent worktrees are named `agent-<id>`; everything else is a slice.
    pub fn from_name(name: &str) -> Self {
        if name.starts_with(SUBAGENT_PREFIX) {
            Kind::Subagent
        } else {
            Kind::Slice
        }
    }
}

/// Progress of setup for one worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStatus {
    /// Setup commands are executing (or were interrupted: see `pid`).
    Running,
    /// Every setup command succeeded.
    Done,
    /// A setup command failed.
    Failed,
}

/// Contents of `state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupState {
    /// Where setup stands.
    pub status: SetupStatus,
    /// Process that runs setup, to tell a live run from an interrupted one.
    pub pid: u32,
    /// The setup log.
    pub log: PathBuf,
    /// Failure reason, when `status` is `Failed`.
    #[serde(default)]
    pub error: Option<String>,
}

/// Input to [`create`].
#[derive(Debug, Clone)]
pub struct CreateRequest {
    /// Project id.
    pub project: String,
    /// Worktree name: the slice name, or `agent-<id>` for a subagent.
    pub name: String,
    /// Directory git runs in and whose `HEAD` the branch starts from: the main checkout for a
    /// slice, the parent's worktree for a subagent.
    pub cwd: PathBuf,
    /// The project's recipe.
    pub recipe: Recipe,
}

/// Result of [`create`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// The worktree path (what the hook prints).
    pub path: PathBuf,
    /// The branch, `dovetail/<name>`.
    pub branch: String,
    /// The setup log, when the recipe has setup commands and this is a slice.
    pub setup_log: Option<PathBuf>,
}

/// State of a worktree, from [`inspect`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inspection {
    /// Whether the worktree directory exists.
    pub exists: bool,
    /// Whether the `dovetail/<name>` branch exists.
    pub branch_exists: bool,
    /// Uncommitted entries (git status entries), excluding our own symlinks.
    pub uncommitted: Vec<String>,
    /// Commits not pushed: ahead of upstream, or, with no upstream, on no remote and not in the
    /// main checkout's `HEAD`.
    pub unpushed_commits: usize,
    /// Whether the branch has an upstream.
    pub has_upstream: bool,
}

impl Inspection {
    /// True when removal needs no confirmation.
    pub fn is_clean(&self) -> bool {
        self.uncommitted.is_empty() && self.unpushed_commits == 0
    }
}

/// Branch name for a worktree name.
pub fn branch_name(name: &str) -> String {
    format!("{BRANCH_PREFIX}{name}")
}

/// The project key a worktree path was made under (`<data>/projects/<key>/worktrees/<name>`),
/// so a worktree made before registration is found where it is.
pub fn project_of(paths: &Paths, worktree: &Path) -> Option<String> {
    let rest = worktree.strip_prefix(paths.data.join("projects")).ok()?;
    let mut parts = rest.components();
    let key = parts.next()?.as_os_str().to_string_lossy().into_owned();
    (parts.next()?.as_os_str() == "worktrees").then_some(key)
}

/// Path of the worktree.
pub fn worktree_path(paths: &Paths, project: &str, name: &str) -> PathBuf {
    paths.worktrees(project).join(name)
}

/// Reject names that are unsafe as a directory or branch component.
pub fn validate_name(name: &str) -> Result<(), WorktreeError> {
    let ok = !name.is_empty()
        && name.len() <= 100
        && !name.starts_with(['.', '-'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(WorktreeError::InvalidName(name.to_owned()))
    }
}

fn git<I, S>(dir: &Path, args: I) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<S> = args.into_iter().collect();
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .map_err(io("running git"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(WorktreeError::Git {
            args: args
                .iter()
                .map(|a| a.as_ref().to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

fn git_ok<I, S>(dir: &Path, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The worktrees `git worktree list` names for the repository at `repo`, main checkout first.
pub fn listed_worktrees(repo: &Path) -> Result<Vec<PathBuf>, WorktreeError> {
    let out = git(repo, ["worktree", "list", "--porcelain"])?;
    Ok(out
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect())
}

/// The main checkout of the repository that contains `dir` (which may be one of its worktrees).
/// The same rule as project identity: this is [`crate::projects::identify`].
pub fn main_checkout(dir: &Path) -> Result<PathBuf, WorktreeError> {
    crate::projects::identify(dir)
        .map(|identity| identity.repo_path)
        .map_err(|e| WorktreeError::Git {
            args: "rev-parse --path-format=absolute --git-common-dir".into(),
            stderr: e.to_string(),
        })
}

struct StatePaths {
    dir: PathBuf,
    state: PathBuf,
    log: PathBuf,
    links: PathBuf,
}

fn state_paths(paths: &Paths, project: &str, name: &str) -> StatePaths {
    let dir = paths.setup_state(project, name);
    StatePaths {
        state: dir.join("state.json"),
        log: dir.join("setup.log"),
        links: dir.join("links"),
        dir,
    }
}

fn read_state(path: &Path) -> Option<SetupState> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// What the `WorktreeCreate` hook reports when it finishes (`hook.json` in the setup state
/// directory). The core removes it before launching a slice and waits for it, so it knows the
/// worktree is ready, or failed, before it sends the first prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookOutcome {
    /// The worktree path, on success.
    pub path: Option<PathBuf>,
    /// What went wrong, on failure.
    pub error: Option<String>,
}

/// Where the hook writes its [`HookOutcome`].
pub fn hook_outcome_path(paths: &Paths, project: &str, name: &str) -> PathBuf {
    state_paths(paths, project, name).dir.join("hook.json")
}

/// Records the hook's outcome (called by `dovetail hook worktree-create`).
pub fn write_hook_outcome(
    paths: &Paths,
    project: &str,
    name: &str,
    outcome: &HookOutcome,
) -> Result<(), WorktreeError> {
    let path = hook_outcome_path(paths, project, name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io(format!("creating {}", dir.display())))?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(outcome).map_err(|e| WorktreeError::State {
        path: path.clone(),
        message: e.to_string(),
    })?;
    fs::write(&tmp, bytes).map_err(io(format!("writing {}", tmp.display())))?;
    fs::rename(&tmp, &path).map_err(io(format!("writing {}", path.display())))
}

/// Reads the hook's outcome, if it has finished.
pub fn read_hook_outcome(paths: &Paths, project: &str, name: &str) -> Option<HookOutcome> {
    serde_json::from_slice(&fs::read(hook_outcome_path(paths, project, name)).ok()?).ok()
}

/// Forgets an earlier outcome, before a new launch.
pub fn clear_hook_outcome(paths: &Paths, project: &str, name: &str) {
    // Nothing there is the usual case.
    let _ = fs::remove_file(hook_outcome_path(paths, project, name));
}

fn write_state(path: &Path, state: &SetupState) -> Result<(), WorktreeError> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(state).map_err(|e| WorktreeError::State {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    fs::write(&tmp, bytes).map_err(io(format!("writing {}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(io(format!("writing {}", path.display())))
}

fn pid_alive(pid: u32) -> bool {
    pid != std::process::id() && Path::new("/proc").join(pid.to_string()).exists()
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git_ok(
        repo,
        [
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
}

/// Create the worktree for `req` (or return the existing one), symlink the recipe's paths, and
/// run setup for slices. Idempotent: a repeat call after completed setup returns at once; an
/// interrupted or failed setup runs again.
pub fn create(paths: &Paths, req: &CreateRequest) -> Result<Created, WorktreeError> {
    validate_name(&req.name)?;
    let kind = Kind::from_name(&req.name);
    let repo = main_checkout(&req.cwd)?;
    let path = worktree_path(paths, &req.project, &req.name);
    let branch = branch_name(&req.name);
    let sp = state_paths(paths, &req.project, &req.name);
    fs::create_dir_all(&sp.dir).map_err(io(format!("creating {}", sp.dir.display())))?;

    // Wait for a concurrent live setup of the same worktree instead of racing it.
    while let Some(s) = read_state(&sp.state) {
        if s.status == SetupStatus::Running && pid_alive(s.pid) {
            std::thread::sleep(Duration::from_millis(500));
        } else {
            break;
        }
    }

    if !path.join(".git").exists() {
        fs::create_dir_all(paths.worktrees(&req.project)).map_err(io("creating worktrees dir"))?;
        git(&repo, ["worktree", "prune"])?;
        if branch_exists(&repo, &branch) {
            git(
                &repo,
                [
                    OsStr::new("worktree"),
                    "add".as_ref(),
                    path.as_os_str(),
                    branch.as_ref(),
                ],
            )?;
        } else {
            git(
                &req.cwd,
                [
                    OsStr::new("worktree"),
                    "add".as_ref(),
                    "-b".as_ref(),
                    branch.as_ref(),
                    path.as_os_str(),
                    "HEAD".as_ref(),
                ],
            )?;
        }
    }

    let links = link_paths(&repo, &path, &req.recipe.worktree.symlink)?;
    if !links.is_empty() {
        let text: String = links.iter().map(|l| format!("{}\n", l.display())).collect();
        fs::write(&sp.links, text).map_err(io("recording symlinks"))?;
    }

    if kind == Kind::Slice && !req.recipe.worktree.setup.is_empty() {
        run_setup(&path, &req.recipe, &sp.state, &sp.log)?;
        Ok(Created {
            path,
            branch,
            setup_log: Some(sp.log),
        })
    } else {
        Ok(Created {
            path,
            branch,
            setup_log: None,
        })
    }
}

/// Symlink each recipe path from the main checkout into the worktree. Returns the relative paths
/// of links that exist afterwards. Missing sources and occupied destinations are skipped.
fn link_paths(repo: &Path, wt: &Path, wanted: &[PathBuf]) -> Result<Vec<PathBuf>, WorktreeError> {
    let mut made = Vec::new();
    for rel in wanted {
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let src = repo.join(rel);
        let dst = wt.join(rel);
        if fs::symlink_metadata(&src).is_err() {
            continue;
        }
        match fs::symlink_metadata(&dst) {
            Ok(m) if m.file_type().is_symlink() => made.push(rel.clone()),
            Ok(_) => {}
            Err(_) => {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .map_err(io(format!("creating {}", parent.display())))?;
                }
                std::os::unix::fs::symlink(&src, &dst)
                    .map_err(io(format!("linking {}", dst.display())))?;
                made.push(rel.clone());
            }
        }
    }
    Ok(made)
}

fn run_setup(
    wt: &Path,
    recipe: &Recipe,
    state_file: &Path,
    log: &Path,
) -> Result<(), WorktreeError> {
    if matches!(read_state(state_file), Some(s) if s.status == SetupStatus::Done) {
        return Ok(());
    }
    let mut state = SetupState {
        status: SetupStatus::Running,
        pid: std::process::id(),
        log: log.to_owned(),
        error: None,
    };
    write_state(state_file, &state)?;
    let logf = fs::File::create(log).map_err(io("creating setup log"))?;
    let envs: BTreeMap<String, String> = recipe.env_for(wt);
    for cmd in &recipe.worktree.setup {
        let out = logf.try_clone().map_err(io("cloning log handle"))?;
        let err = logf.try_clone().map_err(io("cloning log handle"))?;
        {
            use std::io::Write;
            let mut w = &logf;
            // Best effort: a failed log line must not fail setup.
            let _ = writeln!(w, "$ {cmd}");
        }
        let status = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(wt)
            .envs(&envs)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .status();
        let failure = match status {
            Ok(s) if s.success() => None,
            Ok(s) => Some(format!("`{cmd}` exited with {s}")),
            Err(e) => Some(format!("`{cmd}` could not start: {e}")),
        };
        if let Some(reason) = failure {
            state.status = SetupStatus::Failed;
            state.error = Some(reason.clone());
            write_state(state_file, &state)?;
            return Err(WorktreeError::SetupFailed {
                reason,
                log: log.to_owned(),
            });
        }
    }
    state.status = SetupStatus::Done;
    write_state(state_file, &state)
}

/// Read the recorded setup state, if any.
pub fn setup_state(paths: &Paths, project: &str, name: &str) -> Option<SetupState> {
    read_state(&state_paths(paths, project, name).state)
}

fn recorded_links(links_file: &Path) -> Vec<PathBuf> {
    fs::read_to_string(links_file)
        .map(|t| t.lines().map(PathBuf::from).collect())
        .unwrap_or_default()
}

/// Report uncommitted changes, unpushed commits and branch existence. `repo` is any checkout of
/// the project (normally the main one).
pub fn inspect(
    paths: &Paths,
    project: &str,
    name: &str,
    repo: &Path,
) -> Result<Inspection, WorktreeError> {
    validate_name(name)?;
    let wt = worktree_path(paths, project, name);
    let branch = branch_name(name);
    let mut out = Inspection {
        exists: wt.join(".git").exists(),
        branch_exists: branch_exists(repo, &branch),
        ..Inspection::default()
    };
    if out.exists {
        let ours = recorded_links(&state_paths(paths, project, name).links);
        let status = git(&wt, ["status", "--porcelain", "-z"])?;
        let mut entries = status.split('\0').filter(|e| !e.is_empty());
        while let Some(e) = entries.next() {
            let path = e.get(3..).unwrap_or("");
            if e.starts_with('R') || e.starts_with('C') {
                entries.next(); // rename source
            }
            let is_ours = e.starts_with("??")
                && ours
                    .iter()
                    .any(|l| Path::new(path.trim_end_matches('/')) == l);
            if !is_ours {
                out.uncommitted.push(e.to_owned());
            }
        }
    }
    if out.branch_exists {
        let upstream = format!("{branch}@{{upstream}}");
        out.has_upstream = git_ok(repo, ["rev-parse", "--verify", "--quiet", &upstream]);
        let count = if out.has_upstream {
            git(
                repo,
                ["rev-list", "--count", &format!("{upstream}..{branch}")],
            )?
        } else {
            // Only commits after the branch point count: on no remote, and not in the main
            // checkout's HEAD either (work merged there is safe).
            git(
                repo,
                ["rev-list", "--count", &branch, "--not", "--remotes", "HEAD"],
            )?
        };
        out.unpushed_commits = count.trim().parse().unwrap_or(0);
    }
    Ok(out)
}

/// Remove a worktree: our symlinks and setup state, `git worktree remove`, then the
/// `dovetail/*` branch. Refuses with [`WorktreeError::Unsafe`] unless the worktree is clean and
/// pushed, or `confirmed` is true. Removing something already gone succeeds.
pub fn remove(
    paths: &Paths,
    project: &str,
    name: &str,
    repo: &Path,
    confirmed: bool,
) -> Result<(), WorktreeError> {
    let report = inspect(paths, project, name, repo)?;
    if !confirmed && !report.is_clean() {
        return Err(WorktreeError::Unsafe(Box::new(report)));
    }
    let wt = worktree_path(paths, project, name);
    let sp = state_paths(paths, project, name);
    for rel in recorded_links(&sp.links) {
        let p = wt.join(&rel);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            fs::remove_file(&p).map_err(io(format!("removing {}", p.display())))?;
        }
    }
    if report.exists {
        let mut args = vec![OsStr::new("worktree"), "remove".as_ref()];
        if confirmed {
            args.push("--force".as_ref());
        }
        args.push(wt.as_os_str());
        git(repo, args)?;
    } else if wt.exists() {
        // A directory that is not a worktree (failed create): ours to clean.
        fs::remove_dir_all(&wt).map_err(io(format!("removing {}", wt.display())))?;
    }
    git(repo, ["worktree", "prune"])?;
    if report.branch_exists {
        git(repo, ["branch", "-D", &branch_name(name)])?;
    }
    if sp.dir.exists() {
        fs::remove_dir_all(&sp.dir).map_err(io("removing setup state"))?;
    }
    Ok(())
}

/// What [`remove_if_unchanged`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentCleanup {
    /// There was no such worktree or branch: the subagent shared its parent's.
    Absent,
    /// The worktree was unchanged and is gone, with its branch.
    Removed,
    /// The subagent left work behind; nothing was touched.
    Changed {
        /// Uncommitted entries, excluding our symlinks.
        uncommitted: usize,
        /// Commits that exist only on the subagent's branch.
        commits: usize,
    },
}

/// Commits reachable from `branch` and from no other branch or remote: the work done on it.
/// A subagent's branch starts from its parent's `HEAD`, so the parent's own unpushed commits
/// are not counted.
fn own_commits(repo: &Path, branch: &str) -> Result<usize, WorktreeError> {
    // With `--branches` the pattern names the branch without `refs/heads/` (git-rev-list(1)).
    let exclude = format!("--exclude={branch}");
    let count = git(
        repo,
        [
            "rev-list",
            "--count",
            branch,
            "--not",
            exclude.as_str(),
            "--branches",
            "--remotes",
        ],
    )?;
    Ok(count.trim().parse().unwrap_or(0))
}

/// Removes a finished subagent's worktree (`agent-<id>`) if it is unchanged: no uncommitted
/// entries besides our symlinks and no commits of its own (digest §6: `WorktreeRemove` does not
/// fire when subagents finish, so Dovetail cleans up). A changed one is left for the parent.
pub fn remove_if_unchanged(
    paths: &Paths,
    project: &str,
    name: &str,
    repo: &Path,
) -> Result<SubagentCleanup, WorktreeError> {
    let report = inspect(paths, project, name, repo)?;
    if !report.exists && !report.branch_exists {
        return Ok(SubagentCleanup::Absent);
    }
    let commits = if report.branch_exists {
        own_commits(repo, &branch_name(name))?
    } else {
        0
    };
    if !report.uncommitted.is_empty() || commits > 0 {
        return Ok(SubagentCleanup::Changed {
            uncommitted: report.uncommitted.len(),
            commits,
        });
    }
    // Checked above; the parent's unpushed commits on the branch are not the subagent's work,
    // so `remove`'s own unpushed check would refuse wrongly.
    remove(paths, project, name, repo, true)?;
    Ok(SubagentCleanup::Removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A repo with a slice worktree that has an unpushed commit of its own, and a subagent
    /// worktree made from the slice (as the hook does), with `.env` symlinked into both.
    fn scratch() -> (tempfile::TempDir, Paths, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            data: dir.path().join("data"),
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            runtime: dir.path().join("run"),
        };
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).expect("repo");
        run_git(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "a\n").expect("write");
        fs::write(repo.join(".env"), "SECRET=1\n").expect("write");
        fs::write(repo.join(".gitignore"), ".env\n").expect("write");
        run_git(&repo, &["add", "a.txt", ".gitignore"]);
        run_git(&repo, &["commit", "-q", "-m", "init"]);
        let recipe = Recipe {
            worktree: crate::recipe::WorktreeRecipe {
                symlink: vec![PathBuf::from(".env")],
                setup: Vec::new(),
            },
            ..Recipe::default()
        };
        let slice = create(
            &paths,
            &CreateRequest {
                project: "p".into(),
                name: "slice".into(),
                cwd: repo.clone(),
                recipe: recipe.clone(),
            },
        )
        .expect("slice worktree");
        fs::write(slice.path.join("b.txt"), "b\n").expect("write");
        run_git(&slice.path, &["add", "b.txt"]);
        run_git(&slice.path, &["commit", "-q", "-m", "slice work"]);
        let sub = create(
            &paths,
            &CreateRequest {
                project: "p".into(),
                name: "agent-a1".into(),
                cwd: slice.path.clone(),
                recipe,
            },
        )
        .expect("subagent worktree");
        assert!(fs::symlink_metadata(sub.path.join(".env")).is_ok());
        (dir, paths, repo, sub.path)
    }

    #[test]
    fn an_unchanged_subagent_worktree_is_removed_with_its_branch() {
        let (_dir, paths, repo, sub) = scratch();
        let done = remove_if_unchanged(&paths, "p", "agent-a1", &repo).expect("cleanup");
        assert_eq!(done, SubagentCleanup::Removed);
        assert!(!sub.exists());
        assert!(!branch_exists(&repo, "dovetail/agent-a1"));
        // The slice is untouched.
        assert!(branch_exists(&repo, "dovetail/slice"));
        assert!(worktree_path(&paths, "p", "slice").join("b.txt").exists());
    }

    #[test]
    fn a_subagent_worktree_with_work_is_left_alone() {
        let (_dir, paths, repo, sub) = scratch();
        fs::write(sub.join("c.txt"), "c\n").expect("write");
        let found = remove_if_unchanged(&paths, "p", "agent-a1", &repo).expect("cleanup");
        assert_eq!(
            found,
            SubagentCleanup::Changed {
                uncommitted: 1,
                commits: 0
            }
        );
        run_git(&sub, &["add", "c.txt"]);
        run_git(&sub, &["commit", "-q", "-m", "sub work"]);
        let found = remove_if_unchanged(&paths, "p", "agent-a1", &repo).expect("cleanup");
        assert_eq!(
            found,
            SubagentCleanup::Changed {
                uncommitted: 0,
                commits: 1
            }
        );
        assert!(sub.join("c.txt").exists());
    }

    #[test]
    fn a_subagent_without_a_worktree_needs_no_cleanup() {
        let (_dir, paths, repo, _sub) = scratch();
        let found = remove_if_unchanged(&paths, "p", "agent-none", &repo).expect("cleanup");
        assert_eq!(found, SubagentCleanup::Absent);
    }

    #[test]
    fn a_worktree_path_names_the_key_it_was_made_under() {
        let paths = Paths {
            data: PathBuf::from("/d"),
            config: PathBuf::from("/c"),
            state: PathBuf::from("/s"),
            runtime: PathBuf::from("/r"),
        };
        let path = worktree_path(&paths, "new-42", "early");
        assert_eq!(project_of(&paths, &path).as_deref(), Some("new-42"));
        assert_eq!(project_of(&paths, Path::new("/elsewhere/early")), None);
    }
}

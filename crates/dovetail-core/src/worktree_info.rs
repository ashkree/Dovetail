//! Data for the worktree view (M12b): sync state, changes, unpushed commits, PR and setup of
//! each slice's worktree.
//!
//! Everything here is read-only. Each part is gathered on its own, through `git` and `gh`
//! child processes run by tokio with a timeout (a child that overruns is killed, and tokio
//! reaps it). A part that fails is logged and shown as unknown; it never hides the others.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dovetail_api::{
    ActivityKind, AgentId, AgentKind, Changes, Event, PrLookup, PrState, PullRequest, SyncState,
    WorktreeInfo, WorktreeSetup,
};
use serde::Deserialize;

use crate::db::now_ms;
use crate::paths::Paths;
use crate::service::{Core, Input};
use crate::worktree::{self, SetupStatus};

/// Longest a `git` command may run before it is killed. Every one here is local.
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest a network command (`git fetch`, `gh pr list`) may run before it is killed.
pub(crate) const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);
/// Most PRs asked of `gh` per branch; an open one among them wins.
const PR_LIMIT: &str = "10";

/// Why one part of a worktree's state could not be read.
#[derive(Debug, thiserror::Error)]
pub enum InfoError {
    /// The program is not installed.
    #[error("{0} is not installed")]
    Missing(String),
    /// The program could not be started or waited for.
    #[error("{program} could not run: {source}")]
    Io {
        /// The program.
        program: String,
        /// The cause.
        source: std::io::Error,
    },
    /// The program ran past its timeout and was killed.
    #[error("{program} took longer than {}s", .timeout.as_secs())]
    Timeout {
        /// The program.
        program: String,
        /// The limit it ran past.
        timeout: Duration,
    },
    /// The program exited with an error.
    #[error("{program} {args} failed: {stderr}")]
    Failed {
        /// The program.
        program: String,
        /// Its arguments, space-joined.
        args: String,
        /// Trimmed stderr.
        stderr: String,
    },
    /// The worktree directory is gone.
    #[error("no worktree at {}", .0.display())]
    NoWorktree(PathBuf),
    /// The program printed something we could not read.
    #[error("unexpected {what}: {output:?}")]
    Parse {
        /// What was being read.
        what: &'static str,
        /// The offending output, trimmed.
        output: String,
    },
}

fn parse_error(what: &'static str, output: &str) -> InfoError {
    InfoError::Parse {
        what,
        output: output.trim().to_owned(),
    }
}

/// Runs `program args` in `dir` and returns its stdout. Killed (and reaped by tokio) if it runs
/// past `timeout`.
async fn run<S: AsRef<OsStr>>(
    dir: &Path,
    program: &Path,
    args: &[S],
    timeout: Duration,
) -> Result<String, InfoError> {
    run_env(dir, program, args, &[], timeout).await
}

/// [`run`] with extra environment variables.
async fn run_env<S: AsRef<OsStr>>(
    dir: &Path,
    program: &Path,
    args: &[S],
    env: &[(&str, String)],
    timeout: Duration,
) -> Result<String, InfoError> {
    let name = program.display().to_string();
    let child = tokio::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        // Git must never stop to ask for credentials: it fails instead.
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => InfoError::Missing(name.clone()),
            _ => InfoError::Io {
                program: name.clone(),
                source,
            },
        })?;
    // On timeout the child is dropped, which kills it; tokio reaps it in the background.
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| InfoError::Timeout {
            program: name.clone(),
            timeout,
        })?
        .map_err(|source| InfoError::Io {
            program: name.clone(),
            source,
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(InfoError::Failed {
            program: name,
            args: args
                .iter()
                .map(|a| a.as_ref().to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

async fn git<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> Result<String, InfoError> {
    run(dir, Path::new("git"), args, GIT_TIMEOUT).await
}

/// Whether a `git` check command succeeded: `Ok(false)` when git says no, an error when git
/// could not answer at all.
async fn git_check<S: AsRef<OsStr>>(dir: &Path, args: &[S]) -> Result<bool, InfoError> {
    match git(dir, args).await {
        Ok(_) => Ok(true),
        Err(InfoError::Failed { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

fn parse_count(output: &str) -> Result<u32, InfoError> {
    output
        .trim()
        .parse()
        .map_err(|_| parse_error("commit count", output))
}

/// Parses `git rev-list --left-right --count <base>...<branch>`: `behind<TAB>ahead`.
pub(crate) fn parse_left_right(output: &str) -> Result<(u32, u32), InfoError> {
    let mut parts = output.split_whitespace();
    let (Some(left), Some(right), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(parse_error("left-right count", output));
    };
    let count = |s: &str| {
        s.parse()
            .map_err(|_| parse_error("left-right count", output))
    };
    Ok((count(left)?, count(right)?))
}

/// Counts `git status --porcelain` entries (`XY path`, as [`worktree::uncommitted_entries`]
/// returns them).
pub(crate) fn count_changes(entries: &[String]) -> Result<Changes, InfoError> {
    let mut changes = Changes::default();
    for entry in entries {
        let mut chars = entry.chars();
        let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
            return Err(parse_error("git status entry", entry));
        };
        match (x, y) {
            ('?', '?') => changes.untracked += 1,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => changes.conflicted += 1,
            _ => {
                if x != ' ' {
                    changes.staged += 1;
                }
                if y != ' ' {
                    changes.unstaged += 1;
                }
            }
        }
    }
    Ok(changes)
}

#[derive(Deserialize)]
struct GhPr {
    number: u32,
    state: String,
    url: String,
}

/// Parses `gh pr list --json number,state,url`: an open PR if there is one, else the first
/// (gh lists the newest first).
pub(crate) fn parse_prs(output: &str) -> Result<Option<PullRequest>, InfoError> {
    let prs: Vec<GhPr> =
        serde_json::from_str(output).map_err(|_| parse_error("gh pr list output", output))?;
    let mut found = Vec::with_capacity(prs.len());
    for pr in prs {
        let state = match pr.state.as_str() {
            "OPEN" => PrState::Open,
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ => return Err(parse_error("PR state", &pr.state)),
        };
        found.push(PullRequest {
            number: pr.number,
            state,
            url: pr.url,
        });
    }
    let open = found.iter().position(|p| p.state == PrState::Open);
    Ok(match open {
        Some(i) => Some(found.swap_remove(i)),
        None => found.into_iter().next(),
    })
}

/// One slice worktree to look at.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    /// The slice.
    pub agent: AgentId,
    /// The worktree's name (the slice's name).
    pub name: String,
    /// The project key it was made under.
    pub key: String,
    /// The worktree directory.
    pub path: PathBuf,
    /// Its branch.
    pub branch: String,
}

/// How much a refresh looks up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// The worktree view asked: fetch the base from `origin` and look up PRs with `gh`.
    Full,
    /// After an event: local `git` only; the PR stays as last looked up.
    Local,
}

/// The base branch of one refresh.
#[derive(Debug, Clone)]
pub(crate) struct Base {
    /// Its name, or `HEAD` when the main checkout is detached.
    pub name: String,
    /// Whether `origin/<name>` can be trusted: false when a fetch was tried and failed.
    pub origin_known: bool,
}

/// One worktree's gathered state, before the core merges it with what it held.
#[derive(Debug, Clone)]
pub(crate) struct Gathered {
    /// The state. Without `pr_checked` its `pr` is a placeholder.
    pub info: WorktreeInfo,
    /// Whether `gh` was asked this time.
    pub pr_checked: bool,
}

/// What the core should hold after `new` arrives, given the `old` entry: `None` when `old` is
/// newer. A refresh that did not ask `gh` keeps the PR last looked up.
pub(crate) fn merge(old: Option<&WorktreeInfo>, new: Gathered) -> Option<WorktreeInfo> {
    let mut info = new.info;
    match old {
        Some(old) if old.checked_ms > info.checked_ms => return None,
        Some(old) if !new.pr_checked => info.pr = old.pr.clone(),
        _ => {}
    }
    Some(info)
}

/// Where to look, shared by every target of one refresh.
#[derive(Debug, Clone)]
pub(crate) struct Context {
    /// Dovetail's paths, for setup state and our symlinks.
    pub paths: Paths,
    /// The main checkout.
    pub repo: PathBuf,
    /// The `gh` program (a path in tests).
    pub gh: PathBuf,
    /// The user's own `GIT_SSH_COMMAND`, if set: the fetch's ssh command builds on it.
    pub ssh_command: Option<String>,
}

/// Environment for `git fetch`: ssh must fail rather than prompt on the terminal the TUI owns.
/// `user` is the ssh command the user chose (`GIT_SSH_COMMAND`, else `core.sshCommand`);
/// `BatchMode` is added to it, or to plain `ssh`.
pub(crate) fn fetch_env(user: Option<&str>) -> [(&'static str, String); 2] {
    let ssh = match user.map(str::trim).filter(|c| !c.is_empty()) {
        Some(command) => format!("{command} -o BatchMode=yes"),
        None => "ssh -o BatchMode=yes".to_owned(),
    };
    [
        ("GIT_SSH_COMMAND", ssh),
        ("SSH_ASKPASS_REQUIRE", "never".to_owned()),
    ]
}

impl Context {
    /// The ssh command the user chose for git: `GIT_SSH_COMMAND`, else `core.sshCommand`.
    async fn user_ssh_command(&self) -> Option<String> {
        if self.ssh_command.is_some() {
            return self.ssh_command.clone();
        }
        // Exit status 1 means unset.
        git(&self.repo, &["config", "--get", "core.sshCommand"])
            .await
            .ok()
            .map(|c| c.trim().to_owned())
    }

    /// Fetches `base` from `origin`, never prompting for credentials.
    async fn fetch(&self, base: &str) -> Result<String, InfoError> {
        let env = fetch_env(self.user_ssh_command().await.as_deref());
        let args = ["fetch", "--quiet", "origin", base];
        run_env(&self.repo, Path::new("git"), &args, &env, NETWORK_TIMEOUT).await
    }

    /// The base branch: the one checked out in the main checkout, or `HEAD` when it is detached
    /// or unknown. A [`Refresh::Full`] first fetches it from `origin`.
    pub(crate) async fn base(&self, refresh: Refresh) -> Base {
        let name = match git(&self.repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await {
            Ok(name) if !name.trim().is_empty() => name.trim().to_owned(),
            Ok(_) => "HEAD".to_owned(),
            Err(e) => {
                tracing::debug!("worktree view: main checkout's branch: {e}");
                "HEAD".to_owned()
            }
        };
        let origin_known = match refresh {
            Refresh::Local => true,
            Refresh::Full if name == "HEAD" => false,
            Refresh::Full => self
                .fetch(&name)
                .await
                .inspect_err(|e| tracing::warn!("worktree view: fetching {name}: {e}"))
                .is_ok(),
        };
        Base { name, origin_known }
    }

    /// Everything the view shows for `target`, compared with `base`. `gh` is asked only on a
    /// [`Refresh::Full`].
    pub(crate) async fn gather(
        &self,
        target: Target,
        base: &Base,
        now_ms: i64,
        refresh: Refresh,
    ) -> Gathered {
        let pr_checked = refresh == Refresh::Full;
        let pr = async {
            if pr_checked {
                self.pr(&target.branch).await
            } else {
                PrLookup::Unknown {
                    reason: "not looked up yet".to_owned(),
                }
            }
        };
        let (sync, changes, unpushed, pr) = tokio::join!(
            self.sync(&target.branch, base),
            self.changes(&target),
            self.unpushed(&target.branch),
            pr,
        );
        let label = |part: &str| format!("worktree view: {} {part}", target.name);
        let info = WorktreeInfo {
            sync: sync
                .inspect_err(|e| tracing::warn!("{}: {e}", label("sync")))
                .ok(),
            changes: changes
                .inspect_err(|e| tracing::warn!("{}: {e}", label("changes")))
                .ok(),
            unpushed: unpushed
                .inspect_err(|e| tracing::warn!("{}: {e}", label("unpushed")))
                .ok(),
            pr,
            setup: setup(&self.paths, &target.key, &target.name),
            base: base.name.clone(),
            agent: target.agent,
            path: target.path,
            branch: target.branch,
            checked_ms: now_ms,
        };
        Gathered { info, pr_checked }
    }

    async fn sync(&self, branch: &str, base: &Base) -> Result<SyncState, InfoError> {
        let (known, base) = (base.origin_known, base.name.as_str());
        let range = format!("{base}...{branch}");
        let out = git(&self.repo, &["rev-list", "--left-right", "--count", &range]).await?;
        let (behind, ahead) = parse_left_right(&out)?;
        let origin = format!("refs/remotes/origin/{base}");
        let behind_origin = if known
            && base != "HEAD"
            && git_check(&self.repo, &["rev-parse", "--verify", "--quiet", &origin]).await?
        {
            let range = format!("{branch}..{origin}");
            Some(parse_count(
                &git(&self.repo, &["rev-list", "--count", &range]).await?,
            )?)
        } else {
            None
        };
        Ok(SyncState {
            ahead,
            behind,
            behind_origin,
        })
    }

    async fn changes(&self, target: &Target) -> Result<Changes, InfoError> {
        // Checked first: git would fail to start in a missing directory, which reads as
        // "git is not installed".
        if !target.path.join(".git").exists() {
            return Err(InfoError::NoWorktree(target.path.clone()));
        }
        let status = git(&target.path, &worktree::STATUS_ARGS).await?;
        let ours = worktree::our_links(&self.paths, &target.key, &target.name);
        count_changes(&worktree::uncommitted_entries(&status, &ours))
    }

    async fn unpushed(&self, branch: &str) -> Result<u32, InfoError> {
        let has_upstream = git_check(&self.repo, &worktree::upstream_check_args(branch)).await?;
        parse_count(&git(&self.repo, &worktree::unpushed_args(branch, has_upstream)).await?)
    }

    async fn pr(&self, branch: &str) -> PrLookup {
        let args = [
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "all",
            "--json",
            "number,state,url",
            "--limit",
            PR_LIMIT,
        ];
        let found = run(&self.repo, &self.gh, &args, NETWORK_TIMEOUT)
            .await
            .and_then(|out| parse_prs(&out));
        match found {
            Ok(Some(pr)) => PrLookup::Found(pr),
            Ok(None) => PrLookup::None,
            Err(e) => {
                tracing::warn!("worktree view: PR of {branch}: {e}");
                PrLookup::Unknown {
                    reason: match e {
                        InfoError::Missing(_) => "gh is not installed".to_owned(),
                        InfoError::Timeout { .. } => "gh timed out".to_owned(),
                        _ => "gh failed".to_owned(),
                    },
                }
            }
        }
    }
}

impl Core {
    /// Gathers every slice worktree's state afresh, with a fetch and PR lookups
    /// ([`Command::RefreshWorktrees`]).
    ///
    /// [`Command::RefreshWorktrees`]: dovetail_api::Command::RefreshWorktrees
    pub(crate) fn refresh_worktrees(&mut self) {
        let slices: Vec<AgentId> = self
            .agents
            .values()
            .filter(|r| r.agent.kind == AgentKind::Slice && r.agent.worktree.is_some())
            .map(|r| r.agent.id.clone())
            .collect();
        self.gather_worktrees(&slices, Refresh::Full);
    }

    /// Gathers one slice's local git state afresh after an event; nothing for other agents.
    pub(crate) fn refresh_worktree(&mut self, agent: &AgentId) {
        self.gather_worktrees(std::slice::from_ref(agent), Refresh::Local);
    }

    /// Looks at `agents`' worktrees in the background; each result arrives as
    /// [`Input::WorktreeGathered`].
    fn gather_worktrees(&mut self, agents: &[AgentId], refresh: Refresh) {
        // No repo, no worktrees, no git (research projects, decided 2026-11-11).
        if self.project.is_research() {
            return;
        }
        let targets: Vec<Target> = agents
            .iter()
            .filter_map(|id| self.agents.get(id))
            .filter(|r| r.agent.kind == AgentKind::Slice)
            .filter_map(|r| {
                let wt = r.agent.worktree.as_ref()?;
                Some(Target {
                    agent: r.agent.id.clone(),
                    name: r.agent.name.clone(),
                    key: self.worktree_key(&r.agent),
                    path: wt.path.clone(),
                    branch: wt.branch.clone(),
                })
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let ctx = Context {
            paths: self.paths.clone(),
            repo: self.project.folder(&self.paths),
            gh: PathBuf::from("gh"),
            ssh_command: std::env::var("GIT_SSH_COMMAND").ok(),
        };
        let inputs = self.inputs.clone();
        let now = now_ms();
        tokio::spawn(async move {
            let base = ctx.base(refresh).await;
            let mut set = tokio::task::JoinSet::new();
            for target in targets {
                let (ctx, base, inputs) = (ctx.clone(), base.clone(), inputs.clone());
                set.spawn(async move {
                    let gathered = ctx.gather(target, &base, now, refresh).await;
                    let _ = inputs.send(Input::WorktreeGathered(gathered));
                });
            }
            while set.join_next().await.is_some() {}
        });
    }

    /// Stores and sends a gathered state, unless the slice is gone or a newer one is in.
    pub(crate) fn on_worktree_gathered(&mut self, gathered: Gathered) {
        if !self.agents.contains_key(&gathered.info.agent) {
            return;
        }
        let Some(info) = merge(self.worktrees.get(&gathered.info.agent), gathered) else {
            return;
        };
        let change = self
            .worktrees
            .get(&info.agent)
            .and_then(|old| pr_change(&old.pr, &info.pr));
        self.worktrees.insert(info.agent.clone(), info.clone());
        if let Some(text) = change {
            let agent = info.agent.clone();
            self.activity(ActivityKind::Pr, Some(&agent), text);
        }
        self.emit(Event::WorktreeUpserted(info));
    }
}

/// The Activity line when the PR `gh` reports differs from the last one seen. The first lookup
/// after start is a baseline, not news; a failed lookup says nothing. CI status is not
/// observed (`gh pr list` is asked for number, state and url only).
fn pr_change(old: &PrLookup, new: &PrLookup) -> Option<String> {
    let PrLookup::Found(pr) = new else {
        return None;
    };
    match old {
        PrLookup::Found(was) if was.number == pr.number && was.state == pr.state => None,
        PrLookup::Found(was) if was.number == pr.number => Some(format!(
            "PR #{} {}",
            pr.number,
            match pr.state {
                PrState::Open => "reopened",
                PrState::Merged => "merged",
                PrState::Closed => "closed",
            }
        )),
        PrLookup::Found(_) | PrLookup::None => Some(format!("PR #{} opened", pr.number)),
        PrLookup::Unknown { .. } => None,
    }
}

/// The setup state the `WorktreeCreate` hook recorded. A run whose process is gone was
/// interrupted.
fn setup(paths: &Paths, key: &str, name: &str) -> WorktreeSetup {
    match worktree::setup_state(paths, key, name) {
        None => WorktreeSetup::None,
        Some(s) => match s.status {
            SetupStatus::Done => WorktreeSetup::Done,
            SetupStatus::Running if worktree::pid_alive(s.pid) => WorktreeSetup::Running,
            SetupStatus::Running => WorktreeSetup::Failed {
                error: Some("interrupted".to_owned()),
            },
            SetupStatus::Failed => WorktreeSetup::Failed { error: s.error },
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::{Command, Stdio};

    use super::*;

    fn pr(number: u32, state: PrState) -> PrLookup {
        PrLookup::Found(PullRequest {
            number,
            state,
            url: String::new(),
        })
    }

    #[test]
    fn a_pr_that_appears_or_changes_state_is_news_a_steady_one_is_not() {
        let unknown = PrLookup::Unknown {
            reason: "gh timed out".into(),
        };
        assert_eq!(
            pr_change(&PrLookup::None, &pr(7, PrState::Open)).as_deref(),
            Some("PR #7 opened")
        );
        assert_eq!(
            pr_change(&pr(7, PrState::Open), &pr(7, PrState::Merged)).as_deref(),
            Some("PR #7 merged")
        );
        assert_eq!(
            pr_change(&pr(7, PrState::Open), &pr(7, PrState::Open)),
            None
        );
        assert_eq!(pr_change(&unknown, &pr(7, PrState::Open)), None);
        assert_eq!(pr_change(&pr(7, PrState::Open), &unknown), None);
        assert_eq!(pr_change(&pr(7, PrState::Open), &PrLookup::None), None);
    }
    use crate::recipe::Recipe;
    use crate::worktree::{CreateRequest, create};

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

    fn commit(dir: &Path, file: &str) {
        fs::write(dir.join(file), format!("{file}\n")).expect("write");
        run_git(dir, &["add", file]);
        run_git(dir, &["commit", "-q", "-m", file]);
    }

    struct Scratch {
        _dir: tempfile::TempDir,
        ctx: Context,
        target: Target,
    }

    /// A repo on `main` with a bare `origin`, and a slice worktree `s` (with `.env`
    /// symlinked) made by [`create`], as the hook does. `gh` is a program that does not exist.
    fn scratch() -> Scratch {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            data: dir.path().join("data"),
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            runtime: dir.path().join("run"),
        };
        let origin = dir.path().join("origin.git");
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).expect("repo");
        run_git(
            dir.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        run_git(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join(".env"), "SECRET=1\n").expect("write");
        fs::write(repo.join(".gitignore"), ".env\n").expect("write");
        run_git(&repo, &["add", ".gitignore"]);
        commit(&repo, "a.txt");
        run_git(
            &repo,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        run_git(&repo, &["push", "-q", "-u", "origin", "main"]);
        let recipe = Recipe {
            worktree: crate::recipe::WorktreeRecipe {
                symlink: vec![PathBuf::from(".env")],
                setup: Vec::new(),
            },
            ..Recipe::default()
        };
        let made = create(
            &paths,
            &CreateRequest {
                project: "p".into(),
                name: "s".into(),
                cwd: repo.clone(),
                recipe,
            },
        )
        .expect("worktree");
        Scratch {
            ctx: Context {
                paths,
                repo,
                gh: dir.path().join("no-such-gh"),
                ssh_command: None,
            },
            target: Target {
                agent: AgentId("a1".into()),
                name: "s".into(),
                key: "p".into(),
                path: made.path,
                branch: made.branch,
            },
            _dir: dir,
        }
    }

    async fn gathered(s: &Scratch, refresh: Refresh) -> Gathered {
        let base = s.ctx.base(refresh).await;
        s.ctx.gather(s.target.clone(), &base, 1, refresh).await
    }

    /// A full refresh, as when the worktree view opens.
    async fn info(s: &Scratch) -> WorktreeInfo {
        gathered(s, Refresh::Full).await.info
    }

    fn behind_origin(info: &WorktreeInfo) -> Option<u32> {
        info.sync.expect("sync").behind_origin
    }

    /// Someone else pushes a commit to `origin/main`.
    fn push_elsewhere(s: &Scratch) {
        let other = s._dir.path().join("other");
        let origin = s._dir.path().join("origin.git");
        run_git(
            s._dir.path(),
            &["clone", "-q", &origin.to_string_lossy(), "other"],
        );
        commit(&other, "theirs.txt");
        run_git(&other, &["push", "-q", "origin", "main"]);
    }

    #[tokio::test]
    async fn a_full_refresh_fetches_the_base_and_a_local_one_does_not() {
        let s = scratch();
        push_elsewhere(&s);
        let local = gathered(&s, Refresh::Local).await.info;
        // As of the last fetch, origin had not moved.
        assert_eq!(behind_origin(&local), Some(0));
        assert_eq!(behind_origin(&info(&s).await), Some(1));
    }

    #[tokio::test]
    async fn a_failed_fetch_leaves_behind_origin_unknown_and_the_rest_known() {
        let s = scratch();
        let gone = s._dir.path().join("gone.git");
        run_git(
            &s.ctx.repo,
            &["remote", "set-url", "origin", &gone.to_string_lossy()],
        );
        let got = info(&s).await;
        let sync = got.sync.expect("sync");
        assert_eq!((sync.ahead, sync.behind, sync.behind_origin), (0, 0, None));
        assert_eq!(got.changes, Some(Changes::default()));
        // A local refresh reads the last fetched ref and makes no network call.
        let local = gathered(&s, Refresh::Local).await.info;
        assert_eq!(behind_origin(&local), Some(0));
    }

    #[test]
    fn the_fetch_env_adds_batch_mode_to_the_users_ssh_command() {
        let value = |user| {
            let env = fetch_env(user);
            assert_eq!(env[1], ("SSH_ASKPASS_REQUIRE", "never".to_owned()));
            env[0].clone()
        };
        assert_eq!(
            value(None),
            ("GIT_SSH_COMMAND", "ssh -o BatchMode=yes".to_owned())
        );
        assert_eq!(
            value(Some("ssh -i ~/.ssh/work")),
            (
                "GIT_SSH_COMMAND",
                "ssh -i ~/.ssh/work -o BatchMode=yes".to_owned()
            )
        );
    }

    /// A fake ssh that records its arguments and `SSH_ASKPASS_REQUIRE`, then fails.
    fn fake_ssh(s: &Scratch) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let script = s._dir.path().join("fake-ssh");
        let log = s._dir.path().join("ssh.log");
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" \"askpass=$SSH_ASKPASS_REQUIRE\" > '{}'\nexit 1\n",
            log.display()
        );
        fs::write(&script, body).expect("write");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        run_git(
            &s.ctx.repo,
            &["remote", "set-url", "origin", "ssh://example.invalid/r.git"],
        );
        (script, log)
    }

    fn assert_batch_mode(log: &Path) {
        let seen = fs::read_to_string(log).expect("ssh ran");
        assert!(seen.contains("BatchMode=yes"), "{seen}");
        assert!(seen.contains("askpass=never"), "{seen}");
    }

    #[tokio::test]
    async fn the_fetch_runs_the_users_ssh_command_in_batch_mode() {
        let mut s = scratch();
        let (script, log) = fake_ssh(&s);
        s.ctx.ssh_command = Some(script.to_string_lossy().into_owned());
        // ssh failed, so the fetch did: origin is unknown, the rest known.
        let got = info(&s).await;
        assert_eq!(behind_origin(&got), None);
        assert_batch_mode(&log);
    }

    #[tokio::test]
    async fn the_fetch_keeps_a_configured_ssh_command_in_batch_mode() {
        let s = scratch();
        let (script, log) = fake_ssh(&s);
        run_git(
            &s.ctx.repo,
            &["config", "core.sshCommand", &script.to_string_lossy()],
        );
        assert_eq!(behind_origin(&info(&s).await), None);
        assert_batch_mode(&log);
    }

    #[tokio::test]
    async fn a_local_refresh_does_not_ask_gh_and_keeps_the_last_pr() {
        let mut s = scratch();
        // `gh` would fail if it were asked.
        s.ctx.gh = PathBuf::from("false");
        let local = gathered(&s, Refresh::Local).await;
        assert!(!local.pr_checked);
        let found = PrLookup::Found(PullRequest {
            number: 9,
            state: PrState::Open,
            url: "u".into(),
        });
        let old = WorktreeInfo {
            pr: found.clone(),
            checked_ms: 0,
            ..local.info.clone()
        };
        let kept = merge(Some(&old), local).expect("newer");
        assert_eq!(kept.pr, found);
    }

    #[test]
    fn merging_keeps_the_newest_and_a_full_refresh_replaces_the_pr() {
        let info = |checked_ms: i64, pr: PrLookup| WorktreeInfo {
            agent: AgentId("a".into()),
            path: PathBuf::from("/w"),
            branch: "dovetail/a".into(),
            base: "main".into(),
            sync: None,
            changes: None,
            unpushed: None,
            pr,
            setup: WorktreeSetup::None,
            checked_ms,
        };
        let placeholder = PrLookup::Unknown {
            reason: "not looked up yet".into(),
        };
        let old = info(5, PrLookup::None);
        let stale = Gathered {
            info: info(4, PrLookup::None),
            pr_checked: true,
        };
        assert_eq!(merge(Some(&old), stale), None);
        let first_local = Gathered {
            info: info(6, placeholder.clone()),
            pr_checked: false,
        };
        assert_eq!(merge(None, first_local).map(|i| i.pr), Some(placeholder));
        let full = Gathered {
            info: info(
                6,
                PrLookup::Unknown {
                    reason: "gh failed".into(),
                },
            ),
            pr_checked: true,
        };
        assert_eq!(
            merge(Some(&old), full).map(|i| i.pr),
            Some(PrLookup::Unknown {
                reason: "gh failed".into()
            })
        );
    }

    #[tokio::test]
    async fn a_fresh_worktree_is_clean_and_level_with_main() {
        let s = scratch();
        let got = info(&s).await;
        assert_eq!(got.base, "main");
        assert_eq!(
            got.sync,
            Some(SyncState {
                ahead: 0,
                behind: 0,
                behind_origin: Some(0)
            })
        );
        // The `.env` symlink is ours, not a change.
        assert_eq!(got.changes, Some(Changes::default()));
        assert_eq!(got.unpushed, Some(0));
        assert_eq!(got.setup, WorktreeSetup::None);
    }

    #[tokio::test]
    async fn changes_are_counted_by_kind() {
        let s = scratch();
        let wt = &s.target.path;
        commit(wt, "tracked.txt");
        fs::write(wt.join("tracked.txt"), "changed\n").expect("write");
        fs::write(wt.join("staged.txt"), "new\n").expect("write");
        run_git(wt, &["add", "staged.txt"]);
        fs::write(wt.join("loose.txt"), "new\n").expect("write");
        let got = info(&s).await;
        assert_eq!(
            got.changes,
            Some(Changes {
                staged: 1,
                unstaged: 1,
                untracked: 1,
                conflicted: 0
            })
        );
    }

    #[tokio::test]
    async fn ahead_behind_and_unpushed_follow_the_branches() {
        let s = scratch();
        commit(&s.target.path, "b.txt");
        commit(&s.target.path, "c.txt");
        commit(&s.ctx.repo, "m.txt");
        let got = info(&s).await;
        let sync = got.sync.expect("sync");
        assert_eq!((sync.ahead, sync.behind), (2, 1));
        // `main` moved locally but was not pushed: origin has nothing new.
        assert_eq!(sync.behind_origin, Some(0));
        assert_eq!(got.unpushed, Some(2));

        run_git(&s.ctx.repo, &["push", "-q", "origin", "main"]);
        run_git(
            &s.target.path,
            &["push", "-q", "-u", "origin", "dovetail/s"],
        );
        let got = info(&s).await;
        assert_eq!(got.sync.expect("sync").behind_origin, Some(1));
        assert_eq!(got.unpushed, Some(0));

        commit(&s.target.path, "d.txt");
        assert_eq!(info(&s).await.unpushed, Some(1));
    }

    #[tokio::test]
    async fn a_missing_gh_leaves_the_pr_unknown_and_the_rest_known() {
        let s = scratch();
        let got = info(&s).await;
        assert_eq!(
            got.pr,
            PrLookup::Unknown {
                reason: "gh is not installed".into()
            }
        );
        assert!(got.sync.is_some() && got.changes.is_some());
    }

    #[tokio::test]
    async fn a_failing_gh_leaves_the_pr_unknown() {
        let mut s = scratch();
        s.ctx.gh = PathBuf::from("false");
        assert_eq!(
            info(&s).await.pr,
            PrLookup::Unknown {
                reason: "gh failed".into()
            }
        );
    }

    #[tokio::test]
    async fn a_missing_worktree_is_unknown_not_clean() {
        let s = scratch();
        fs::remove_dir_all(&s.target.path).expect("remove");
        let got = info(&s).await;
        assert_eq!(got.changes, None);
        // The branch is still there, so its counts are still known.
        assert!(got.sync.is_some());
    }

    #[tokio::test]
    async fn an_overrunning_command_times_out() {
        let dir = tempfile::tempdir().expect("tempdir");
        let found = run(
            dir.path(),
            Path::new("sleep"),
            &["5"],
            Duration::from_millis(50),
        )
        .await;
        assert!(matches!(found, Err(InfoError::Timeout { .. })));
    }

    #[test]
    fn pr_output_prefers_an_open_pr() {
        let out =
            r#"[{"number":3,"state":"CLOSED","url":"u3"},{"number":4,"state":"OPEN","url":"u4"}]"#;
        let pr = parse_prs(out).expect("parse").expect("a PR");
        assert_eq!((pr.number, pr.state), (4, PrState::Open));
        let pr = parse_prs(r#"[{"number":2,"state":"MERGED","url":"u"}]"#)
            .expect("parse")
            .expect("a PR");
        assert_eq!(pr.state, PrState::Merged);
        assert_eq!(parse_prs("[]").expect("parse"), None);
        assert!(parse_prs("not json").is_err());
        assert!(parse_prs(r#"[{"number":1,"state":"WEIRD","url":"u"}]"#).is_err());
    }

    #[test]
    fn counts_reject_garbage() {
        assert_eq!(parse_left_right("3\t5\n").expect("parse"), (3, 5));
        assert!(parse_left_right("3\n").is_err());
        assert!(parse_left_right("a\tb").is_err());
        assert!(count_changes(&["?".to_owned()]).is_err());
        let conflicted = count_changes(&["UU f".to_owned(), "AA g".to_owned()]).expect("parse");
        assert_eq!(conflicted.conflicted, 2);
    }
}

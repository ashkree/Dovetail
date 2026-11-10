//! Memory caps and memory display (M13a, spike S4).
//!
//! Every session runs in its own `systemd-run --user --scope` with a hard `MemoryMax` from
//! `config.toml` (B8). A probe at startup checks that capped scopes start and that the memory
//! controller reaches them; without that, sessions run uncapped and the status bar says so.
//! Memory is read from each scope's `memory.current` on [`SAMPLE_INTERVAL`] while sessions run;
//! when caps are unavailable, from `/proc` instead (the resident memory of each session's process
//! and its descendants, decision #47). OOM kills are counted from `memory.events` (M13b, and
//! `crate::oom` for what happens then).

use std::path::{Path, PathBuf};
use std::time::Duration;

use dovetail_driver::SystemdScope;

/// How often running sessions' memory is read. Nothing is read while no session runs.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Where cgroup v2 is mounted.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
/// The probe gives up after this long (S4: a scope starts in under 10 ms).
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Whether sessions can run under capped scopes, as the startup probe found.
#[derive(Debug, Clone, PartialEq)]
pub enum Scopes {
    /// Capped scopes work. Their cgroups are created in `slice_dir`.
    Available {
        /// The cgroup directory `systemd-run --user --scope` puts scopes in (`app.slice`).
        slice_dir: PathBuf,
        /// `systemd-run` expands `$` in the command's arguments.
        escape_dollars: bool,
    },
    /// Sessions run uncapped; the text says why (shown as a startup warning).
    Unavailable(String),
    /// Not probed (tests): uncapped, and no warning.
    Off,
}

impl Scopes {
    /// The scope to launch a session in, or `None` to launch it uncapped.
    pub fn scope(&self, unit: &str, cap_bytes: u64) -> Option<SystemdScope> {
        match self {
            Self::Available { escape_dollars, .. } => {
                Some(SystemdScope::capped(unit, cap_bytes, *escape_dollars))
            }
            _ => None,
        }
    }

    /// The cgroup directory of a scope started as `unit`.
    pub fn cgroup(&self, unit: &str) -> Option<PathBuf> {
        match self {
            Self::Available { slice_dir, .. } => Some(slice_dir.join(format!("{unit}.scope"))),
            _ => None,
        }
    }
}

/// The scope unit of a session (without `.scope`): `dovetail-<project>-<agent>`. Stable for an
/// agent across restarts, so its cgroup can be found from the agent alone. Project keys are slugs
/// (B9) and slice names pass `worktree::validate_name` (`[A-Za-z0-9._-]`, at most 100), so names
/// are unique and well under systemd's 255; mapping other characters to `_` is only a guard.
pub fn unit_name(project_key: &str, agent_id: &str) -> String {
    format!("dovetail-{project_key}-{agent_id}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The `systemctl` calls that clear a scope left behind by a session that is gone: kill what is
/// left in it, then stop it. A SIGKILLed `claude`, or a crashed Dovetail, can leave tools running
/// in its scope, which keeps the unit alive (`--collect` only removes empty or failed scopes), so
/// relaunching the agent would fail with "unit already exists".
pub fn clear_scope_commands(unit: &str) -> [Vec<String>; 2] {
    let scope = format!("{unit}.scope");
    [
        vec![
            "--user".into(),
            "kill".into(),
            "--signal=SIGKILL".into(),
            scope.clone(),
        ],
        vec!["--user".into(), "stop".into(), scope],
    ]
}

/// Before launching agent `unit` when it has no running session: if its scope still exists,
/// whatever runs in it belongs to a dead session, so it is killed and the scope stopped.
/// Returns whether a scope was cleared. Blocks briefly; only happens after a crash.
pub fn clear_stale_scope(cgroup: &Path, unit: &str) -> bool {
    if !cgroup.exists() {
        return false;
    }
    tracing::info!(unit, "stopping a scope left by a dead session");
    for args in clear_scope_commands(unit) {
        match std::process::Command::new("systemctl")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            // `stop` fails harmlessly once the emptied scope has been collected.
            Ok(status) if !status.success() => {
                tracing::debug!(unit, ?args, %status, "systemctl");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(unit, "running systemctl: {e}"),
        }
    }
    true
}

/// A cap from `config.toml`, in bytes.
pub fn cap_bytes(gb: u32) -> u64 {
    u64::from(gb) * GIB
}

/// Checks that a capped scope starts (with the same properties a session gets) and that the
/// memory controller reaches it, and whether `systemd-run` expands `$` in arguments.
pub async fn probe() -> Scopes {
    match probe_inner().await {
        Ok(scopes) => scopes,
        Err(e) => {
            tracing::warn!("memory caps off: {e}");
            Scopes::Unavailable(e.to_string())
        }
    }
}

/// Why sessions cannot run under capped scopes.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// `systemd-run` could not be started.
    #[error("running systemd-run: {0}")]
    Spawn(std::io::Error),
    /// It did not finish within [`PROBE_TIMEOUT`].
    #[error("systemd-run --user --scope did not finish in {} s", PROBE_TIMEOUT.as_secs())]
    Timeout,
    /// It ran and failed (no user systemd instance, no delegation, unknown property).
    #[error("systemd-run --user --scope failed ({status}): {stderr}")]
    Failed {
        /// Its exit status.
        status: std::process::ExitStatus,
        /// What it printed.
        stderr: String,
    },
    /// The scope is not in a cgroup v2 hierarchy.
    #[error("no cgroup v2 path in the probe scope's /proc/self/cgroup: {0}")]
    NoCgroup(String),
    /// The slice the scope lands in could not be read.
    #[error("reading {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The memory controller does not reach the scope.
    #[error("the memory controller is not enabled in {0}")]
    NoMemoryController(PathBuf),
    /// `$` came back neither escaped nor unchanged.
    #[error("systemd-run passed `a$$b` on as `{0}`")]
    Escaping(String),
}

async fn probe_inner() -> Result<Scopes, ProbeError> {
    // Unique per probe, so probes running at once (tests) don't collide on the unit name.
    static PROBES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = PROBES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let unit = format!("dovetail-probe-{}-{n}", std::process::id());
    let out = run_scoped(&unit, &["cat", "/proc/self/cgroup"]).await?;
    let cgroup = parse_proc_cgroup(&out).ok_or_else(|| ProbeError::NoCgroup(out.clone()))?;
    let scope_dir = Path::new(CGROUP_ROOT).join(cgroup.trim_start_matches('/'));
    let slice_dir = scope_dir
        .parent()
        .ok_or_else(|| ProbeError::NoCgroup(out.clone()))?
        .to_path_buf();
    let control = slice_dir.join("cgroup.subtree_control");
    let controllers = std::fs::read_to_string(&control).map_err(|source| ProbeError::Read {
        path: control,
        source,
    })?;
    if !controllers.split_whitespace().any(|c| c == "memory") {
        return Err(ProbeError::NoMemoryController(slice_dir));
    }
    let echoed = run_scoped(&format!("{unit}-escape"), &["printf", "%s", "a$$b"]).await?;
    let escape_dollars = parse_escape(&echoed).ok_or(ProbeError::Escaping(echoed))?;
    Ok(Scopes::Available {
        slice_dir,
        escape_dollars,
    })
}

/// Runs `command` in a short capped scope and returns its stdout.
async fn run_scoped(unit: &str, command: &[&str]) -> Result<String, ProbeError> {
    let scope = SystemdScope::capped(unit, 64 * MIB, false);
    let run = tokio::process::Command::new("systemd-run")
        .args(scope.args())
        .args(command)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(PROBE_TIMEOUT, run).await {
        Err(_) => return Err(ProbeError::Timeout),
        Ok(Err(e)) => return Err(ProbeError::Spawn(e)),
        Ok(Ok(out)) => out,
    };
    if !out.status.success() {
        return Err(ProbeError::Failed {
            status: out.status,
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The cgroup v2 path in `/proc/<pid>/cgroup` text (the `0::` line).
pub fn parse_proc_cgroup(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(str::trim)
        .filter(|p| p.starts_with('/'))
}

/// Whether `systemd-run` escapes `$`, from what it passed on for `a$$b`.
pub fn parse_escape(echoed: &str) -> Option<bool> {
    match echoed.trim_end() {
        "a$b" => Some(true),
        "a$$b" => Some(false),
        _ => None,
    }
}

/// A scope's `memory.current` in bytes; `None` while it cannot be read (not created yet, or
/// gone).
pub fn read_current(cgroup: &Path) -> Option<u64> {
    std::fs::read_to_string(cgroup.join("memory.current"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Memory as the UI shows it, in MB: steps of 10 MB under 1 GiB and of 0.1 GiB above, so a
/// session's figure changes (and the UI redraws) only when the shown value would.
pub fn shown_mb(bytes: u64) -> u32 {
    let mb = if bytes < GIB {
        (bytes + 5 * MIB) / (10 * MIB) * 10
    } else {
        let tenths = (bytes * 10 + GIB / 2) / GIB;
        (tenths * 1024 + 5) / 10
    };
    u32::try_from(mb).unwrap_or(u32::MAX)
}

// --- OOM kills (M13b) ---

/// How long after an `oom_kill` rise the session process may still die of it. Dying by SIGKILL
/// within this window means the kernel killed the session itself; still running after it means
/// only one of its tools was killed (scopes run with `OOMPolicy=continue`).
pub const OOM_GRACE: Duration = Duration::from_secs(3);

/// After an automatic restart, an agent killed again for memory within this window is not
/// restarted again: the user is asked instead, so a session that keeps hitting its cap does not
/// loop.
pub const RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);

/// The signal the kernel's OOM killer sends.
pub const SIGKILL: i32 = 9;

/// The `oom_kill` count in a scope's `memory.events` text.
pub fn parse_oom_kills(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let (key, value) = l.split_once(' ')?;
        if key == "oom_kill" {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

/// A scope's `memory.events` file.
pub fn events_file(cgroup: &Path) -> PathBuf {
    cgroup.join("memory.events")
}

/// A scope's `oom_kill` count; `None` while the file cannot be read (gone with the scope).
pub fn read_oom_kills(cgroup: &Path) -> Option<u64> {
    parse_oom_kills(&std::fs::read_to_string(events_file(cgroup)).ok()?)
}

/// What an OOM kill in a session's scope killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OomKill {
    /// The session process itself: it is restarted.
    Session,
    /// One of its tools; the session runs on.
    Tool,
}

/// What the `oom_kill` rise seen at `seen` killed, as known at `now`. `exit` is the session
/// process's exit signal (`Some(None)`: it exited without one) once it has exited. `None` while
/// it cannot be told yet.
pub fn oom_verdict(
    seen: std::time::Instant,
    now: std::time::Instant,
    exit: Option<Option<i32>>,
) -> Option<OomKill> {
    let since = now.saturating_duration_since(seen);
    match exit {
        Some(Some(SIGKILL)) if since <= OOM_GRACE => Some(OomKill::Session),
        // It died some other way, or long after: the kill hit a tool.
        Some(_) => Some(OomKill::Tool),
        None if since >= OOM_GRACE => Some(OomKill::Tool),
        None => None,
    }
}

/// Automatic restarts after OOM kills, per agent: at most one per [`RESTART_WINDOW`].
#[derive(Debug, Default)]
pub struct RestartGuard {
    last: std::collections::HashMap<dovetail_api::AgentId, std::time::Instant>,
}

impl RestartGuard {
    /// Whether `agent` may be restarted automatically at `now`.
    pub fn allowed(&self, agent: &dovetail_api::AgentId, now: std::time::Instant) -> bool {
        self.last
            .get(agent)
            .is_none_or(|at| now.saturating_duration_since(*at) >= RESTART_WINDOW)
    }

    /// Counts an automatic restart of `agent` at `now`. Only restarts that happened count.
    pub fn record(&mut self, agent: &dovetail_api::AgentId, now: std::time::Instant) {
        self.last.insert(agent.clone(), now);
    }
}

// --- /proc fallback (decision #47) ---

/// Where the proc filesystem is mounted.
const PROC_ROOT: &str = "/proc";

/// `(pid, ppid)` from a `/proc/<pid>/stat` line. The command name sits in parentheses and may
/// itself hold spaces and parentheses, so fields are counted from the last `)`.
pub fn parse_stat(text: &str) -> Option<(u32, u32)> {
    let (head, rest) = text.rsplit_once(')')?;
    let pid = head.split_once(" (")?.0.trim().parse().ok()?;
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    Some((pid, ppid))
}

/// `VmRSS` from `/proc/<pid>/status` text, in bytes. Kernel threads have none.
pub fn parse_vm_rss(text: &str) -> Option<u64> {
    let line = text.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let kb: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kb * 1024)
}

/// `root` and every process descended from it, given each process's `(pid, ppid)`.
pub fn tree_of(root: u32, parents: &[(u32, u32)]) -> Vec<u32> {
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let pid = tree[i];
        tree.extend(
            parents
                .iter()
                .filter(|(child, ppid)| *ppid == pid && *child != pid)
                .map(|(child, _)| *child),
        );
        i += 1;
    }
    tree
}

/// Every process's `(pid, ppid)`, read once per sample for all sessions.
pub fn read_parents() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir(PROC_ROOT) else {
        return Vec::new();
    };
    dir.filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|e| parse_stat(&std::fs::read_to_string(e.path().join("stat")).ok()?))
        .collect()
}

/// The resident memory of `root` and its descendants in bytes, or `None` once `root` is gone.
/// For sessions that run uncapped and so have no scope to read.
pub fn read_tree_rss(root: u32, parents: &[(u32, u32)]) -> Option<u64> {
    let rss = |pid: u32| {
        std::fs::read_to_string(Path::new(PROC_ROOT).join(pid.to_string()).join("status"))
            .ok()
            .map(|t| parse_vm_rss(&t).unwrap_or(0))
    };
    let own = rss(root)?;
    let rest: u64 = tree_of(root, parents)
        .into_iter()
        .skip(1)
        .filter_map(rss)
        .sum();
    Some(own + rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_are_stable_and_safe() {
        assert_eq!(
            unit_name("dovetail", "slice-m13 caps"),
            "dovetail-dovetail-slice-m13_caps"
        );
        assert_eq!(unit_name("p", "coordinator"), "dovetail-p-coordinator");
    }

    #[test]
    fn a_stale_scope_is_killed_then_stopped() {
        let [kill, stop] = clear_scope_commands("dovetail-p-coordinator");
        assert_eq!(
            kill,
            [
                "--user",
                "kill",
                "--signal=SIGKILL",
                "dovetail-p-coordinator.scope"
            ]
        );
        assert_eq!(stop, ["--user", "stop", "dovetail-p-coordinator.scope"]);
        // No cgroup, nothing to clear (and no systemctl call).
        let dir = tempfile::tempdir().unwrap();
        assert!(!clear_stale_scope(&dir.path().join("x.scope"), "x"));
    }

    #[test]
    fn caps_come_from_gigabytes() {
        assert_eq!(cap_bytes(4), 4_294_967_296);
        assert_eq!(cap_bytes(3), 3 * GIB);
    }

    #[test]
    fn only_available_scopes_cap() {
        let on = Scopes::Available {
            slice_dir: PathBuf::from("/sys/fs/cgroup/u/app.slice"),
            escape_dollars: true,
        };
        let scope = on.scope("dovetail-p-a", 3 * GIB).unwrap();
        assert_eq!(scope.unit, "dovetail-p-a");
        assert!(scope.escape_dollars);
        assert!(
            scope
                .properties
                .contains(&"MemoryMax=3221225472".to_string())
        );
        assert_eq!(
            on.cgroup("dovetail-p-a").unwrap(),
            Path::new("/sys/fs/cgroup/u/app.slice/dovetail-p-a.scope")
        );
        let off = Scopes::Unavailable("no systemd".into());
        assert!(off.scope("u", 1).is_none() && off.cgroup("u").is_none());
        assert!(Scopes::Off.scope("u", 1).is_none());
    }

    #[test]
    fn proc_cgroup_gives_the_v2_path() {
        let text = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope\n";
        assert_eq!(
            parse_proc_cgroup(text),
            Some("/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope")
        );
        assert_eq!(parse_proc_cgroup("1:name=systemd:/x\n"), None);
        assert_eq!(parse_proc_cgroup(""), None);
    }

    #[test]
    fn dollar_escaping_is_read_from_the_echo() {
        assert_eq!(parse_escape("a$b"), Some(true));
        assert_eq!(parse_escape("a$$b\n"), Some(false));
        assert_eq!(parse_escape("ab"), None);
    }

    #[test]
    fn memory_is_rounded_to_what_the_ui_shows() {
        assert_eq!(shown_mb(0), 0);
        assert_eq!(shown_mb(412 * MIB), 410);
        assert_eq!(shown_mb(416 * MIB), 420);
        // 2.1 GiB and anything that rounds to it give the same figure.
        assert_eq!(shown_mb(2150 * MIB), 2150);
        assert_eq!(shown_mb(2170 * MIB), 2150);
        assert_eq!(shown_mb(GIB), 1024);
    }

    /// On a machine with user systemd: the probe passes and a capped scope's memory is read
    /// from the cgroup path derived from its unit. Opt-in, like the live tests.
    #[tokio::test]
    async fn probe_and_read_a_real_scope() {
        if std::env::var("DOVETAIL_CGROUP_TESTS").as_deref() != Ok("1") {
            return;
        }
        let scopes = probe().await;
        let Scopes::Available { .. } = &scopes else {
            panic!("probe failed: {scopes:?}");
        };
        let unit = unit_name("test", &format!("probe-{}", std::process::id()));
        let scope = scopes.scope(&unit, 64 * MIB).unwrap();
        let mut child = tokio::process::Command::new("systemd-run")
            .args(scope.args())
            .args(["sleep", "30"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let cgroup = scopes.cgroup(&unit).unwrap();
        let mut current = None;
        for _ in 0..50 {
            current = read_current(&cgroup);
            if current.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let max = std::fs::read_to_string(cgroup.join("memory.max")).unwrap();
        assert_eq!(max.trim(), (64 * MIB).to_string());
        assert!(current.is_some_and(|b| b > 0));
        // A scope still in use is cleared before a relaunch, freeing its name.
        assert!(clear_stale_scope(&cgroup, &unit));
        assert!(!child.wait().await.unwrap().success());
        let mut gone = false;
        for _ in 0..50 {
            gone = !cgroup.exists();
            if gone {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(gone, "the scope's cgroup is still there");
    }

    /// `memory.events` of a scope after one OOM kill, as kernel 6.x writes it.
    const EVENTS: &str = "low 0\nhigh 0\nmax 213\noom 1\noom_kill 1\noom_group_kill 0\n";

    #[test]
    fn oom_kills_are_read_from_memory_events() {
        assert_eq!(parse_oom_kills(EVENTS), Some(1));
        assert_eq!(
            parse_oom_kills("low 0\nhigh 0\nmax 0\noom 0\noom_kill 0\n"),
            Some(0)
        );
        // `oom_group_kill` is another counter, not this one.
        assert_eq!(parse_oom_kills("oom_group_kill 3\n"), None);
        assert_eq!(parse_oom_kills(""), None);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_oom_kills(dir.path()), None);
        std::fs::write(events_file(dir.path()), EVENTS).unwrap();
        assert_eq!(read_oom_kills(dir.path()), Some(1));
    }

    #[test]
    fn a_kill_the_session_dies_of_at_once_is_the_sessions() {
        let seen = std::time::Instant::now();
        let soon = seen + Duration::from_millis(200);
        let late = seen + OOM_GRACE + Duration::from_secs(1);
        // Dead by SIGKILL within the grace: the session itself.
        assert_eq!(
            oom_verdict(seen, soon, Some(Some(SIGKILL))),
            Some(OomKill::Session)
        );
        // Still running after the grace: a tool.
        assert_eq!(oom_verdict(seen, late, None), Some(OomKill::Tool));
        // Too early to tell.
        assert_eq!(oom_verdict(seen, soon, None), None);
        // Dead another way, or by SIGKILL long after: not this kill.
        assert_eq!(oom_verdict(seen, soon, Some(None)), Some(OomKill::Tool));
        assert_eq!(oom_verdict(seen, soon, Some(Some(15))), Some(OomKill::Tool));
        assert_eq!(
            oom_verdict(seen, late, Some(Some(SIGKILL))),
            Some(OomKill::Tool)
        );
    }

    #[test]
    fn an_agent_is_restarted_automatically_at_most_once_per_window() {
        let mut guard = RestartGuard::default();
        let a = dovetail_api::AgentId("a".into());
        let b = dovetail_api::AgentId("b".into());
        let t = std::time::Instant::now();
        // A check alone (a restart that then failed) uses nothing up.
        assert!(guard.allowed(&a, t));
        assert!(guard.allowed(&a, t));
        guard.record(&a, t);
        assert!(!guard.allowed(&a, t + Duration::from_secs(60)));
        // Another agent has its own allowance.
        assert!(guard.allowed(&b, t + Duration::from_secs(60)));
        assert!(!guard.allowed(&a, t + RESTART_WINDOW - Duration::from_secs(1)));
        assert!(guard.allowed(&a, t + RESTART_WINDOW));
    }

    #[test]
    fn stat_lines_give_pid_and_parent_even_with_odd_names() {
        let stat = "4242 (claude) S 4100 4242 4100 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 11 0 1 1 1";
        assert_eq!(parse_stat(stat), Some((4242, 4100)));
        // A command name with spaces and parentheses.
        let odd = "77 (tmux: server (1)) R 1 77 77 0 -1 0 0 0 0 0";
        assert_eq!(parse_stat(odd), Some((77, 1)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn rss_comes_from_status_in_bytes() {
        let status = "Name:\tclaude\nVmPeak:\t 999 kB\nVmRSS:\t  215040 kB\nRssAnon:\t 1 kB\n";
        assert_eq!(parse_vm_rss(status), Some(215_040 * 1024));
        // Kernel threads have no VmRSS line.
        assert_eq!(parse_vm_rss("Name:\tkthreadd\n"), None);
    }

    #[test]
    fn a_session_tree_holds_its_descendants_only() {
        // 10 is claude; 11 its MCP bridge; 12 a shell running cargo (13), whose rustc is 14.
        // 20 and 21 belong to something else; 1 is init.
        let parents = [
            (1, 0),
            (10, 1),
            (11, 10),
            (12, 10),
            (13, 12),
            (14, 13),
            (20, 1),
            (21, 20),
        ];
        let mut tree = tree_of(10, &parents);
        tree.sort_unstable();
        assert_eq!(tree, [10, 11, 12, 13, 14]);
        assert_eq!(tree_of(99, &parents), [99]);
    }

    /// This test's own process tree is readable from `/proc`.
    #[test]
    fn the_proc_walk_reads_a_live_tree() {
        let me = std::process::id();
        let parents = read_parents();
        assert!(parents.iter().any(|(pid, _)| *pid == me));
        assert!(read_tree_rss(me, &parents).is_some_and(|b| b > 0));
        assert_eq!(read_tree_rss(u32::MAX, &parents), None);
    }

    /// On a machine with user systemd: an OOM in one capped scope kills only that scope's
    /// process, the watch on its `memory.events` sees the kill before the scope is gone, and a
    /// second capped scope runs on. Opt-in, like the live tests.
    #[tokio::test]
    async fn a_forced_oom_kills_one_scope_only() {
        if std::env::var("DOVETAIL_CGROUP_TESTS").as_deref() != Ok("1") {
            return;
        }
        use notify::Watcher;
        use std::os::unix::process::ExitStatusExt;
        let scopes = probe().await;
        let Scopes::Available { .. } = &scopes else {
            panic!("probe failed: {scopes:?}");
        };
        let start = |name: &str, script: &str| {
            let unit = unit_name("test", &format!("{name}-{}", std::process::id()));
            let scope = scopes.scope(&unit, 64 * MIB).unwrap();
            let child = tokio::process::Command::new("systemd-run")
                .args(scope.args())
                .args(["sh", "-c", script])
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            (unit.clone(), scopes.cgroup(&unit).unwrap(), child)
        };
        let wait_for = |cgroup: PathBuf| async move {
            for _ in 0..100 {
                if events_file(&cgroup).exists() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no scope at {}", cgroup.display());
        };
        let (survivor, survivor_cgroup, mut survivor_child) = start("survivor", "sleep 30");
        // The eater replaces the shell after a pause, so the process the scope started is the
        // one killed, as when the kernel kills a session's `claude`.
        let (_, eater_cgroup, mut eater_child) = start("eater", "sleep 1; exec tail /dev/zero");
        wait_for(survivor_cgroup.clone()).await;
        wait_for(eater_cgroup.clone()).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            for path in event.paths {
                if let Some(count) = path.parent().and_then(read_oom_kills) {
                    let _ = tx.send(count);
                }
            }
        })
        .unwrap();
        watcher
            .watch(
                &events_file(&eater_cgroup),
                notify::RecursiveMode::NonRecursive,
            )
            .unwrap();
        let status = tokio::time::timeout(Duration::from_secs(20), eater_child.wait())
            .await
            .expect("the eater was not killed")
            .unwrap();
        assert_eq!(status.signal(), Some(SIGKILL), "{status:?}");
        // The watcher's thread read the count before the scope went; give its send a moment.
        let mut seen = 0;
        while let Ok(Some(count)) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            seen = seen.max(count);
        }
        assert!(seen >= 1, "the watch saw no oom_kill");
        // The other scope is untouched.
        assert!(survivor_child.try_wait().unwrap().is_none());
        assert_eq!(read_oom_kills(&survivor_cgroup), Some(0));
        assert!(clear_stale_scope(&survivor_cgroup, &survivor));
    }

    #[test]
    fn a_missing_cgroup_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_current(dir.path()), None);
        std::fs::write(dir.path().join("memory.current"), "123456\n").unwrap();
        assert_eq!(read_current(dir.path()), Some(123_456));
    }
}

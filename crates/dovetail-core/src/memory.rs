//! Memory caps and memory display (M13a, spike S4).
//!
//! Every session runs in its own `systemd-run --user --scope` with a hard `MemoryMax` from
//! `config.toml` (B8). A probe at startup checks that capped scopes start and that the memory
//! controller reaches them; without that, sessions run uncapped and the status bar says so.
//! Memory is read from each scope's `memory.current` on [`SAMPLE_INTERVAL`] while sessions run.

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
/// agent across restarts, so its cgroup can be found from the agent alone. Characters systemd
/// does not allow in unit names become `_`.
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
    let unit = format!("dovetail-probe-{}", std::process::id());
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
            .args(["sleep", "2"])
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
        child.kill().await.unwrap();
    }

    #[test]
    fn a_missing_cgroup_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_current(dir.path()), None);
        std::fs::write(dir.path().join("memory.current"), "123456\n").unwrap();
        assert_eq!(read_current(dir.path()), Some(123_456));
    }
}

//! Conditions found at start that warn instead of failing (M16). Each check is a pure function
//! of what was observed, so tests feed it fakes; the core gathers the observations.

use std::ffi::OsStr;
use std::time::Duration;

use dovetail_api::{Warning, WarningId};

/// The oldest Claude Code with cross-session messaging (digest §6; `scripts/bootstrap.sh`).
pub(crate) const MIN_CLAUDE: [u32; 3] = [2, 1, 224];

/// Environment variables that can silently turn messaging off.
const MESSAGING_BLOCKERS: [&str; 4] = [
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "DISABLE_TELEMETRY",
    "DO_NOT_TRACK",
    "DISABLE_GROWTHBOOK",
];

/// How long `claude --version` may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

fn warning(id: &str, text: String) -> Warning {
    Warning {
        id: WarningId(id.into()),
        text,
    }
}

/// The messaging blockers that are set (non-empty) in the environment `var` reads.
pub(crate) fn messaging(var: impl Fn(&str) -> Option<String>) -> Option<Warning> {
    let set: Vec<&str> = MESSAGING_BLOCKERS
        .into_iter()
        .filter(|v| var(v).is_some_and(|x| !x.is_empty()))
        .collect();
    (!set.is_empty()).then(|| {
        warning(
            "messaging",
            format!(
                "{} set: sessions may not be able to message each other",
                set.join(", ")
            ),
        )
    })
}

/// Role prompts that are not installed (`missing` lists the roles).
pub(crate) fn roles(missing: &[&str]) -> Option<Warning> {
    (!missing.is_empty()).then(|| {
        warning(
            "roles",
            format!(
                "role prompts missing ({}): run scripts/bootstrap.sh",
                missing.join(", ")
            ),
        )
    })
}

/// A registered project without a recipe file: worktrees get the default setup.
pub(crate) fn recipe(project: &str, exists: bool) -> Option<Warning> {
    (!exists).then(|| {
        warning(
            "recipe",
            format!("no recipe for {project}: worktrees get no setup; run scripts/bootstrap.sh"),
        )
    })
}

/// `major.minor.patch` from the first version-looking word of `claude --version` output.
pub(crate) fn parse_version(output: &str) -> Option<[u32; 3]> {
    output.split_whitespace().find_map(|word| {
        let mut parts = word.split('.');
        let mut next = || parts.next()?.parse::<u32>().ok();
        Some([next()?, next()?, next()?])
    })
}

/// What `claude --version` gave: its output, or why it could not run.
pub(crate) fn claude_version(output: Result<String, String>) -> Option<Warning> {
    let min = MIN_CLAUDE.map(|n| n.to_string()).join(".");
    match output {
        Err(why) => Some(warning(
            "claude-version",
            format!("could not run claude --version ({why}): sessions will not start without it"),
        )),
        Ok(out) => match parse_version(&out) {
            None => Some(warning(
                "claude-version",
                format!("claude gave no version ({}): need ≥ {min}", out.trim()),
            )),
            Some(have) if have < MIN_CLAUDE => Some(warning(
                "claude-version",
                format!(
                    "claude {} is older than {min}: sessions cannot message each other",
                    have.map(|n| n.to_string()).join(".")
                ),
            )),
            Some(_) => None,
        },
    }
}

/// Runs `program --version` with a time limit.
pub(crate) async fn run_claude_version(program: &OsStr) -> Result<String, String> {
    let run = tokio::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(VERSION_TIMEOUT, run).await {
        Err(_) => Err("timed out".into()),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(out)) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Ok(Ok(out)) => Err(format!("exit {}", out.status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("2.1.224 (Claude Code)\n"), Some([2, 1, 224]));
        assert_eq!(parse_version("claude 2.2.0"), Some([2, 2, 0]));
        assert_eq!(parse_version("no version"), None);
        assert!(claude_version(Ok("2.1.224 (Claude Code)".into())).is_none());
        assert!(claude_version(Ok("2.10.0".into())).is_none());
        let old = claude_version(Ok("2.1.223 (Claude Code)".into())).unwrap();
        assert!(old.text.contains("older than 2.1.224"), "{}", old.text);
        assert!(claude_version(Ok("hello".into())).is_some());
        assert!(claude_version(Err("not found".into())).is_some());
    }

    #[test]
    fn messaging_blockers_must_be_set_and_non_empty() {
        let env = |set: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                set.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert!(messaging(env(&[])).is_none());
        assert!(messaging(env(&[("DO_NOT_TRACK", "")])).is_none());
        let w = messaging(env(&[("DO_NOT_TRACK", "1"), ("DISABLE_TELEMETRY", "1")])).unwrap();
        assert!(
            w.text.contains("DISABLE_TELEMETRY, DO_NOT_TRACK"),
            "{}",
            w.text
        );
    }

    #[test]
    fn missing_files_each_give_one_warning() {
        assert!(roles(&[]).is_none());
        assert!(roles(&["slice"]).unwrap().text.contains("slice"));
        assert!(recipe("dovetail", true).is_none());
        assert!(recipe("dovetail", false).unwrap().text.contains("dovetail"));
    }
}

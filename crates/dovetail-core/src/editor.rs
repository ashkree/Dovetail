//! Runs the user's editor in the terminal the UI has let go of
//! ([`dovetail_api::terminal::TerminalCommand::OpenEditor`]).
//!
//! The UI leaves the alternate screen and raw mode and stops reading keys before it asks; the
//! core then runs the editor with the terminal as its stdin, stdout and stderr, waits for it on
//! a blocking thread, and answers [`dovetail_api::terminal::TerminalEvent::Returned`] so the UI takes the
//! terminal back. The command line is `$EDITOR` (as git reads it: through `sh`, so it may carry
//! arguments such as `code -w`), else `nvim`.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// The editor used when `$EDITOR` is unset or empty.
pub const FALLBACK_EDITOR: &str = "nvim";

/// Why the editor did not run cleanly. `Display` is the short notice shown in the status bar.
#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    /// The path to open is gone.
    #[error("{0} does not exist")]
    Missing(String),
    /// The shell could not be started.
    #[error("could not start {editor}: {source}")]
    Spawn {
        /// The editor command line.
        editor: String,
        /// Why.
        source: io::Error,
    },
    /// The editor exited unsuccessfully (127 from `sh`: the editor is not installed).
    #[error("{editor} exited with {code}")]
    Exited {
        /// The editor command line.
        editor: String,
        /// Its exit code, or the signal that ended it.
        code: String,
    },
}

/// The editor command line: `configured` (tests, or a later config key), else a non-empty
/// `$EDITOR`, else [`FALLBACK_EDITOR`].
pub fn resolve(configured: Option<&OsStr>) -> OsString {
    configured
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("EDITOR"))
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| FALLBACK_EDITOR.into())
}

/// Runs `editor` on `path` in the current terminal and waits for it to exit. A folder is also
/// the editor's working directory; a file's parent folder is. The child is always waited for.
pub fn run(editor: &OsStr, path: &Path) -> Result<(), EditorError> {
    if !path.exists() {
        return Err(EditorError::Missing(path.display().to_string()));
    }
    let dir = if path.is_dir() {
        Some(path)
    } else {
        path.parent().filter(|p| !p.as_os_str().is_empty())
    };
    let shown = editor.to_string_lossy().into_owned();
    // `sh -c '<editor> "$@"' sh <path>`: the editor line is split as a shell would, and the
    // path is passed as an argument, never spliced into the script.
    let mut script = editor.to_os_string();
    script.push(" \"$@\"");
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let child = command.spawn().map_err(|source| EditorError::Spawn {
        editor: shown.clone(),
        source,
    })?;
    // A Ctrl-Z in the editor must not leave the terminal hanging (`foreground`).
    let status = crate::foreground::wait(child).map_err(|source| EditorError::Spawn {
        editor: shown.clone(),
        source,
    })?;
    if status.success() {
        return Ok(());
    }
    let code = match status.code() {
        Some(127) => "127 (not found)".to_owned(),
        Some(code) => code.to_string(),
        None => status.to_string(),
    };
    Err(EditorError::Exited {
        editor: shown,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_editor_wins() {
        assert_eq!(resolve(Some(OsStr::new("vi -n"))), OsString::from("vi -n"));
    }

    /// Writes a fake editor that records its working directory and its argument in `out`.
    pub(crate) fn fake_editor(dir: &Path, out: &Path) -> OsString {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-editor");
        let body = format!(
            "#!/bin/sh\npwd -P >'{0}'\nprintf '%s' \"$1\" >>'{0}'\n",
            out.display()
        );
        std::fs::write(&script, body).expect("fake editor");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
        script.into_os_string()
    }

    #[test]
    fn the_editor_gets_the_path_and_starts_in_the_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        let wt = dir.path().join("my worktree");
        std::fs::create_dir(&wt).expect("worktree dir");
        let out = dir.path().join("seen");
        run(&fake_editor(dir.path(), &out), &wt).expect("fake editor runs");
        let seen = std::fs::read_to_string(&out).expect("record");
        let real = wt.canonicalize().expect("canonical");
        assert_eq!(seen, format!("{}\n{}", real.display(), wt.display()));
    }

    #[test]
    fn a_failing_editor_is_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = run(OsStr::new("false"), dir.path()).expect_err("false fails");
        assert_eq!(err.to_string(), "false exited with 1");
    }

    #[test]
    fn a_missing_editor_says_not_found() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = run(OsStr::new("no-such-editor-xyz"), dir.path()).expect_err("missing");
        assert!(err.to_string().contains("127 (not found)"), "{err}");
    }

    #[test]
    fn a_missing_path_runs_nothing() {
        let err = run(OsStr::new("true"), Path::new("/no/such/worktree")).expect_err("missing");
        assert!(matches!(err, EditorError::Missing(_)), "{err}");
    }
}

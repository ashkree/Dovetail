//! Opens a web page in the user's browser ([`dovetail_api::Command::OpenExternal`], `gx` in
//! reading mode, ADR 0018). The TUI never spawns processes; the core does, here.
//!
//! The program is `$BROWSER` (its first entry, as the `BROWSER` convention allows a
//! `:`-separated list) if set, else `xdg-open`. The URL is passed as one argument, never
//! through a shell. The browser is detached from the terminal (no stdin, output discarded) and
//! waited for on a blocking thread, so it is always reaped.

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Child, Command, Stdio};

/// The opener used when `$BROWSER` is unset or empty.
pub const FALLBACK_OPENER: &str = "xdg-open";

/// Why a page could not be opened. `Display` is the notice shown in the status bar.
#[derive(Debug, thiserror::Error)]
pub enum ExternalError {
    /// Not an `http` or `https` URL, or one with spaces or control characters.
    #[error("only web pages open in the browser, not {0}")]
    NotWeb(String),
    /// The opener could not be started.
    #[error("could not start {opener}: {source}")]
    Spawn {
        /// The program.
        opener: String,
        /// Why.
        source: io::Error,
    },
}

/// Checks that `url` is a plain `http` or `https` URL.
pub fn check(url: &str) -> Result<(), ExternalError> {
    let lower = url.to_ascii_lowercase();
    let web = lower.starts_with("http://") || lower.starts_with("https://");
    let host = url.split_once("://").map_or("", |(_, rest)| rest);
    let clean = !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if web && clean && !host.is_empty() {
        Ok(())
    } else {
        Err(ExternalError::NotWeb(url.to_string()))
    }
}

/// The opener: `configured` (tests), else the first entry of a non-empty `$BROWSER`, else
/// [`FALLBACK_OPENER`].
pub fn resolve(configured: Option<&OsStr>) -> OsString {
    if let Some(opener) = configured.filter(|o| !o.is_empty()) {
        return opener.to_os_string();
    }
    std::env::var("BROWSER")
        .ok()
        .and_then(|b| b.split(':').next().map(str::trim).map(str::to_string))
        .filter(|b| !b.is_empty())
        .map_or_else(|| FALLBACK_OPENER.into(), OsString::from)
}

/// Starts `opener` on `url`, detached from the terminal. The caller waits for the child.
pub fn spawn(opener: &OsStr, url: &str) -> Result<Child, ExternalError> {
    check(url)?;
    Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| ExternalError::Spawn {
            opener: opener.to_string_lossy().into_owned(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_web_urls_pass() {
        assert!(check("https://docs.rs/nucleo").is_ok());
        assert!(check("HTTP://example.com/a?b=c").is_ok());
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https://a b",
            "https://a\nb",
            "-https://x",
            "/home/you/report.md",
        ] {
            assert!(check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_configured_opener_wins() {
        assert_eq!(resolve(Some(OsStr::new("/bin/true"))), "/bin/true");
    }

    #[test]
    fn spawn_refuses_before_starting_anything() {
        assert!(matches!(
            spawn(OsStr::new("/nonexistent"), "file:///x"),
            Err(ExternalError::NotWeb(_))
        ));
    }
}

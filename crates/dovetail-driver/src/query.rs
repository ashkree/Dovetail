//! A short-lived process asked one question: the usage reports ([`crate::SessionDriver::usage`]).
//! It is sent a few lines, its stdout is read until one line is the answer, and it is killed
//! then (or at the deadline). No session, no events, no model call.

use crate::process;
use serde_json::Value;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// How long a query may take before it is given up.
pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Where and how a usage query runs (task #46).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageQuery {
    /// The working directory (any folder will do; the project's root is natural).
    pub cwd: PathBuf,
    /// Extra environment, e.g. the Claude profile's `CLAUDE_CONFIG_DIR` (ADR 0009).
    pub env: Vec<(String, String)>,
}

/// Why a query got no answer.
#[derive(Debug, thiserror::Error)]
pub(crate) enum QueryError {
    /// The process could not be started.
    #[error(transparent)]
    Spawn(#[from] crate::DriverError),
    /// Writing to it or reading from it failed.
    #[error("talking to it failed: {0}")]
    Io(#[from] std::io::Error),
    /// It ended without an answer.
    #[error("it ended without an answer")]
    NoAnswer,
    /// It did not answer in time.
    #[error("no answer within {0}s")]
    Timeout(u64),
}

/// An RFC 3339 time (as both usage reports give reset times) as Unix milliseconds.
pub(crate) fn rfc3339_ms(v: Option<&Value>) -> Option<i64> {
    let ts: jiff::Timestamp = v?.as_str()?.parse().ok()?;
    Some(ts.as_millisecond())
}

/// Runs `program args` in the query's folder, writes `input` (one JSON line each; stdin stays
/// open when `keep_stdin` is set, as an idle stream-json session needs), and returns the first
/// stdout line `pick` accepts. A stdout that ends without one is tried as one whole document.
pub(crate) async fn ask(
    program: OsString,
    args: Vec<OsString>,
    query: &UsageQuery,
    input: Vec<Value>,
    keep_stdin: bool,
    pick: impl Fn(&Value) -> Option<Value>,
) -> Result<Value, QueryError> {
    let spawned = process::spawn_in(program, args, &query.cwd, &query.env)?;
    let mut child = spawned.child;
    let mut stdin = Some(spawned.stdin);
    let read = async {
        if let Some(pipe) = stdin.as_mut() {
            for line in &input {
                pipe.write_all(format!("{line}\n").as_bytes()).await?;
            }
            pipe.flush().await?;
        }
        if !keep_stdin {
            stdin = None;
        }
        let mut lines = BufReader::new(spawned.stdout).lines();
        let mut all = String::new();
        while let Some(line) = lines.next_line().await? {
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && let Some(answer) = pick(&v)
            {
                return Ok(answer);
            }
            all.push_str(&line);
            all.push('\n');
        }
        serde_json::from_str::<Value>(&all)
            .ok()
            .and_then(|v| pick(&v))
            .ok_or(QueryError::NoAnswer)
    };
    let answer = match tokio::time::timeout(QUERY_TIMEOUT, read).await {
        Ok(answer) => answer,
        Err(_) => Err(QueryError::Timeout(QUERY_TIMEOUT.as_secs())),
    };
    drop(stdin);
    let _ = child.start_kill();
    let _ = child.wait().await;
    answer
}

/// Runs `program args` in the query's folder and returns what it printed to stdout once it
/// exits (for a command that prints a plain list and ends). Killed at the deadline.
pub(crate) async fn run_text(
    program: OsString,
    args: Vec<OsString>,
    query: &UsageQuery,
) -> Result<String, QueryError> {
    let spawned = process::spawn_in(program, args, &query.cwd, &query.env)?;
    let mut child = spawned.child;
    drop(spawned.stdin);
    let read = async {
        let mut out = String::new();
        let mut stdout = spawned.stdout;
        tokio::io::AsyncReadExt::read_to_string(&mut stdout, &mut out).await?;
        Ok(out)
    };
    let answer = match tokio::time::timeout(QUERY_TIMEOUT, read).await {
        Ok(answer) => answer,
        Err(_) => Err(QueryError::Timeout(QUERY_TIMEOUT.as_secs())),
    };
    let _ = child.start_kill();
    let _ = child.wait().await;
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sh(script: &str) -> (OsString, Vec<OsString>) {
        ("sh".into(), vec!["-c".into(), script.into()])
    }

    fn query() -> UsageQuery {
        UsageQuery {
            cwd: std::env::temp_dir(),
            env: vec![("ANSWER".into(), "42".into())],
        }
    }

    #[tokio::test]
    async fn the_first_accepted_line_is_the_answer() {
        let (p, a) = sh(r#"read q; echo '{"noise":1}'; echo "{\"answer\":$ANSWER}"; sleep 5"#);
        let got = ask(p, a, &query(), vec![json!({"q": 1})], true, |v| {
            v.get("answer").cloned()
        })
        .await;
        assert_eq!(got.ok(), Some(json!(42)));
    }

    #[tokio::test]
    async fn a_document_over_several_lines_is_read_whole() {
        let (p, a) = sh(r#"printf '{\n "answer": 7\n}\n'"#);
        let got = ask(p, a, &query(), Vec::new(), false, |v| {
            v.get("answer").cloned()
        })
        .await;
        assert_eq!(got.ok(), Some(json!(7)));
    }

    #[tokio::test]
    async fn run_text_returns_what_the_command_printed() {
        let (p, a) = sh("echo one; echo two");
        let got = run_text(p, a, &query()).await;
        assert_eq!(got.ok().as_deref(), Some("one\ntwo\n"));
    }

    #[tokio::test]
    async fn no_answer_is_an_error() {
        let (p, a) = sh("echo nothing");
        let got = ask(p, a, &query(), Vec::new(), false, |v| {
            v.get("answer").cloned()
        })
        .await;
        assert!(matches!(got, Err(QueryError::NoAnswer)));
    }
}

//! Hand-written NDJSON layer over `claude -p --input-format stream-json
//! --output-format stream-json`. Spike quality: enough to answer S1's checks
//! and record fixtures, not the real driver.
//!
//! Every stdout line is kept as raw `serde_json::Value` (unknown types pass
//! through untouched). `control_response` lines are routed to the waiting
//! `control()` call; `can_use_tool` requests surface as `Ev::Perm` and are
//! answered by `answer()` from any task, in any order.
//!
//! Each session writes a log: one JSON object per line,
//! `{"t": <ms since spawn>, "dir": "in"|"out"|"err", "msg": <line>}`.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

pub fn uuid() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "00000000-0000-4000-8000-000000000000".into())
}

pub struct Spec {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
}

/// Base flags every Dovetail session gets.
pub fn base_args() -> Vec<String> {
    [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug)]
pub enum Ev {
    /// Any stdout line that is not a control message (raw JSON).
    Line(Value),
    /// `control_request` with subtype `can_use_tool`.
    Perm { request_id: String, request: Value },
    /// Other inbound control requests (raw).
    OtherControl(Value),
    /// stdout closed.
    Closed,
}

struct Log {
    start: Instant,
    file: std::sync::Mutex<std::fs::File>,
}

impl Log {
    fn write(&self, dir: &str, msg: Value) {
        use std::io::Write;
        let t = self.start.elapsed().as_millis() as u64;
        let line = json!({"t": t, "dir": dir, "msg": msg});
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub struct Session {
    pub child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    counter: AtomicU64,
    log: Arc<Log>,
    pub start: Instant,
}

impl Session {
    pub async fn spawn(spec: Spec) -> std::io::Result<(Session, mpsc::UnboundedReceiver<Ev>)> {
        let mut cmd = Command::new("claude");
        cmd.args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let start = Instant::now();
        let log = Arc::new(Log {
            start,
            file: std::sync::Mutex::new(std::fs::File::create(&spec.log)?),
        });
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| std::io::Error::other("no stderr"))?;
        let stdin = child.stdin.take();
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>> = Arc::default();
        let (tx, rx) = mpsc::unbounded_channel();

        let (p2, l2) = (pending.clone(), log.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let v: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|_| json!({"_non_json": line}));
                l2.write("out", v.clone());
                let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
                match ty {
                    "control_response" => {
                        let id = v["response"]["request_id"].as_str().unwrap_or("").to_string();
                        if let Some(waiter) = p2.lock().await.remove(&id) {
                            let _ = waiter.send(v);
                        }
                    }
                    "control_request" => {
                        if v["request"]["subtype"] == "can_use_tool" {
                            let _ = tx.send(Ev::Perm {
                                request_id: v["request_id"].as_str().unwrap_or("").into(),
                                request: v["request"].clone(),
                            });
                        } else {
                            let _ = tx.send(Ev::OtherControl(v));
                        }
                    }
                    _ => {
                        let _ = tx.send(Ev::Line(v));
                    }
                }
            }
            let _ = tx.send(Ev::Closed);
        });
        let l3 = log.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                l3.write("err", Value::String(line));
            }
        });
        Ok((
            Session {
                child,
                stdin: Arc::new(Mutex::new(stdin)),
                pending,
                counter: AtomicU64::new(0),
                log,
                start,
            },
            rx,
        ))
    }

    pub fn secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub async fn write(&self, v: Value) -> std::io::Result<()> {
        self.log.write("in", v.clone());
        let mut g = self.stdin.lock().await;
        let stdin = g.as_mut().ok_or_else(|| std::io::Error::other("stdin closed"))?;
        stdin.write_all(format!("{v}\n").as_bytes()).await?;
        stdin.flush().await
    }

    pub async fn send_user(&self, text: &str) -> std::io::Result<()> {
        self.write(json!({
            "type": "user",
            "message": {"role": "user", "content": text},
            "parent_tool_use_id": null,
        }))
        .await
    }

    /// Send a control request and wait for its response.
    pub async fn control(&self, request: Value) -> std::io::Result<Value> {
        let id = format!("dt_{}", self.counter.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        self.write(json!({"type": "control_request", "request_id": id, "request": request}))
            .await?;
        rx.await.map_err(|_| std::io::Error::other("control response channel closed"))
    }

    /// Answer a `can_use_tool` request.
    pub async fn answer(&self, request_id: &str, allow: bool, input: &Value) -> std::io::Result<()> {
        let response = if allow {
            json!({"behavior": "allow", "updatedInput": input})
        } else {
            json!({"behavior": "deny", "message": "denied by spike"})
        };
        self.write(json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": response}
        }))
        .await
    }

    pub async fn close_stdin(&self) {
        let mut g = self.stdin.lock().await;
        if let Some(mut s) = g.take() {
            let _ = s.shutdown().await;
        }
    }

    pub async fn finish(mut self, timeout_s: u64) -> Option<std::process::ExitStatus> {
        self.close_stdin().await;
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), self.child.wait()).await {
            Ok(Ok(st)) => Some(st),
            _ => {
                let _ = self.child.kill().await;
                None
            }
        }
    }
}

pub fn ty(v: &Value) -> String {
    let t = v.get("type").and_then(Value::as_str).unwrap_or("?");
    match v.get("subtype").and_then(Value::as_str) {
        Some(s) => format!("{t}/{s}"),
        None => t.to_string(),
    }
}

pub fn ensure_dir(p: &Path) {
    let _ = std::fs::create_dir_all(p);
}

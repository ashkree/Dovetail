//! `FakeDriver`: replays a recorded fixture through the real driver's mapping and records what
//! it is sent.
//!
//! Fixtures are NDJSON, one `{"t": <ms>, "dir": "in"|"out"|"err", "msg": <line>}` per line
//! (`tests/fixtures/`). A bare line without those keys is treated as `out`. Only `out` lines
//! are replayed as events; `err` lines are skipped, and `in` lines are never sent (what the
//! recording sent is not what the code under test sends). An agy fixture
//! ([`FakeDriver::for_agy`]) still reads its `in` lines, so each recorded answer finds the
//! request it answers, as in the live driver. Pieces: `replay` (the loop), `session` (the
//! handle).
//!
//! ```ignore
//! let fake = FakeDriver::from_file("tests/fixtures/concurrent_permissions.ndjson")?
//!     .with_options(ReplayOptions { wait_for_answers: true, ..Default::default() });
//! let mut session = fake.start(SessionSpec::new("id", "/tmp"))?;
//! let mut events = session.take_events();
//! // ... events.recv().await, session.answer_control(..) ...
//! assert_eq!(fake.sent(), vec![/* Sent::Answer { .. } */]);
//! ```

mod replay;
mod session;

use crate::event::*;
use crate::{
    ControlAnswer, DriverError, DriverKind, PermissionMode, Session, SessionDriver, SessionSpec,
};
use replay::Shared;
use serde_json::Value;
use session::FakeSession;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// How the recorded `t` timestamps are honoured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplayTiming {
    /// Emit everything as fast as possible (default).
    Instant,
    /// Sleep the recorded gaps divided by `speed`, each capped at `max_gap`.
    Recorded {
        /// 1.0 is real time, 10.0 ten times faster.
        speed: f64,
        /// Upper bound for one gap (the fixtures contain a 60 s hold).
        max_gap: Duration,
    },
    /// The same pause between every event.
    Fixed(Duration),
}

/// Replay settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayOptions {
    /// Timing of the replay.
    pub timing: ReplayTiming,
    /// After a permission request (or a back-to-back burst of them), pause until every one has
    /// been answered with `answer_control`. Lets a test interleave answers deterministically.
    pub wait_for_answers: bool,
    /// Pause before emitting the fixture's n-th `out` line (0-based, counted over replayed lines)
    /// until [`FakeDriver::resume`] is called.
    pub pause_before: Vec<usize>,
    /// `(n, signal)`: the first session started dies before its n-th `out` line (after any pause
    /// there) and exits by `signal`, as an OOM-killed session does. Later sessions replay in full.
    pub kill_first_before: Option<(usize, i32)>,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            timing: ReplayTiming::Instant,
            wait_for_answers: false,
            pause_before: Vec::new(),
            kill_first_before: None,
        }
    }
}

/// One thing the code under test sent to a fake session.
#[derive(Debug, Clone, PartialEq)]
pub enum Sent {
    /// A user message.
    User(String),
    /// An answer to a permission request.
    Answer {
        /// The request.
        id: ControlId,
        /// The answer.
        answer: ControlAnswer,
    },
    /// An interrupt.
    Interrupt,
    /// A permission-mode switch.
    Mode(PermissionMode),
    /// A model switch.
    Model {
        /// The model asked for; `None` keeps it.
        model: Option<String>,
        /// The effort asked for.
        effort: Option<dovetail_api::agents::model::ReasoningEffort>,
    },
    /// An MCP menu request.
    Mcp(dovetail_api::agents::mcp::McpRequest),
    /// `shutdown(grace)`.
    Shutdown(Duration),
    /// `kill()`.
    Kill,
}

/// One recorded line the replay reads.
#[derive(Debug, Clone)]
struct Line {
    t: u64,
    /// `out` (the CLI wrote it) rather than `in` (the recording sent it).
    out: bool,
    msg: Value,
}

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Sent>>,
    specs: Mutex<Vec<SessionSpec>>,
    /// The stream-json line each user message would have been written as.
    user_lines: Mutex<Vec<Value>>,
    /// Model switch answers are held, not sent ([`FakeDriver::hold_model_answers`]).
    hold_models: std::sync::atomic::AtomicBool,
    /// Held model switch answers, in the order the switches were asked.
    held: Mutex<Vec<DriverEvent>>,
    /// Every session reports these settings as applied, as the start-up `get_settings` read
    /// does ([`FakeDriver::apply_at_start`]).
    applied: Mutex<
        Option<(
            Option<String>,
            Option<dovetail_api::agents::model::ReasoningEffort>,
        )>,
    >,
    /// What the fake's `mcp_status` lists ([`FakeDriver::set_mcp_servers`]).
    mcp_servers: Mutex<Vec<dovetail_api::agents::mcp::McpServer>>,
    /// Sessions refuse MCP requests, as a closed one does ([`FakeDriver::close_mcp`]).
    mcp_closed: std::sync::atomic::AtomicBool,
    /// The latest session's event stream, for [`FakeDriver::send_event`].
    latest: Mutex<Option<UnboundedSender<DriverEvent>>>,
    /// What `usage` reports ([`FakeDriver::set_usage`]); `None` reports an empty one.
    usage: Mutex<Option<dovetail_api::activity::status::DriverUsage>>,
    /// The queries `usage` was asked.
    usage_queries: Mutex<Vec<crate::UsageQuery>>,
    /// What `models` answers ([`FakeDriver::set_models`]); `None`: an error.
    models: Mutex<Option<Result<Vec<crate::ModelInfo>, crate::ModelsError>>>,
    /// How many times `models` was asked.
    models_asked: std::sync::atomic::AtomicUsize,
}

/// Replays a fixture as the event stream of every session it starts, and records what it is
/// sent. Clones share the recording.
#[derive(Clone)]
pub struct FakeDriver {
    lines: Arc<Vec<Line>>,
    options: ReplayOptions,
    recorder: Arc<Recorder>,
    gate: Arc<Notify>,
    kind: DriverKind,
}

impl FakeDriver {
    /// Load a fixture file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, DriverError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| DriverError::Fixture(format!("{}: {e}", path.display())))?;
        Self::from_ndjson(&text)
    }

    /// Build from fixture text.
    pub fn from_ndjson(text: &str) -> Result<Self, DriverError> {
        let mut lines = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(raw)
                .map_err(|e| DriverError::Fixture(format!("line {}: {e}", n + 1)))?;
            let (t, dir, msg) = match (v.get("dir"), v.get("msg")) {
                (Some(d), Some(m)) => (
                    v.get("t").and_then(Value::as_u64).unwrap_or(0),
                    d.as_str().unwrap_or("out").to_owned(),
                    m.clone(),
                ),
                _ => (0, "out".to_owned(), v),
            };
            if dir == "out" || dir == "in" {
                lines.push(Line {
                    t,
                    out: dir == "out",
                    msg,
                });
            }
        }
        Ok(Self {
            lines: Arc::new(lines),
            options: ReplayOptions::default(),
            recorder: Arc::default(),
            gate: Arc::default(),
            kind: DriverKind::Claude,
        })
    }

    /// From now on, every session reports `model` and `effort` as applied at start (Claude
    /// Code's start-up `get_settings` read), whatever it was asked for. `model: None` reports
    /// the model the session was started on.
    pub fn apply_at_start(
        &self,
        model: Option<&str>,
        effort: Option<dovetail_api::agents::model::ReasoningEffort>,
    ) {
        if let Ok(mut applied) = self.recorder.applied.lock() {
            *applied = Some((model.map(str::to_owned), effort));
        }
    }

    /// Replays the fixture as an agy (ACP) recording, and reports [`DriverKind::Agy`].
    pub fn for_agy(mut self) -> Self {
        self.kind = DriverKind::Agy;
        self
    }

    /// Replace the replay options.
    pub fn with_options(mut self, options: ReplayOptions) -> Self {
        self.options = options;
        self
    }

    /// The stream-json `user` lines the real driver would have written for every message sent
    /// so far, in order.
    pub fn user_messages(&self) -> Vec<Value> {
        self.recorder
            .user_lines
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Everything sent to any session of this driver, in order.
    pub fn sent(&self) -> Vec<Sent> {
        self.recorder
            .sent
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// The specs sessions were started with.
    pub fn started_specs(&self) -> Vec<SessionSpec> {
        self.recorder
            .specs
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// What every session's `mcp_status` lists from now on. Actions on a server not in the list
    /// are refused, as Claude Code refuses an unknown server.
    pub fn set_mcp_servers(&self, servers: Vec<dovetail_api::agents::mcp::McpServer>) {
        if let Ok(mut list) = self.recorder.mcp_servers.lock() {
            *list = servers;
        }
    }

    /// What `usage` reports from now on.
    pub fn set_usage(&self, usage: dovetail_api::activity::status::DriverUsage) {
        if let Ok(mut u) = self.recorder.usage.lock() {
            *u = Some(usage);
        }
    }

    /// What `models` answers from now on.
    pub fn set_models(&self, models: Result<Vec<crate::ModelInfo>, crate::ModelsError>) {
        if let Ok(mut m) = self.recorder.models.lock() {
            *m = Some(models);
        }
    }

    /// How many times `models` was asked.
    pub fn models_asked(&self) -> usize {
        self.recorder
            .models_asked
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The queries `usage` was asked, in order.
    pub fn usage_queries(&self) -> Vec<crate::UsageQuery> {
        self.recorder
            .usage_queries
            .lock()
            .map(|q| q.clone())
            .unwrap_or_default()
    }

    /// From now on `mcp` fails with `SessionClosed`, as when the process is gone.
    pub fn close_mcp(&self) {
        self.recorder
            .mcp_closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// From now on, model switch answers are held until [`FakeDriver::release_model_answers`],
    /// so a test can order them against other answers.
    pub fn hold_model_answers(&self) {
        self.recorder
            .hold_models
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Sends the held model switch answers, in `order` (indexes into the held list, oldest
    /// switch 0), and stops holding.
    pub fn release_model_answers(&self, order: &[usize]) {
        self.recorder
            .hold_models
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let held = self
            .recorder
            .held
            .lock()
            .map(|mut h| std::mem::take(&mut *h))
            .unwrap_or_default();
        for &i in order {
            if let Some(event) = held.get(i) {
                self.send_event(event.clone());
            }
        }
    }

    /// Sends `event` on the latest session's stream, as if the process had produced it (a late
    /// or stale answer, say).
    pub fn send_event(&self, event: DriverEvent) {
        if let Ok(tx) = self.recorder.latest.lock()
            && let Some(tx) = tx.as_ref()
        {
            let _ = tx.send(event);
        }
    }

    /// Release a replay paused by [`ReplayOptions::pause_before`] (one pause per call).
    pub fn resume(&self) {
        self.gate.notify_one();
    }
}

impl SessionDriver for FakeDriver {
    fn kind(&self) -> DriverKind {
        self.kind
    }

    fn usage(&self, query: crate::UsageQuery) -> crate::UsageFuture {
        if let Ok(mut q) = self.recorder.usage_queries.lock() {
            q.push(query);
        }
        let usage = self
            .recorder
            .usage
            .lock()
            .ok()
            .and_then(|u| u.clone())
            .unwrap_or(dovetail_api::activity::status::DriverUsage {
                driver: self.kind,
                plan: None,
                groups: Vec::new(),
                note: None,
                failed: false,
            });
        Box::pin(async move { usage })
    }

    fn models(&self, _query: crate::UsageQuery) -> crate::ModelsFuture {
        self.recorder
            .models_asked
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let answer = self
            .recorder
            .models
            .lock()
            .ok()
            .and_then(|m| m.clone())
            .unwrap_or_else(|| Err(crate::ModelsError("no models set".to_owned())));
        Box::pin(async move { answer })
    }

    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| DriverError::NoRuntime)?;
        let mut options = self.options.clone();
        if let Ok(mut s) = self.recorder.specs.lock() {
            s.push(spec.clone());
            if s.len() > 1 {
                options.kill_first_before = None;
            }
        }
        let (tx, rx) = unbounded_channel();
        if let Ok(mut latest) = self.recorder.latest.lock() {
            *latest = Some(tx.clone());
        }
        let applied = self.recorder.applied.lock().ok().and_then(|a| a.clone());
        if let Some((model, effort)) = applied {
            let _ = tx.send(DriverEvent::ModelSettings {
                switch: None,
                model: model.or_else(|| spec.model.clone()),
                effort,
            });
        }
        let shared = Arc::new(Shared::default());
        if spec
            .model
            .as_deref()
            .is_some_and(|m| m.starts_with("bogus"))
        {
            // Claude Code refuses an unknown `--model` and exits at once.
            let _ = tx.send(DriverEvent::Exited(ExitInfo {
                code: Some(1),
                signal: None,
                success: false,
                stderr_tail: vec![format!(
                    "Error: Model '{}' not found",
                    spec.model.as_deref().unwrap_or_default()
                )],
            }));
        } else {
            handle.spawn(replay::replay(
                self.lines.clone(),
                options,
                spec.clone(),
                self.kind,
                tx.clone(),
                shared.clone(),
                self.gate.clone(),
            ));
        }
        Ok(Box::new(FakeSession {
            session_id: spec.session_id,
            events: Some(rx),
            events_tx: tx,
            recorder: self.recorder.clone(),
            shared,
        }))
    }
}

//! `FakeDriver`: replays a recorded fixture through the real parser and records what it is sent.
//!
//! Fixtures are NDJSON, one `{"t": <ms>, "dir": "in"|"out"|"err", "msg": <line>}` per line
//! (`tests/fixtures/`). A bare stream-json line without those keys is treated as `out`. Only
//! `out` lines are replayed; `in` and `err` lines are skipped (what the recording sent is not
//! what the code under test sends).
//!
//! ```ignore
//! let fake = FakeDriver::from_file("tests/fixtures/concurrent_permissions.ndjson")?
//!     .with_options(ReplayOptions { wait_for_answers: true, ..Default::default() });
//! let mut session = fake.start(SessionSpec::new("id", "/tmp"))?;
//! let mut events = session.take_events();
//! // ... events.recv().await, session.answer_control(..) ...
//! assert_eq!(fake.sent(), vec![/* Sent::Answer { .. } */]);
//! ```

use crate::event::*;
use crate::parse;
use crate::{
    ControlAnswer, DriverError, PermissionMode, Session, SessionDriver, SessionSpec, UserInput,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            timing: ReplayTiming::Instant,
            wait_for_answers: false,
            pause_before: Vec::new(),
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
    /// `shutdown(grace)`.
    Shutdown(Duration),
    /// `kill()`.
    Kill,
}

#[derive(Debug, Clone)]
struct Line {
    t: u64,
    msg: Value,
}

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<Sent>>,
    specs: Mutex<Vec<SessionSpec>>,
}

/// Replays a fixture as the event stream of every session it starts, and records what it is
/// sent. Clones share the recording.
#[derive(Clone)]
pub struct FakeDriver {
    lines: Arc<Vec<Line>>,
    options: ReplayOptions,
    recorder: Arc<Recorder>,
    gate: Arc<Notify>,
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
            if dir == "out" {
                lines.push(Line { t, msg });
            }
        }
        Ok(Self {
            lines: Arc::new(lines),
            options: ReplayOptions::default(),
            recorder: Arc::default(),
            gate: Arc::default(),
        })
    }

    /// Replace the replay options.
    pub fn with_options(mut self, options: ReplayOptions) -> Self {
        self.options = options;
        self
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

    /// Release a replay paused by [`ReplayOptions::pause_before`] (one pause per call).
    pub fn resume(&self) {
        self.gate.notify_one();
    }
}

impl SessionDriver for FakeDriver {
    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| DriverError::NoRuntime)?;
        if let Ok(mut s) = self.recorder.specs.lock() {
            s.push(spec.clone());
        }
        let (tx, rx) = unbounded_channel();
        let shared = Arc::new(Shared::default());
        handle.spawn(replay(
            self.lines.clone(),
            self.options.clone(),
            tx,
            shared.clone(),
            self.gate.clone(),
        ));
        Ok(Box::new(FakeSession {
            session_id: spec.session_id,
            events: Some(rx),
            recorder: self.recorder.clone(),
            shared,
        }))
    }
}

#[derive(Default)]
struct Shared {
    /// Permission requests the session has emitted and not yet seen answered.
    outstanding: Mutex<HashSet<String>>,
    answered: Notify,
    closed: Mutex<bool>,
}

impl Shared {
    fn is_closed(&self) -> bool {
        self.closed.lock().map(|c| *c).unwrap_or(true)
    }
}

async fn replay(
    lines: Arc<Vec<Line>>,
    options: ReplayOptions,
    tx: UnboundedSender<DriverEvent>,
    shared: Arc<Shared>,
    gate: Arc<Notify>,
) {
    let mut prev_t = 0u64;
    for (n, line) in lines.iter().enumerate() {
        if shared.is_closed() {
            break;
        }
        if options.pause_before.contains(&n) {
            gate.notified().await;
        }
        match options.timing {
            ReplayTiming::Instant => {}
            ReplayTiming::Fixed(d) => tokio::time::sleep(d).await,
            ReplayTiming::Recorded { speed, max_gap } => {
                let gap = Duration::from_millis(line.t.saturating_sub(prev_t));
                tokio::time::sleep(gap.div_f64(speed.max(f64::MIN_POSITIVE)).min(max_gap)).await;
            }
        }
        prev_t = line.t;
        for ev in parse::map_value(line.msg.clone()) {
            if let DriverEvent::PermissionRequest(req) = &ev
                && let Ok(mut o) = shared.outstanding.lock()
            {
                o.insert(req.id.0.clone());
            }
            if tx.send(ev).is_err() {
                return;
            }
        }
        if options.wait_for_answers && is_permission_request(&line.msg) {
            let next_is_request = lines
                .get(n + 1)
                .is_some_and(|l| is_permission_request(&l.msg));
            if !next_is_request {
                loop {
                    let empty = shared
                        .outstanding
                        .lock()
                        .map(|o| o.is_empty())
                        .unwrap_or(true);
                    if empty || shared.is_closed() {
                        break;
                    }
                    shared.answered.notified().await;
                }
            }
        }
    }
    let _ = tx.send(DriverEvent::Exited(ExitInfo {
        code: Some(0),
        signal: None,
        success: true,
        stderr_tail: Vec::new(),
    }));
}

fn is_permission_request(msg: &Value) -> bool {
    msg.get("type").and_then(Value::as_str) == Some("control_request")
        && msg.pointer("/request/subtype").and_then(Value::as_str) == Some("can_use_tool")
}

struct FakeSession {
    session_id: String,
    events: Option<UnboundedReceiver<DriverEvent>>,
    recorder: Arc<Recorder>,
    shared: Arc<Shared>,
}

impl FakeSession {
    fn record(&self, s: Sent) {
        if let Ok(mut v) = self.recorder.sent.lock() {
            v.push(s);
        }
    }

    fn close(&self) {
        if let Ok(mut c) = self.shared.closed.lock() {
            *c = true;
        }
        self.shared.answered.notify_one();
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        self.close();
    }
}

impl Session for FakeSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn pid(&self) -> Option<u32> {
        None
    }

    fn send(&self, input: UserInput) -> Result<(), DriverError> {
        self.record(Sent::User(input.text));
        Ok(())
    }

    fn interrupt(&self) -> Result<(), DriverError> {
        self.record(Sent::Interrupt);
        Ok(())
    }

    fn set_permission_mode(&self, mode: PermissionMode) -> Result<(), DriverError> {
        self.record(Sent::Mode(mode));
        Ok(())
    }

    fn answer_control(&self, id: ControlId, answer: ControlAnswer) -> Result<(), DriverError> {
        let known = self
            .shared
            .outstanding
            .lock()
            .map(|mut o| o.remove(&id.0))
            .unwrap_or(false);
        if !known {
            return Err(DriverError::UnknownControl(id));
        }
        self.record(Sent::Answer { id, answer });
        self.shared.answered.notify_one();
        Ok(())
    }

    fn take_events(&mut self) -> UnboundedReceiver<DriverEvent> {
        self.events.take().unwrap_or_else(|| unbounded_channel().1)
    }

    fn shutdown(&self, grace: Duration) {
        self.record(Sent::Shutdown(grace));
        self.close();
    }

    fn kill(&self) {
        self.record(Sent::Kill);
        self.close();
    }
}

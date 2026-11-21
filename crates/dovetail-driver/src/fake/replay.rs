//! The replay loop: a fixture's lines through the driver's own mapping, at the chosen pace,
//! pausing where the options say.

use super::{Line, ReplayOptions, ReplayTiming};
use crate::agy::conversation::{Conversation, Plan};
use crate::event::*;
use crate::{DriverKind, SessionSpec, parse};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

/// What a replay and its session handle share.
#[derive(Default)]
pub(super) struct Shared {
    /// Permission requests the session has emitted and not yet seen answered.
    pub(super) outstanding: Mutex<HashSet<String>>,
    pub(super) answered: Notify,
    pub(super) closed: Mutex<bool>,
}

impl Shared {
    pub(super) fn is_closed(&self) -> bool {
        self.closed.lock().map(|c| *c).unwrap_or(true)
    }
}

/// Maps recorded lines as the real driver of the fixture's kind does.
enum Mapper {
    /// Claude Code: each line on its own.
    Claude,
    /// agy: one conversation, which also learns the recorded requests (`in` lines) so their
    /// answers map. What it would write back is dropped.
    Agy(Box<Conversation>),
}

impl Mapper {
    fn new(kind: DriverKind, spec: &SessionSpec) -> Self {
        match kind {
            DriverKind::Claude => Self::Claude,
            DriverKind::Agy => Self::Agy(Box::new(Conversation::new(Plan::of(spec, None)))),
        }
    }

    fn map(&mut self, line: &Line) -> Vec<DriverEvent> {
        match self {
            Self::Claude if line.out => parse::map_value(line.msg.clone()),
            Self::Claude => Vec::new(),
            Self::Agy(c) if line.out => c.received(&line.msg).events,
            Self::Agy(c) => {
                c.note_sent(&line.msg);
                Vec::new()
            }
        }
    }

    fn is_permission_request(&self, msg: &Value) -> bool {
        match self {
            Self::Claude => {
                msg.get("type").and_then(Value::as_str) == Some("control_request")
                    && msg.pointer("/request/subtype").and_then(Value::as_str)
                        == Some("can_use_tool")
            }
            Self::Agy(_) => {
                msg.get("method").and_then(Value::as_str) == Some("session/request_permission")
            }
        }
    }

    /// Whether a line ends a burst of permission requests: Claude Code's requests of one burst
    /// arrive together, possibly with the assistant lines of the calls between them (the
    /// `concurrent_permissions` recording), up to the next tool result or `result`. agy asks one
    /// at a time.
    fn ends_burst(&self, msg: &Value) -> bool {
        match self {
            Self::Claude => matches!(
                msg.get("type").and_then(Value::as_str),
                Some("user" | "result")
            ),
            Self::Agy(_) => true,
        }
    }
}

/// Replays `lines`. Each init reports the folder the session was started in, as a real CLI
/// does, not the folder the fixture was recorded in.
pub(super) async fn replay(
    lines: Arc<Vec<Line>>,
    options: ReplayOptions,
    spec: SessionSpec,
    kind: DriverKind,
    tx: UnboundedSender<DriverEvent>,
    shared: Arc<Shared>,
    gate: Arc<Notify>,
) {
    let cwd = spec.cwd.to_string_lossy().into_owned();
    let mut mapper = Mapper::new(kind, &spec);
    let mut prev_t = 0u64;
    let mut n = 0;
    for (at, line) in lines.iter().enumerate() {
        if !line.out {
            mapper.map(line);
            continue;
        }
        if shared.is_closed() {
            break;
        }
        if options.pause_before.contains(&n) {
            gate.notified().await;
        }
        if let Some((before, signal)) = options.kill_first_before
            && before == n
        {
            let _ = tx.send(DriverEvent::Exited(ExitInfo {
                code: None,
                signal: Some(signal),
                success: false,
                stderr_tail: Vec::new(),
            }));
            return;
        }
        n += 1;
        match options.timing {
            ReplayTiming::Instant => {}
            ReplayTiming::Fixed(d) => tokio::time::sleep(d).await,
            ReplayTiming::Recorded { speed, max_gap } => {
                let gap = Duration::from_millis(line.t.saturating_sub(prev_t));
                tokio::time::sleep(gap.div_f64(speed.max(f64::MIN_POSITIVE)).min(max_gap)).await;
            }
        }
        prev_t = line.t;
        for mut ev in mapper.map(line) {
            if let DriverEvent::Init(init) = &mut ev
                && init.cwd.is_some()
            {
                init.cwd = Some(cwd.clone());
            }
            if let DriverEvent::PermissionRequest(req) = &ev
                && let Ok(mut o) = shared.outstanding.lock()
            {
                o.insert(req.id.0.clone());
            }
            if tx.send(ev).is_err() {
                return;
            }
        }
        if options.wait_for_answers && mapper.is_permission_request(&line.msg) {
            let more_in_burst = lines[at + 1..]
                .iter()
                .filter(|l| l.out)
                .take_while(|l| !mapper.ends_burst(&l.msg))
                .any(|l| mapper.is_permission_request(&l.msg));
            if !more_in_burst {
                wait_for_answers(&shared).await;
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

/// Until every request emitted so far is answered (or the session closes).
async fn wait_for_answers(shared: &Shared) {
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

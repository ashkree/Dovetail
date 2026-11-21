//! The stdout reader of a `claude` process: each line through [`parse`], control answers routed
//! to the request that waits for them (a model switch step, an MCP menu request), everything
//! else on to the event stream.

use super::mcp::McpCalls;
use super::parse;
use super::switch::Switches;
use crate::event::DriverEvent;
use crate::process::{self, Log, WriterMsg};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::process::ChildStdout;
use tokio::sync::mpsc::UnboundedSender;

/// What the stdout reader shares with the session handle.
pub(super) struct Reader {
    pub(super) events: UnboundedSender<DriverEvent>,
    /// For the next request of a model switch, sent when the previous one is answered.
    pub(super) writer: UnboundedSender<WriterMsg>,
    /// Inputs of pending `can_use_tool` requests, to echo back when the answer has none.
    pub(super) pending: Arc<Mutex<HashMap<String, Value>>>,
    pub(super) switches: Arc<Mutex<Switches>>,
    pub(super) mcp: Arc<Mutex<McpCalls>>,
    pub(super) log: Option<Arc<Log>>,
}

impl Reader {
    /// Once `initialize` is answered: read the launch model and effort, for the usage row before
    /// any switch, and what each model accepts, for the selector (task #39). Neither makes a
    /// model call.
    fn after_initialize(&self) {
        let lines = self
            .switches
            .lock()
            .map(|mut s| s.after_initialize())
            .unwrap_or_default();
        for line in lines {
            let _ = self.writer.send(WriterMsg::Line(line));
        }
    }

    /// A switch step's answer becomes the next request or the switch's result; any other event
    /// passes through.
    fn route(&self, ev: DriverEvent) -> Option<DriverEvent> {
        let DriverEvent::ControlResponse {
            request_id,
            success,
            response,
        } = &ev
        else {
            return Some(ev);
        };
        let mcp = self
            .mcp
            .lock()
            .ok()
            .and_then(|mut m| m.answer(request_id, *success, response));
        if mcp.is_some() {
            return mcp;
        }
        let outcome = self
            .switches
            .lock()
            .ok()
            .and_then(|mut s| s.answer(request_id, *success, response));
        let Some(outcome) = outcome else {
            return Some(ev);
        };
        if let Some(line) = outcome.send {
            let _ = self.writer.send(WriterMsg::Line(line));
        }
        outcome.event
    }

    /// One stdout line; `false` once nobody listens any more.
    fn on_line(&self, line: &str) -> bool {
        if parse::is_initialize_answer(line) {
            self.after_initialize();
        }
        for ev in parse::map_line(line) {
            let Some(ev) = self.route(ev) else {
                continue;
            };
            if let DriverEvent::PermissionRequest(req) = &ev
                && let Ok(mut map) = self.pending.lock()
            {
                map.insert(req.id.0.clone(), req.input.clone());
            }
            if self.events.send(ev).is_err() {
                return false;
            }
        }
        true
    }
}

/// Reads until stdout closes. Never awaits anything but the next line and an unbounded send, so
/// a held permission request cannot stall the stream.
pub(super) async fn read_stdout(stdout: ChildStdout, reader: Reader) {
    process::read_lines(stdout, reader.log.clone(), |line| reader.on_line(line)).await;
    // The session is over: whatever the MCP menu still waits for will not be answered.
    let ended = reader
        .mcp
        .lock()
        .map(|mut m| m.fail_all("the session ended"))
        .unwrap_or_default();
    for event in ended {
        let _ = reader.events.send(event);
    }
}

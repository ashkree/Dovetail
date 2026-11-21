//! The fake session handle: records everything sent and answers what no fixture records (model
//! switches, the MCP menu) at once.

use super::replay::Shared;
use super::{Recorder, Sent};
use crate::event::*;
use crate::{ControlAnswer, DriverError, PermissionMode, Session, UserInput};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub(super) struct FakeSession {
    pub(super) session_id: String,
    pub(super) events: Option<UnboundedReceiver<DriverEvent>>,
    /// For answering a model switch, which no fixture records.
    pub(super) events_tx: UnboundedSender<DriverEvent>,
    pub(super) recorder: Arc<Recorder>,
    pub(super) shared: Arc<Shared>,
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
        if let Ok(mut lines) = self.recorder.user_lines.lock() {
            lines.push(input.to_stream_json());
        }
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

    /// Records the switch and confirms it at once with exactly what was asked, except a model
    /// named `bogus…`, which is refused as Claude Code refuses an unknown model.
    fn set_model(
        &self,
        model: Option<&str>,
        effort: Option<dovetail_api::ReasoningEffort>,
    ) -> Result<SwitchId, DriverError> {
        // Numbered across the fake's sessions; only distinctness matters.
        let switch = SwitchId(
            self.recorder
                .sent
                .lock()
                .map(|s| s.iter().filter(|s| matches!(s, Sent::Model { .. })).count() as u64)
                .unwrap_or(0),
        );
        self.record(Sent::Model {
            model: model.map(str::to_owned),
            effort,
        });
        let event = match model {
            Some(m) if m.starts_with("bogus") => DriverEvent::ModelSwitchFailed {
                switch,
                error: format!("Model '{m}' not found"),
            },
            _ => DriverEvent::ModelSettings {
                switch: Some(switch),
                model: model.map(str::to_owned),
                effort,
            },
        };
        if self
            .recorder
            .hold_models
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            if let Ok(mut held) = self.recorder.held.lock() {
                held.push(event);
            }
        } else {
            let _ = self.events_tx.send(event);
        }
        Ok(switch)
    }

    /// Records the request and answers it at once from [`FakeDriver::set_mcp_servers`]: a list
    /// for a status request, `Done` for an action on a listed server (an `AuthUrl` for a
    /// log-in), `Failed` for an unknown one.
    fn mcp(&self, request: dovetail_api::McpRequest) -> Result<(), DriverError> {
        use dovetail_api::{McpOutcome, McpRequest};
        if self
            .recorder
            .mcp_closed
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(DriverError::SessionClosed);
        }
        self.record(Sent::Mcp(request.clone()));
        let servers = self
            .recorder
            .mcp_servers
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let outcome = match (&request, request.server()) {
            (McpRequest::Status, _) => McpOutcome::Status(servers),
            (_, Some(name)) if !servers.iter().any(|s| s.name == name) => {
                McpOutcome::Failed(format!("Server not found: {name}"))
            }
            (McpRequest::Authenticate { .. }, _) => McpOutcome::AuthUrl {
                url: "https://login.example/start".into(),
                callback_expected: false,
            },
            _ => McpOutcome::Done,
        };
        let _ = self
            .events_tx
            .send(DriverEvent::McpReply { request, outcome });
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

    /// Graceful, like a real session whose stdin closes: the replay goes on to its end (a turn
    /// in progress can still finish); only `kill` stops it.
    fn shutdown(&self, grace: Duration) {
        self.record(Sent::Shutdown(grace));
    }

    fn kill(&self) {
        self.record(Sent::Kill);
        self.close();
    }
}

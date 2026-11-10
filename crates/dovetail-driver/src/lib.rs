//! `SessionDriver`: runs headless Claude Code sessions over the NDJSON stream protocol.
//!
//! The driver hides Claude Code entirely behind [`SessionDriver`] and [`Session`]. The core never
//! sees a process, a pipe or a raw line unless it asks for it: every stdout line becomes a typed
//! [`DriverEvent`] that still carries (or can carry) its raw JSON.
//!
//! # Stream rules (spike S1, `docs/spikes/driver.md`; these are law)
//!
//! 1. **Route by `parent_tool_use_id`.** A subagent's events arrive on the parent session's
//!    stream. Every event that can come from a subagent carries an [`Origin`]; a non-`None`
//!    `parent_tool_use_id` names the `Task` tool use that spawned it. `agent_id` is set too when
//!    the CLI provides it (always on subagent permission requests).
//! 2. **The first `result` is not the end.** A background subagent produces several
//!    [`DriverEvent::TurnResult`]s per prompt (`result_index` counts them), each after a fresh
//!    `Init`. A session is done when the process exits ([`DriverEvent::Exited`]) or when no
//!    background tasks remain ([`DriverEvent::BackgroundTasksChanged`] with an empty list), never
//!    on a `result` alone.
//! 3. **Answer permission requests concurrently.** The stdout reader never waits for an answer.
//!    All [`Session`] methods only enqueue, so a held [`DriverEvent::PermissionRequest`] never
//!    blocks another request, the stream, or an interrupt. Answers may come in any order.
//! 4. **Record session ids yourself.** Dovetail picks the id (`--session-id`) and stores it; the
//!    `Init` event confirms it. Resume with [`SessionSpec::resume`].
//! 5. **Never crash on the unknown.** Unknown line types, subtypes and fields end up in
//!    [`DriverEvent::Unknown`] with the raw JSON; non-JSON lines too.
//!
//! # Pieces
//!
//! - [`SessionDriver`] / [`Session`]: the traits the core depends on.
//! - [`ClaudeDriver`]: spawns the real `claude` (optionally inside `systemd-run --user --scope`).
//! - [`FakeDriver`]: replays recorded fixtures through the same parser and records everything
//!   sent to it.
//! - [`parse`]: the line-to-event mapping, usable on its own.

mod base64;
mod claude;
mod event;
mod fake;
pub mod parse;
mod spec;
pub mod transcript;

pub use claude::ClaudeDriver;
pub use event::*;
pub use fake::{FakeDriver, ReplayOptions, ReplayTiming, Sent};
pub use spec::{PermissionMode, SessionSpec, SystemdScope};

use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

/// Errors from the driver layer.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The `claude` process (or `systemd-run`) could not be started.
    #[error("failed to spawn session process: {0}")]
    Spawn(#[source] std::io::Error),
    /// `start` was called outside a tokio runtime.
    #[error("no tokio runtime is running")]
    NoRuntime,
    /// The session has exited or been shut down, so nothing can be sent to it.
    #[error("session is closed")]
    SessionClosed,
    /// An answer named a control request that is not pending (never seen, or already answered).
    #[error("no pending control request {0:?}")]
    UnknownControl(ControlId),
    /// A fixture could not be read or parsed (fake driver).
    #[error("bad fixture: {0}")]
    Fixture(String),
    /// Opening the optional NDJSON log failed.
    #[error("log file error: {0}")]
    Log(#[source] std::io::Error),
}

/// Starts sessions. Implemented by [`ClaudeDriver`] and [`FakeDriver`].
pub trait SessionDriver: Send + Sync + 'static {
    /// Start (or, with [`SessionSpec::resume`], resume) a session. Must be called inside a tokio
    /// runtime. Returns once the process is spawned; events arrive on the session's stream.
    fn start(&self, spec: SessionSpec) -> Result<Box<dyn Session>, DriverError>;
}

/// One running session. Methods only enqueue: they never wait on the process, so a held
/// permission request can never block anything else.
pub trait Session: Send {
    /// The session id Dovetail chose for this session.
    fn session_id(&self) -> &str;
    /// OS process id, if a real process exists.
    fn pid(&self) -> Option<u32>;
    /// Send a user message (starts or queues a turn).
    fn send(&self, input: UserInput) -> Result<(), DriverError>;
    /// Interrupt the current turn (control request `interrupt`). The session stays alive.
    fn interrupt(&self) -> Result<(), DriverError>;
    /// Switch permission mode on the live process (control request `set_permission_mode`).
    fn set_permission_mode(&self, mode: PermissionMode) -> Result<(), DriverError>;
    /// Answer a pending [`DriverEvent::PermissionRequest`], in any order.
    fn answer_control(&self, id: ControlId, answer: ControlAnswer) -> Result<(), DriverError>;
    /// Take the event stream. Call once; later calls return an already-closed receiver. The
    /// stream ends right after [`DriverEvent::Exited`].
    fn take_events(&mut self) -> UnboundedReceiver<DriverEvent>;
    /// Graceful stop: close stdin so the process exits by itself, kill it after `grace`.
    fn shutdown(&self, grace: Duration);
    /// Kill now. The process is still reaped and `Exited` is still delivered.
    fn kill(&self);
}

/// A user message to send.
#[derive(Debug, Clone, PartialEq)]
pub struct UserInput {
    /// Plain message text; may be empty when images are attached.
    pub text: String,
    /// Images sent before the text, as image content blocks.
    pub images: Vec<ImageInput>,
}

/// One image in a [`UserInput`].
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInput {
    /// MIME type: `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub media_type: String,
    /// The raw file bytes; the driver base64-encodes them.
    pub data: Vec<u8>,
}

impl UserInput {
    /// A text message.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }

    /// The stream-json `user` line for this message. Text alone goes as a plain string, as
    /// before; with images, `content` is the image blocks followed by a text block (when the
    /// text is not empty).
    pub fn to_stream_json(&self) -> Value {
        let content = if self.images.is_empty() {
            Value::String(self.text.clone())
        } else {
            let mut blocks: Vec<Value> = self
                .images
                .iter()
                .map(|image| {
                    serde_json::json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": image.media_type,
                            "data": base64::encode(&image.data),
                        },
                    })
                })
                .collect();
            if !self.text.is_empty() {
                blocks.push(serde_json::json!({"type": "text", "text": self.text}));
            }
            Value::Array(blocks)
        };
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": content},
            "parent_tool_use_id": null,
        })
    }
}

/// Our answer to a `can_use_tool` control request.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlAnswer {
    /// Allow the tool call.
    Allow {
        /// Replacement tool input; `None` echoes the original input back unchanged.
        updated_input: Option<Value>,
        /// Permission rules to apply from now on ("always allow"), usually taken from
        /// [`PermissionRequest::suggestions`]. Sent as `updatedPermissions`.
        updated_permissions: Vec<Value>,
    },
    /// Deny the tool call; `message` is shown to the model.
    Deny {
        /// Reason given to the model.
        message: String,
        /// Also interrupt the turn.
        interrupt: bool,
    },
}

impl ControlAnswer {
    /// Allow once with the original input.
    pub fn allow() -> Self {
        Self::Allow {
            updated_input: None,
            updated_permissions: Vec::new(),
        }
    }

    /// Allow and apply the given rule suggestions.
    pub fn allow_with_rules(rules: Vec<Value>) -> Self {
        Self::Allow {
            updated_input: None,
            updated_permissions: rules,
        }
    }

    /// Deny with a message, without interrupting.
    pub fn deny(message: impl Into<String>) -> Self {
        Self::Deny {
            message: message.into(),
            interrupt: false,
        }
    }
}

//! Typed events mapped from the raw stream JSON (see [`crate::parse`]).

use serde_json::Value;

/// Id of a control request that Claude Code sent us (a `can_use_tool` request).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControlId(pub String);

/// Where an event came from. Route by this: a subagent's events share the parent's stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origin {
    /// `parent_tool_use_id`: the `Task` tool use that spawned the subagent, `None` for the
    /// session's own events.
    pub parent_tool_use_id: Option<String>,
    /// `agent_id` of the subagent, when the CLI sends it.
    pub agent_id: Option<String>,
}

impl Origin {
    /// True for events of the session itself (not a subagent).
    pub fn is_main(&self) -> bool {
        self.parent_tool_use_id.is_none() && self.agent_id.is_none()
    }
}

/// One MCP server as listed in `system/init`.
#[derive(Debug, Clone, PartialEq)]
pub struct McpServerInfo {
    /// Server name.
    pub name: String,
    /// Status string, e.g. `connected`.
    pub status: String,
}

/// `system/init`. There is a fresh one before every `result` of a background-subagent turn.
#[derive(Debug, Clone, PartialEq)]
pub struct InitInfo {
    /// Session id the process reports.
    pub session_id: String,
    /// Model id.
    pub model: Option<String>,
    /// Permission mode as the CLI names it (`auto`, `default`, ...).
    pub permission_mode: Option<String>,
    /// Working directory (the worktree for slice sessions).
    pub cwd: Option<String>,
    /// Tool names.
    pub tools: Vec<String>,
    /// MCP servers and their status.
    pub mcp_servers: Vec<McpServerInfo>,
    /// `claude_code_version`.
    pub version: Option<String>,
    /// The whole line.
    pub raw: Value,
}

/// A block of an assistant message.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    /// Text.
    Text(String),
    /// Thinking (hidden in the UI by default).
    Thinking(String),
    /// A tool call.
    ToolUse {
        /// `tool_use_id`; later `ToolResult`s and `parent_tool_use_id`s refer to it.
        id: String,
        /// Tool name (`Bash`, `Task`, `SendMessage`, `mcp__dovetail__...`).
        name: String,
        /// Tool input.
        input: Value,
    },
    /// Anything else, raw.
    Other(Value),
}

/// A complete assistant message (after its deltas).
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantMessage {
    /// Who produced it.
    pub origin: Origin,
    /// API message id (`msg_...`), when present.
    pub message_id: Option<String>,
    /// Line `uuid`.
    pub uuid: Option<String>,
    /// Content blocks in order.
    pub blocks: Vec<ContentBlock>,
    /// The model that wrote it (`message.model`), when present.
    pub model: Option<String>,
}

impl AssistantMessage {
    /// All text blocks joined with blank lines.
    pub fn text(&self) -> String {
        let parts: Vec<&str> = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        parts.join("\n\n")
    }
}

/// A tool result (one `tool_result` block of a `user` line).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// Who produced it.
    pub origin: Origin,
    /// The `tool_use_id` it answers.
    pub tool_use_id: String,
    /// Raw content: a string or an array of blocks.
    pub content: Value,
    /// `is_error`.
    pub is_error: bool,
    /// The line's `tool_use_result`: structured details of the result (for `SendMessage`, the
    /// `msg_id` the receiver's [`PeerMessage`] carries).
    pub meta: Option<Value>,
}

impl ToolResult {
    /// The content flattened to text (text blocks joined with newlines).
    pub fn content_text(&self) -> String {
        match &self.content {
            Value::String(s) => s.clone(),
            Value::Array(items) => items
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

/// A message another session sent with `SendMessage`, from a `user` line's `origin` object
/// (`{"kind": "peer", "name", "msg_id", "body", ...}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMessage {
    /// The sender's `--name`.
    pub name: String,
    /// Joins with the sender's `SendMessage` result.
    pub msg_id: Option<String>,
    /// The message text, unwrapped.
    pub body: String,
}

impl PeerMessage {
    /// Reads an `origin` object; `None` unless its `kind` is `peer`.
    pub fn from_origin(origin: &Value) -> Option<Self> {
        if origin.get("kind").and_then(Value::as_str) != Some("peer") {
            return None;
        }
        let text = |key: &str| origin.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            name: text("name").unwrap_or_default(),
            msg_id: text("msg_id"),
            body: text("body").unwrap_or_default(),
        })
    }
}

/// Which `system/task_*` event this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPhase {
    /// `task_started`.
    Started,
    /// `task_progress`.
    Progress,
    /// `task_updated` (a patch, e.g. `status: completed`).
    Updated,
    /// `task_notification` (final summary).
    Notification,
}

/// `system/task_*`: background subagents and background shell tasks (`task_type` tells which:
/// `local_agent`, `local_bash`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskEvent {
    /// Which of the four events.
    pub phase: TaskPhase,
    /// Task id.
    pub task_id: String,
    /// The `tool_use_id` that started it (matches `parent_tool_use_id` of the subagent's events).
    pub tool_use_id: Option<String>,
    /// `local_agent`, `local_bash`, ...
    pub task_type: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Subagent type, e.g. `general-purpose`.
    pub subagent_type: Option<String>,
    /// For tasks spawned by a subagent: the owning task.
    pub parent_task_id: Option<String>,
    /// `status` (notification) or `patch.status` (update).
    pub status: Option<String>,
    /// Summary (notification).
    pub summary: Option<String>,
    /// Output file (notification).
    pub output_file: Option<String>,
    /// Last tool used (progress).
    pub last_tool_name: Option<String>,
    /// `usage.total_tokens` (progress).
    pub total_tokens: Option<u64>,
    /// `usage.tool_uses` (progress).
    pub tool_uses: Option<u64>,
    /// `usage.duration_ms` (progress).
    pub duration_ms: Option<u64>,
    /// The whole line.
    pub raw: Value,
}

/// One entry of `background_tasks_changed`.
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundTask {
    /// Task id.
    pub task_id: String,
    /// `local_agent`, `local_bash`, ...
    pub task_type: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Subagent type.
    pub subagent_type: Option<String>,
}

/// Per-model usage from a `result`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelUsage {
    /// Model id.
    pub model: String,
    /// `contextWindow`: read the window from the stream, no model table needed.
    pub context_window: Option<u64>,
    /// `maxOutputTokens`.
    pub max_output_tokens: Option<u64>,
    /// `costUSD`.
    pub cost_usd: Option<f64>,
    /// `inputTokens`.
    pub input_tokens: u64,
    /// `outputTokens`.
    pub output_tokens: u64,
    /// `cacheReadInputTokens`.
    pub cache_read_tokens: u64,
    /// `cacheCreationInputTokens`.
    pub cache_creation_tokens: u64,
}

/// An entry of `permission_denials` in a `result`.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionDenial {
    /// Tool name.
    pub tool_name: String,
    /// Tool use id.
    pub tool_use_id: Option<String>,
    /// Tool input.
    pub tool_input: Value,
}

/// A `result` line. **Not the end of the session**: see the crate docs.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    /// `success` or `error_during_execution` (interrupt) etc.
    pub subtype: String,
    /// `is_error`.
    pub is_error: bool,
    /// Result text.
    pub result: Option<String>,
    /// Counts results in this process; use it rather than counting yourself.
    pub result_index: Option<u64>,
    /// `terminal_reason`, e.g. `aborted_streaming`.
    pub terminal_reason: Option<String>,
    /// `stop_reason`.
    pub stop_reason: Option<String>,
    /// `num_turns`.
    pub num_turns: Option<u64>,
    /// `duration_ms`.
    pub duration_ms: Option<u64>,
    /// `total_cost_usd`, falling back to the sum of per-model `costUSD`.
    pub total_cost_usd: Option<f64>,
    /// Per-model usage, from `modelUsage`, or `model_usage_since_last_result` on interrupted
    /// results.
    pub model_usage: Vec<ModelUsage>,
    /// Denials during the turn.
    pub permission_denials: Vec<PermissionDenial>,
    /// `queued_turn_count`.
    pub queued_turn_count: Option<u64>,
    /// The whole line.
    pub raw: Value,
}

impl TurnResult {
    /// The context window reported for the first model, if any.
    pub fn context_window(&self) -> Option<u64> {
        self.model_usage.iter().find_map(|m| m.context_window)
    }
}

/// A `can_use_tool` control request: the session is blocked on this tool call until answered
/// with [`crate::Session::answer_control`]. Other requests keep flowing meanwhile.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    /// Request id; pass it back in the answer.
    pub id: ControlId,
    /// Tool name.
    pub tool_name: String,
    /// `display_name`.
    pub display_name: Option<String>,
    /// Tool input.
    pub input: Value,
    /// Tool use id.
    pub tool_use_id: Option<String>,
    /// `description`.
    pub description: Option<String>,
    /// `decision_reason`.
    pub decision_reason: Option<String>,
    /// `decision_reason_type`.
    pub decision_reason_type: Option<String>,
    /// `permission_suggestions` (addRules entries); pass some back for "always allow".
    pub suggestions: Vec<Value>,
    /// `agent_id` when a subagent asks.
    pub agent_id: Option<String>,
    /// The whole control request line.
    pub raw: Value,
}

/// How the process ended.
#[derive(Debug, Clone, PartialEq)]
pub struct ExitInfo {
    /// Exit code, if it exited normally.
    pub code: Option<i32>,
    /// Signal number, if killed by a signal (9 after a kill or an OOM kill).
    pub signal: Option<i32>,
    /// True for exit code 0.
    pub success: bool,
    /// Last stderr lines (up to 20), useful for error inbox items.
    pub stderr_tail: Vec<String>,
}

/// `rate_limit_info.unifiedWindows` of a `rate_limit_event`. Limits are per account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RateLimitInfo {
    /// `five_hour`, the rolling 5-hour window.
    pub five_hour: Option<RateLimitWindow>,
    /// `seven_day`, the weekly window.
    pub seven_day: Option<RateLimitWindow>,
}

/// One window of [`RateLimitInfo`], as Claude Code reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitWindow {
    /// `utilization`, 0–1 (may exceed 1 when over the limit).
    pub utilization: f64,
    /// `resetsAt`, in Unix seconds.
    pub resets_at_s: Option<i64>,
    /// `status`, e.g. `allowed`; empty when missing.
    pub status: String,
}

/// Everything a session reports. Mapped from raw lines by [`crate::parse::map_value`].
#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    /// `system/init`.
    Init(InitInfo),
    /// `system/status`: request status and/or permission mode (also emitted after a
    /// `set_permission_mode`).
    Status {
        /// `status`, e.g. `requesting`.
        status: Option<String>,
        /// `permissionMode`.
        permission_mode: Option<String>,
    },
    /// `system/session_title_changed` (follows `--name`).
    SessionTitle {
        /// Title.
        title: String,
    },
    /// A text delta from `stream_event`.
    TextDelta {
        /// Who produced it.
        origin: Origin,
        /// API message id.
        message_id: Option<String>,
        /// The delta.
        text: String,
    },
    /// A thinking delta from `stream_event`.
    ThinkingDelta {
        /// Who produced it.
        origin: Origin,
        /// API message id.
        message_id: Option<String>,
        /// The delta.
        text: String,
    },
    /// A complete assistant message, with its text, thinking and tool-use blocks.
    Assistant(AssistantMessage),
    /// A tool result.
    ToolResult(ToolResult),
    /// A user message: one replayed by `--replay-user-messages` (our own prompt, a subagent's
    /// prompt, or a message from another session), or a text block next to tool results.
    UserMessage {
        /// Who it belongs to.
        origin: Origin,
        /// Line `uuid`.
        uuid: Option<String>,
        /// Text.
        text: String,
        /// `isReplay`.
        is_replay: bool,
        /// The line's `origin` object, present on messages from another session (spike S2);
        /// see [`PeerMessage::from_origin`].
        message_origin: Option<Value>,
        /// The whole line.
        raw: Value,
    },
    /// `system/task_*`.
    Task(TaskEvent),
    /// `system/background_tasks_changed`: the full list of running background tasks. An empty
    /// list means nothing is left in the background.
    BackgroundTasksChanged(Vec<BackgroundTask>),
    /// `result`. Never assume it ends the session.
    TurnResult(TurnResult),
    /// `can_use_tool` control request.
    PermissionRequest(PermissionRequest),
    /// The CLI withdrew a pending request (`control_cancel_request`).
    PermissionCancelled(ControlId),
    /// Any other control request from the CLI (hook callbacks, MCP messages): raw.
    ControlRequest {
        /// Request id.
        id: ControlId,
        /// The whole line.
        raw: Value,
    },
    /// The CLI's reply to one of our control requests (`interrupt`, `set_permission_mode`).
    ControlResponse {
        /// Our request id (`dt_N`).
        request_id: String,
        /// `subtype == "success"`.
        success: bool,
        /// The `response` payload (or error text).
        response: Value,
    },
    /// `system/permission_denied`: a request was auto-denied (no prompt tool, or classifier).
    PermissionDenied {
        /// Tool name.
        tool_name: Option<String>,
        /// Tool use id.
        tool_use_id: Option<String>,
        /// Message.
        message: Option<String>,
    },
    /// `rate_limit_event` (arrives every turn), with the windows of its `rate_limit_info`.
    RateLimit(RateLimitInfo),
    /// The process ended and was reaped. Always the last event.
    Exited(ExitInfo),
    /// A line whose type or subtype is not mapped (or was not JSON): never fatal.
    Unknown(Value),
}

impl DriverEvent {
    /// The origin, for events that have one.
    pub fn origin(&self) -> Option<&Origin> {
        match self {
            Self::TextDelta { origin, .. }
            | Self::ThinkingDelta { origin, .. }
            | Self::UserMessage { origin, .. } => Some(origin),
            Self::Assistant(m) => Some(&m.origin),
            Self::ToolResult(r) => Some(&r.origin),
            _ => None,
        }
    }
}

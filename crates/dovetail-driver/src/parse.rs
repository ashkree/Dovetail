//! Mapping from raw stream lines to [`DriverEvent`]s.
//!
//! Lenient by design: missing or oddly typed fields become `None`/defaults, unknown types become
//! [`DriverEvent::Unknown`]. Nothing here can panic or fail.
//!
//! Dropped on purpose (no event): `stream_event` kinds other than text and thinking deltas
//! (`message_start`, `content_block_start`, `input_json_delta`, ...), `system/thinking_tokens`
//! and `system/permission_check_status`. The complete `assistant` message carries everything.

use crate::event::*;
use serde_json::{Value, json};

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn u64_of(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn origin_of(v: &Value) -> Origin {
    Origin {
        parent_tool_use_id: str_of(v, "parent_tool_use_id"),
        agent_id: str_of(v, "agent_id"),
    }
}

/// Map one stdout line. Non-JSON lines become `Unknown({"_non_json": line})`. Empty lines map to
/// nothing.
pub fn map_line(line: &str) -> Vec<DriverEvent> {
    if line.trim().is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(line) {
        Ok(v) => map_value(v),
        Err(_) => vec![DriverEvent::Unknown(json!({ "_non_json": line }))],
    }
}

/// Map one parsed stdout line. One line can yield several events (a `user` line with several
/// tool results) or none (ignored kinds).
pub fn map_value(v: Value) -> Vec<DriverEvent> {
    let ty = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    match ty.as_str() {
        "system" => map_system(v),
        "stream_event" => map_stream_event(&v).into_iter().collect(),
        "assistant" => vec![map_assistant(&v)],
        "user" => map_user(&v),
        "result" => vec![DriverEvent::TurnResult(map_result(v))],
        "rate_limit_event" => vec![DriverEvent::RateLimit(map_rate_limit(&v))],
        "control_request" => vec![map_control_request(v)],
        "control_response" => {
            let r = v.get("response").cloned().unwrap_or(Value::Null);
            vec![DriverEvent::ControlResponse {
                request_id: str_of(&r, "request_id").unwrap_or_default(),
                success: r.get("subtype").and_then(Value::as_str) == Some("success"),
                response: r
                    .get("response")
                    .or_else(|| r.get("error"))
                    .cloned()
                    .unwrap_or(Value::Null),
            }]
        }
        "control_cancel_request" => match str_of(&v, "request_id") {
            Some(id) => vec![DriverEvent::PermissionCancelled(ControlId(id))],
            None => vec![DriverEvent::Unknown(v)],
        },
        _ => vec![DriverEvent::Unknown(v)],
    }
}

fn map_system(v: Value) -> Vec<DriverEvent> {
    let sub = v
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let ev = match sub.as_str() {
        "init" => DriverEvent::Init(InitInfo {
            session_id: str_of(&v, "session_id").unwrap_or_default(),
            model: str_of(&v, "model"),
            permission_mode: str_of(&v, "permissionMode"),
            cwd: str_of(&v, "cwd"),
            tools: v
                .get("tools")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            mcp_servers: v
                .get("mcp_servers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|s| McpServerInfo {
                            name: str_of(s, "name").unwrap_or_default(),
                            status: str_of(s, "status").unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            version: str_of(&v, "claude_code_version"),
            raw: v,
        }),
        "status" => DriverEvent::Status {
            status: str_of(&v, "status"),
            permission_mode: str_of(&v, "permissionMode"),
        },
        "session_title_changed" => match str_of(&v, "title") {
            Some(title) => DriverEvent::SessionTitle { title },
            None => DriverEvent::Unknown(v),
        },
        "task_started" | "task_progress" | "task_updated" | "task_notification" => {
            DriverEvent::Task(map_task(&sub, v))
        }
        "background_tasks_changed" => DriverEvent::BackgroundTasksChanged(
            v.get("tasks")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|t| BackgroundTask {
                            task_id: str_of(t, "task_id").unwrap_or_default(),
                            task_type: str_of(t, "task_type"),
                            description: str_of(t, "description"),
                            subagent_type: str_of(t, "subagent_type"),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        "permission_denied" => DriverEvent::PermissionDenied {
            tool_name: str_of(&v, "tool_name"),
            tool_use_id: str_of(&v, "tool_use_id"),
            message: str_of(&v, "message"),
        },
        "thinking_tokens" | "permission_check_status" => return Vec::new(),
        _ => DriverEvent::Unknown(v),
    };
    vec![ev]
}

fn map_task(sub: &str, v: Value) -> TaskEvent {
    let phase = match sub {
        "task_started" => TaskPhase::Started,
        "task_progress" => TaskPhase::Progress,
        "task_updated" => TaskPhase::Updated,
        _ => TaskPhase::Notification,
    };
    let usage = v.get("usage").cloned().unwrap_or(Value::Null);
    let status = str_of(&v, "status").or_else(|| {
        v.get("patch")
            .and_then(|p| p.get("status"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    TaskEvent {
        phase,
        task_id: str_of(&v, "task_id").unwrap_or_default(),
        tool_use_id: str_of(&v, "tool_use_id"),
        task_type: str_of(&v, "task_type"),
        description: str_of(&v, "description"),
        subagent_type: str_of(&v, "subagent_type"),
        parent_task_id: str_of(&v, "parent_task_id"),
        status,
        summary: str_of(&v, "summary"),
        output_file: str_of(&v, "output_file").filter(|s| !s.is_empty()),
        last_tool_name: str_of(&v, "last_tool_name"),
        total_tokens: u64_of(&usage, "total_tokens"),
        tool_uses: u64_of(&usage, "tool_uses"),
        duration_ms: u64_of(&usage, "duration_ms"),
        raw: v,
    }
}

fn map_stream_event(v: &Value) -> Option<DriverEvent> {
    let ev = v.get("event")?;
    if ev.get("type").and_then(Value::as_str) != Some("content_block_delta") {
        return None;
    }
    let delta = ev.get("delta")?;
    let origin = origin_of(v);
    let message_id = str_of(v, "api_message_id");
    match delta.get("type").and_then(Value::as_str)? {
        "text_delta" => Some(DriverEvent::TextDelta {
            origin,
            message_id,
            text: str_of(delta, "text")?,
        }),
        "thinking_delta" => Some(DriverEvent::ThinkingDelta {
            origin,
            message_id,
            text: str_of(delta, "thinking")?,
        }),
        _ => None,
    }
}

fn map_block(b: &Value) -> ContentBlock {
    match b.get("type").and_then(Value::as_str) {
        Some("text") => ContentBlock::Text(str_of(b, "text").unwrap_or_default()),
        Some("thinking") => ContentBlock::Thinking(str_of(b, "thinking").unwrap_or_default()),
        Some("tool_use") => ContentBlock::ToolUse {
            id: str_of(b, "id").unwrap_or_default(),
            name: str_of(b, "name").unwrap_or_default(),
            input: b.get("input").cloned().unwrap_or(Value::Null),
        },
        _ => ContentBlock::Other(b.clone()),
    }
}

fn map_assistant(v: &Value) -> DriverEvent {
    let msg = v.get("message").unwrap_or(&Value::Null);
    let blocks = match msg.get("content") {
        Some(Value::Array(a)) => a.iter().map(map_block).collect(),
        Some(Value::String(s)) => vec![ContentBlock::Text(s.clone())],
        _ => Vec::new(),
    };
    DriverEvent::Assistant(AssistantMessage {
        origin: origin_of(v),
        message_id: str_of(msg, "id"),
        uuid: str_of(v, "uuid"),
        blocks,
        model: str_of(msg, "model"),
        usage: msg.get("usage").filter(|u| u.is_object()).map(|u| TokenUsage {
            input_tokens: u64_of(u, "input_tokens").unwrap_or(0),
            cache_read_tokens: u64_of(u, "cache_read_input_tokens").unwrap_or(0),
            cache_creation_tokens: u64_of(u, "cache_creation_input_tokens").unwrap_or(0),
            output_tokens: u64_of(u, "output_tokens").unwrap_or(0),
        }),
    })
}

fn map_user(v: &Value) -> Vec<DriverEvent> {
    let origin = origin_of(v);
    let uuid = str_of(v, "uuid");
    let is_replay = v.get("isReplay").and_then(Value::as_bool).unwrap_or(false);
    let message_origin = v.get("origin").filter(|o| o.is_object()).cloned();
    let meta = v.get("tool_use_result").cloned();
    let user = |text: String| DriverEvent::UserMessage {
        origin: origin.clone(),
        uuid: uuid.clone(),
        text,
        is_replay,
        message_origin: message_origin.clone(),
        raw: v.clone(),
    };
    let content = v.get("message").and_then(|m| m.get("content"));
    match content {
        Some(Value::String(s)) => vec![user(s.clone())],
        Some(Value::Array(blocks)) => {
            let mut out = Vec::new();
            let mut text = Vec::new();
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("tool_result") => out.push(DriverEvent::ToolResult(ToolResult {
                        origin: origin.clone(),
                        tool_use_id: str_of(b, "tool_use_id").unwrap_or_default(),
                        content: b.get("content").cloned().unwrap_or(Value::Null),
                        is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                        meta: meta.clone(),
                    })),
                    Some("text") => text.extend(str_of(b, "text")),
                    _ => {}
                }
            }
            if !text.is_empty() {
                out.push(user(text.join("\n\n")));
            }
            if out.is_empty() {
                out.push(DriverEvent::Unknown(v.clone()));
            }
            out
        }
        _ => vec![DriverEvent::Unknown(v.clone())],
    }
}

fn map_usage(obj: &Value) -> Vec<ModelUsage> {
    let Some(map) = obj.as_object() else {
        return Vec::new();
    };
    map.iter()
        .map(|(model, u)| ModelUsage {
            model: model.clone(),
            context_window: u64_of(u, "contextWindow"),
            max_output_tokens: u64_of(u, "maxOutputTokens"),
            cost_usd: u.get("costUSD").and_then(Value::as_f64),
            input_tokens: u64_of(u, "inputTokens").unwrap_or(0),
            output_tokens: u64_of(u, "outputTokens").unwrap_or(0),
            cache_read_tokens: u64_of(u, "cacheReadInputTokens").unwrap_or(0),
            cache_creation_tokens: u64_of(u, "cacheCreationInputTokens").unwrap_or(0),
        })
        .collect()
}

/// `rate_limit_info.unifiedWindows` of a `rate_limit_event`. Either window may be missing:
/// Claude Code 2.1.319 sends only `seven_day` at times (task #26). A window without a numeric
/// `utilization` is left out, with a warning.
fn map_rate_limit(v: &Value) -> RateLimitInfo {
    let windows = v
        .get("rate_limit_info")
        .and_then(|i| i.get("unifiedWindows"));
    let window = |key: &str| {
        let w = windows?.get(key)?;
        let Some(utilization) = w.get("utilization").and_then(Value::as_f64) else {
            tracing::warn!("rate_limit_event window {key} has no numeric utilization: {w}");
            return None;
        };
        Some(RateLimitWindow {
            utilization,
            resets_at_s: w.get("resetsAt").and_then(Value::as_i64),
            status: str_of(w, "status").unwrap_or_default(),
        })
    };
    RateLimitInfo {
        five_hour: window("five_hour"),
        seven_day: window("seven_day"),
    }
}

fn map_result(v: Value) -> TurnResult {
    let mut model_usage = v.get("modelUsage").map(map_usage).unwrap_or_default();
    if model_usage.is_empty() {
        // Interrupted results carry the usage under another key.
        model_usage = v
            .get("model_usage_since_last_result")
            .map(map_usage)
            .unwrap_or_default();
    }
    let total_cost_usd = v.get("total_cost_usd").and_then(Value::as_f64).or_else(|| {
        let costs: Vec<f64> = model_usage.iter().filter_map(|m| m.cost_usd).collect();
        (!costs.is_empty()).then(|| costs.iter().sum())
    });
    let permission_denials = v
        .get("permission_denials")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|d| PermissionDenial {
                    tool_name: str_of(d, "tool_name").unwrap_or_default(),
                    tool_use_id: str_of(d, "tool_use_id"),
                    tool_input: d.get("tool_input").cloned().unwrap_or(Value::Null),
                })
                .collect()
        })
        .unwrap_or_default();
    TurnResult {
        subtype: str_of(&v, "subtype").unwrap_or_default(),
        is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        result: str_of(&v, "result"),
        result_index: u64_of(&v, "result_index"),
        terminal_reason: str_of(&v, "terminal_reason"),
        stop_reason: str_of(&v, "stop_reason"),
        num_turns: u64_of(&v, "num_turns"),
        duration_ms: u64_of(&v, "duration_ms"),
        total_cost_usd,
        model_usage,
        permission_denials,
        queued_turn_count: u64_of(&v, "queued_turn_count"),
        raw: v,
    }
}

fn map_control_request(v: Value) -> DriverEvent {
    let id = ControlId(str_of(&v, "request_id").unwrap_or_default());
    let req = v.get("request").cloned().unwrap_or(Value::Null);
    if req.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
        return DriverEvent::ControlRequest { id, raw: v };
    }
    DriverEvent::PermissionRequest(PermissionRequest {
        id,
        tool_name: str_of(&req, "tool_name").unwrap_or_default(),
        display_name: str_of(&req, "display_name"),
        input: req.get("input").cloned().unwrap_or(Value::Null),
        tool_use_id: str_of(&req, "tool_use_id"),
        description: str_of(&req, "description"),
        decision_reason: str_of(&req, "decision_reason"),
        decision_reason_type: str_of(&req, "decision_reason_type"),
        suggestions: req
            .get("permission_suggestions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        agent_id: str_of(&req, "agent_id"),
        raw: v,
    })
}

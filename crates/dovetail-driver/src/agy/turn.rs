//! One turn's `session/update` notifications as [`DriverEvent`]s.
//!
//! ACP streams chunks and never sends a complete message, while the core expects deltas followed
//! by the [`AssistantMessage`] that confirms them. So the text and thinking chunks are gathered
//! under a message id of our own (`agy-<n>`), and the message is closed when a tool call starts
//! or the turn ends. The answer to `session/prompt` ends the turn as a [`TurnResult`].

use super::args;
use crate::event::*;
use serde_json::Value;

/// The turn in progress.
#[derive(Debug, Default)]
pub(crate) struct Turn {
    /// Numbers our message ids.
    messages: u64,
    text: String,
    thinking: String,
    /// All of this turn's text, for the result.
    reply: String,
    /// The latest `usage_update`: context tokens used and the window size.
    used: Option<u64>,
    size: Option<u64>,
    /// Results so far in this process (rule 2's `result_index`).
    results: u64,
}

fn text_of(update: &Value) -> String {
    update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A finished tool call's output as text: its content blocks, else its raw output.
fn tool_output(update: &Value) -> Value {
    let texts: Vec<&str> = update
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.pointer("/content/text").and_then(Value::as_str))
        .collect();
    if !texts.is_empty() {
        return Value::String(texts.join("\n"));
    }
    match update.get("rawOutput") {
        Some(Value::Object(o)) => o
            .get("output")
            .cloned()
            .unwrap_or_else(|| Value::Object(o.clone())),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

impl Turn {
    fn message_id(&self) -> Option<String> {
        Some(format!("agy-{}", self.messages))
    }

    /// Closes the message in progress: the confirmed text and thinking, with `usage` when the
    /// turn ends. `None` when there is nothing to confirm.
    fn close_message(
        &mut self,
        model: Option<&str>,
        usage: Option<TokenUsage>,
    ) -> Option<DriverEvent> {
        if self.text.is_empty() && self.thinking.is_empty() && usage.is_none() {
            return None;
        }
        let mut blocks = Vec::new();
        if !self.thinking.is_empty() {
            blocks.push(ContentBlock::Thinking(std::mem::take(&mut self.thinking)));
        }
        if !self.text.is_empty() {
            blocks.push(ContentBlock::Text(std::mem::take(&mut self.text)));
        }
        let event = DriverEvent::Assistant(AssistantMessage {
            origin: Origin::default(),
            message_id: self.message_id(),
            uuid: None,
            blocks,
            model: model.map(str::to_owned),
            usage,
        });
        self.messages += 1;
        Some(event)
    }

    /// One `session/update`'s `update` object. `model` is the agy id in use.
    pub(crate) fn on_update(&mut self, update: &Value, model: Option<&str>) -> Vec<DriverEvent> {
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "agent_message_chunk" => {
                let text = text_of(update);
                self.text.push_str(&text);
                self.reply.push_str(&text);
                vec![DriverEvent::TextDelta {
                    origin: Origin::default(),
                    message_id: self.message_id(),
                    text,
                }]
            }
            "agent_thought_chunk" => {
                let text = text_of(update);
                self.thinking.push_str(&text);
                vec![DriverEvent::ThinkingDelta {
                    origin: Origin::default(),
                    message_id: self.message_id(),
                    text,
                }]
            }
            // A resume replays history with these; Dovetail has its own record.
            "user_message_chunk" => Vec::new(),
            "tool_call" => {
                let mut events: Vec<_> = self.close_message(model, None).into_iter().collect();
                events.push(DriverEvent::Assistant(AssistantMessage {
                    origin: Origin::default(),
                    message_id: None,
                    uuid: None,
                    blocks: vec![ContentBlock::ToolUse {
                        id: str_of(update, "toolCallId").unwrap_or_default(),
                        name: str_of(update, "name").unwrap_or_default(),
                        input: update
                            .get("rawInput")
                            .or_else(|| update.get("args"))
                            .cloned()
                            .unwrap_or_default(),
                    }],
                    model: model.map(str::to_owned),
                    usage: None,
                }));
                events
            }
            "tool_call_update" => match update.get("status").and_then(Value::as_str) {
                Some(status @ ("completed" | "failed")) => {
                    vec![DriverEvent::ToolResult(ToolResult {
                        origin: Origin::default(),
                        tool_use_id: str_of(update, "toolCallId").unwrap_or_default(),
                        content: tool_output(update),
                        is_error: status == "failed",
                        meta: None,
                    })]
                }
                _ => Vec::new(),
            },
            "usage_update" => {
                self.used = update.get("used").and_then(Value::as_u64).or(self.used);
                self.size = update.get("size").and_then(Value::as_u64).or(self.size);
                Vec::new()
            }
            "available_commands_update" => vec![DriverEvent::Commands(commands_of(update))],
            "session_info_update" => str_of(update, "title")
                .map(|title| DriverEvent::SessionTitle { title })
                .into_iter()
                .collect(),
            "current_mode_update" => vec![DriverEvent::Status {
                status: None,
                permission_mode: update
                    .get("currentModeId")
                    .and_then(Value::as_str)
                    .and_then(args::mode_as_cli)
                    .map(str::to_owned),
            }],
            _ => vec![DriverEvent::Unknown(update.clone())],
        }
    }

    /// The `session/prompt` answer (`msg` is the whole line): the last message, carrying the
    /// context the session holds now, then the result.
    pub(crate) fn end(
        &mut self,
        msg: &Value,
        model: Option<&str>,
        queued: usize,
    ) -> Vec<DriverEvent> {
        let usage = self.used.map(|used| TokenUsage {
            input_tokens: used,
            ..TokenUsage::default()
        });
        let mut events: Vec<_> = self.close_message(model, usage).into_iter().collect();
        self.results += 1;
        let answer = msg.get("result");
        let error = msg.get("error").map(|_| super::rpc::error_text(msg));
        let stop = answer.and_then(|r| str_of(r, "stopReason"));
        let tokens = |key: &str| {
            answer
                .and_then(|r| r.pointer(&format!("/usage/{key}")))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let subtype = match (stop.as_deref(), &error) {
            (_, Some(_)) => "error",
            // As Claude Code names an interrupted turn.
            (Some("cancelled"), _) => "error_during_execution",
            _ => "success",
        };
        let reply = std::mem::take(&mut self.reply);
        events.push(DriverEvent::TurnResult(TurnResult {
            subtype: subtype.to_owned(),
            is_error: error.is_some() || matches!(stop.as_deref(), Some("refusal")),
            result: error.or(Some(reply)),
            result_index: Some(self.results),
            terminal_reason: None,
            stop_reason: stop,
            num_turns: None,
            duration_ms: None,
            total_cost_usd: None,
            model_usage: vec![ModelUsage {
                model: model.unwrap_or_default().to_owned(),
                context_window: self.size,
                max_output_tokens: None,
                cost_usd: None,
                input_tokens: tokens("inputTokens"),
                output_tokens: tokens("outputTokens"),
                cache_read_tokens: tokens("cachedReadTokens"),
                cache_creation_tokens: tokens("cachedWriteTokens"),
            }],
            permission_denials: Vec::new(),
            queued_turn_count: Some(queued as u64),
            raw: msg.clone(),
        }));
        events
    }
}

/// `available_commands_update`'s commands.
fn commands_of(update: &Value) -> Vec<CommandInfo> {
    update
        .get("availableCommands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(CommandInfo {
                name: str_of(c, "name")?,
                description: str_of(c, "description").unwrap_or_default(),
                argument_hint: c
                    .pointer("/input/hint")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MODEL: Option<&str> = Some("gemini-3.8-flash-low");

    fn chunk(kind: &str, text: &str) -> Value {
        json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}})
    }

    #[test]
    fn chunks_stream_then_a_tool_call_confirms_the_message() {
        let mut turn = Turn::default();
        let delta = turn.on_update(&chunk("agent_message_chunk", "Let me look"), MODEL);
        assert!(matches!(
            &delta[..],
            [DriverEvent::TextDelta { message_id: Some(id), text, .. }] if id == "agy-0" && text == "Let me look"
        ));
        let events = turn.on_update(
            &json!({"sessionUpdate": "tool_call", "toolCallId": "call_1", "name": "run_command",
                    "rawInput": {"CommandLine": "ls"}, "status": "in_progress"}),
            MODEL,
        );
        let [DriverEvent::Assistant(text), DriverEvent::Assistant(tool)] = &events[..] else {
            panic!("{events:?}");
        };
        assert_eq!(text.message_id.as_deref(), Some("agy-0"));
        assert_eq!(text.text(), "Let me look");
        assert_eq!(
            tool.blocks,
            [ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "run_command".into(),
                input: json!({"CommandLine": "ls"}),
            }]
        );
        // The next text is a new message.
        let delta = turn.on_update(&chunk("agent_message_chunk", "Done"), MODEL);
        assert!(matches!(
            &delta[..],
            [DriverEvent::TextDelta { message_id: Some(id), .. }] if id == "agy-1"
        ));
    }

    #[test]
    fn a_finished_tool_call_is_its_result() {
        let mut turn = Turn::default();
        let events = turn.on_update(
            &json!({"sessionUpdate": "tool_call_update", "toolCallId": "call_1", "status": "completed",
                    "content": [{"type": "content", "content": {"type": "text", "text": "a.txt"}}]}),
            MODEL,
        );
        let [DriverEvent::ToolResult(result)] = &events[..] else {
            panic!("{events:?}");
        };
        assert_eq!(result.tool_use_id, "call_1");
        assert_eq!(result.content_text(), "a.txt");
        assert!(!result.is_error);
        let pending = json!({"sessionUpdate": "tool_call_update", "toolCallId": "call_1", "status": "pending"});
        assert!(turn.on_update(&pending, MODEL).is_empty());
        let failed =
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c", "status": "failed"});
        assert!(
            matches!(&turn.on_update(&failed, MODEL)[..], [DriverEvent::ToolResult(r)] if r.is_error)
        );
    }

    #[test]
    fn the_prompt_answer_ends_the_turn_with_usage_and_window() {
        let mut turn = Turn::default();
        turn.on_update(&chunk("agent_thought_chunk", "hmm"), MODEL);
        turn.on_update(&chunk("agent_message_chunk", "pong"), MODEL);
        turn.on_update(
            &json!({"sessionUpdate": "usage_update", "used": 8527, "size": 1048576}),
            MODEL,
        );
        let answer = json!({"jsonrpc": "2.0", "id": 4, "result": {"stopReason": "end_turn",
            "usage": {"inputTokens": 8526, "outputTokens": 1, "cachedReadTokens": 5, "cachedWriteTokens": 0}}});
        let events = turn.end(&answer, MODEL, 0);
        let [
            DriverEvent::Assistant(message),
            DriverEvent::TurnResult(result),
        ] = &events[..]
        else {
            panic!("{events:?}");
        };
        assert_eq!(message.blocks[0], ContentBlock::Thinking("hmm".into()));
        assert_eq!(message.text(), "pong");
        assert_eq!(message.usage.map(|u| u.context_tokens()), Some(8527));
        assert_eq!(result.subtype, "success");
        assert_eq!(result.result.as_deref(), Some("pong"));
        assert_eq!(result.context_window(), Some(1_048_576));
        assert_eq!(result.model_usage[0].input_tokens, 8526);
        assert_eq!(result.result_index, Some(1));
    }

    #[test]
    fn a_cancelled_turn_reads_as_interrupted_and_an_error_as_an_error() {
        let mut turn = Turn::default();
        let cancelled = json!({"id": 4, "result": {"stopReason": "cancelled"}});
        let events = turn.end(&cancelled, MODEL, 0);
        assert!(
            matches!(events.last(), Some(DriverEvent::TurnResult(r)) if r.subtype == "error_during_execution" && !r.is_error)
        );
        let failed = json!({"id": 5, "error": {"code": -32603, "message": "quota exhausted"}});
        let events = turn.end(&failed, MODEL, 1);
        let Some(DriverEvent::TurnResult(r)) = events.last() else {
            panic!("{events:?}");
        };
        assert!(r.is_error);
        assert_eq!(r.result.as_deref(), Some("quota exhausted"));
        assert_eq!(r.result_index, Some(2));
    }

    #[test]
    fn session_notices_map_to_their_events() {
        let mut turn = Turn::default();
        let events = turn.on_update(
            &json!({"sessionUpdate": "available_commands_update",
                    "availableCommands": [{"name": "compact", "description": "Compact now"}]}),
            MODEL,
        );
        assert!(matches!(&events[..], [DriverEvent::Commands(c)] if c[0].name == "compact"));
        let events = turn.on_update(
            &json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"}),
            MODEL,
        );
        assert!(
            matches!(&events[..], [DriverEvent::Status { permission_mode: Some(m), .. }] if m == "plan")
        );
        let events = turn.on_update(
            &json!({"sessionUpdate": "session_info_update", "title": "T"}),
            MODEL,
        );
        assert!(matches!(&events[..], [DriverEvent::SessionTitle { title }] if title == "T"));
        let events = turn.on_update(&json!({"sessionUpdate": "plan", "entries": []}), MODEL);
        assert!(matches!(&events[..], [DriverEvent::Unknown(_)]));
    }
}

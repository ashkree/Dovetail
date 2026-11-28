//! The MCP menu's control requests (task #44; `docs/spikes/mcp-headless.md`). Each
//! [`McpRequest`] is one control request on the channel the driver already writes; its answer
//! comes back as [`DriverEvent::McpReply`] carrying the request, so the core never needs an id.
//!
//! Claude Code answers control requests out of order, so every request has its own id and the
//! menu never relies on answer order. A refusal is a normal [`McpOutcome::Failed`]: the subtypes
//! are not documented and may change between versions.

use std::collections::HashMap;

use dovetail_api::agents::mcp::{McpOutcome, McpRequest, McpServer, McpStatus, McpTool};
use serde_json::{Value, json};

use crate::event::DriverEvent;

/// The requests in flight on one session, keyed by request id.
#[derive(Debug, Default)]
pub(crate) struct McpCalls {
    next: u64,
    pending: HashMap<String, McpRequest>,
}

impl McpCalls {
    /// The request id and the line to write for `request`.
    pub(crate) fn begin(&mut self, request: McpRequest) -> (String, Value) {
        let id = format!("dt_mcp_{}", self.next);
        self.next += 1;
        let line = json!({
            "type": "control_request",
            "request_id": id,
            "request": request_body(&request),
        });
        self.pending.insert(id.clone(), request);
        (id, line)
    }

    /// Ends request `id` with a failure (its write failed, it timed out); `None` when it was
    /// already answered.
    pub(crate) fn fail(&mut self, id: &str, why: &str) -> Option<DriverEvent> {
        let request = self.pending.remove(id)?;
        Some(failed(request, why))
    }

    /// Ends every request in flight with a failure: the session is gone.
    pub(crate) fn fail_all(&mut self, why: &str) -> Vec<DriverEvent> {
        self.pending
            .drain()
            .map(|(_, request)| failed(request, why))
            .collect()
    }

    /// The event for the answer to request `id`; `None` when it is not an MCP request.
    pub(crate) fn answer(&mut self, id: &str, success: bool, body: &Value) -> Option<DriverEvent> {
        let request = self.pending.remove(id)?;
        let outcome = if success {
            outcome_of(&request, body)
        } else {
            McpOutcome::Failed(error_text(body))
        };
        Some(DriverEvent::McpReply { request, outcome })
    }
}

fn failed(request: McpRequest, why: &str) -> DriverEvent {
    DriverEvent::McpReply {
        request,
        outcome: McpOutcome::Failed(why.to_owned()),
    }
}

/// The `request` object of the control request for `request`.
pub(crate) fn request_body(request: &McpRequest) -> Value {
    match request {
        McpRequest::Status => json!({"subtype": "mcp_status"}),
        McpRequest::Reconnect { server } => {
            json!({"subtype": "mcp_reconnect", "serverName": server})
        }
        McpRequest::Toggle { server, enabled } => {
            json!({"subtype": "mcp_toggle", "serverName": server, "enabled": enabled})
        }
        McpRequest::Authenticate {
            server,
            redirect_uri,
        } => json!({
            "subtype": "mcp_authenticate",
            "serverName": server,
            "redirectUri": redirect_uri,
            "redirectUriOnly": true,
        }),
        McpRequest::OAuthCallback { server, url } => {
            json!({"subtype": "mcp_oauth_callback_url", "serverName": server, "callbackUrl": url})
        }
        McpRequest::ClearAuth { server } => {
            json!({"subtype": "mcp_clear_auth", "serverName": server})
        }
    }
}

fn error_text(body: &Value) -> String {
    match body {
        Value::String(s) => s.clone(),
        Value::Null => "no reason given".into(),
        other => other.to_string(),
    }
}

fn outcome_of(request: &McpRequest, body: &Value) -> McpOutcome {
    match request {
        McpRequest::Status => McpOutcome::Status(servers_of(body)),
        McpRequest::Authenticate { .. } => match body.get("authUrl").and_then(Value::as_str) {
            Some(url) => McpOutcome::AuthUrl {
                url: url.to_owned(),
                callback_expected: body
                    .get("callbackExpected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            // Already logged in, or a flow that needed no browser.
            None => McpOutcome::Done,
        },
        _ => McpOutcome::Done,
    }
}

/// The servers in an `mcp_status` answer's payload (`mcpServers`).
pub(crate) fn servers_of(body: &Value) -> Vec<McpServer> {
    let Some(list) = body.get("mcpServers").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter().filter_map(server_of).collect()
}

fn server_of(v: &Value) -> Option<McpServer> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let config = v.get("config");
    let config_text = |key: &str| config?.get(key)?.as_str().map(str::to_owned);
    let transport = text("transport")
        .or_else(|| config_text("type"))
        .unwrap_or_default();
    let target = match config_text("command") {
        Some(command) => {
            let args = config
                .and_then(|c| c.get("args"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            std::iter::once(command.as_str())
                .chain(args)
                .collect::<Vec<_>>()
                .join(" ")
        }
        None => config_text("url").unwrap_or_default(),
    };
    let server_info = v.get("serverInfo").and_then(|i| {
        let name = i.get("name")?.as_str()?;
        Some(match i.get("version").and_then(Value::as_str) {
            Some(version) => format!("{name} {version}"),
            None => name.to_owned(),
        })
    });
    let tools = v
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| {
                    let flag = |key: &str| {
                        t.pointer(&format!("/annotations/{key}"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    };
                    Some(McpTool {
                        name: t.get("name")?.as_str()?.to_owned(),
                        read_only: flag("readOnly"),
                        destructive: flag("destructive"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(McpServer {
        name: text("name")?,
        status: McpStatus::parse(&text("status")?),
        error: text("error"),
        server_info,
        transport,
        target,
        scope: text("scope").unwrap_or_default(),
        tools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_of(line: &Value) -> String {
        line["request_id"].as_str().unwrap_or_default().to_owned()
    }

    #[test]
    fn every_request_is_the_spikes_control_request() {
        let s = |n: &str| n.to_owned();
        let cases = [
            (McpRequest::Status, json!({"subtype": "mcp_status"})),
            (
                McpRequest::Reconnect { server: s("a") },
                json!({"subtype": "mcp_reconnect", "serverName": "a"}),
            ),
            (
                McpRequest::Toggle {
                    server: s("a"),
                    enabled: false,
                },
                json!({"subtype": "mcp_toggle", "serverName": "a", "enabled": false}),
            ),
            (
                McpRequest::Authenticate {
                    server: s("a"),
                    redirect_uri: s("http://localhost:1/cb"),
                },
                json!({"subtype": "mcp_authenticate", "serverName": "a",
                       "redirectUri": "http://localhost:1/cb", "redirectUriOnly": true}),
            ),
            (
                McpRequest::OAuthCallback {
                    server: s("a"),
                    url: s("http://localhost:1/cb?code=x"),
                },
                json!({"subtype": "mcp_oauth_callback_url", "serverName": "a",
                       "callbackUrl": "http://localhost:1/cb?code=x"}),
            ),
            (
                McpRequest::ClearAuth { server: s("a") },
                json!({"subtype": "mcp_clear_auth", "serverName": "a"}),
            ),
        ];
        let mut calls = McpCalls::default();
        for (request, body) in cases {
            let (_, line) = calls.begin(request);
            assert_eq!(line["type"], "control_request");
            assert_eq!(line["request"], body);
        }
    }

    #[test]
    fn a_failed_write_a_timeout_and_a_session_exit_each_clear_the_entry_with_a_failure() {
        let mut calls = McpCalls::default();
        let (a, _) = calls.begin(McpRequest::Status);
        let reconnect = McpRequest::Reconnect { server: "x".into() };
        let (b, _) = calls.begin(reconnect.clone());
        let (c, _) = calls.begin(McpRequest::ClearAuth { server: "y".into() });
        // A write that failed, or a timeout: the one request fails, once.
        assert_eq!(
            calls.fail(&a, "could not write"),
            Some(DriverEvent::McpReply {
                request: McpRequest::Status,
                outcome: McpOutcome::Failed("could not write".into()),
            })
        );
        assert_eq!(calls.fail(&a, "timed out"), None, "already cleared");
        // The late answer of a timed-out request is not ours any more.
        assert_eq!(calls.answer(&a, true, &Value::Null), None);
        // The session ends: everything still in flight fails.
        let events = calls.fail_all("the session ended");
        assert_eq!(events.len(), 2);
        assert!(events.contains(&DriverEvent::McpReply {
            request: reconnect,
            outcome: McpOutcome::Failed("the session ended".into()),
        }));
        assert_eq!(calls.fail(&b, "x"), None);
        assert_eq!(calls.fail(&c, "x"), None);
        assert!(calls.fail_all("again").is_empty());
    }

    #[test]
    fn status_parses_servers_tools_and_errors() {
        let body = json!({"mcpServers": [
            {"name": "good", "status": "connected", "serverInfo": {"name": "spike", "version": "0"},
             "config": {"type": "stdio", "command": "python3", "args": ["server.py", "-v"]},
             "transport": "stdio", "scope": "dynamic",
             "tools": [{"name": "ping", "annotations": {}},
                       {"name": "read", "annotations": {"readOnly": true}},
                       {"name": "delete", "annotations": {"destructive": true}}]},
            {"name": "broken", "status": "failed", "error": "ENOENT",
             "config": {"type": "stdio", "command": "/nonexistent", "args": []}, "transport": "stdio"},
            {"name": "claude.ai Docs", "status": "needs-auth", "transport": "claudeai-proxy",
             "config": {"type": "claudeai-proxy", "url": "https://x/y", "id": "mcpsrv_1"}, "scope": "claudeai"},
            {"name": "off", "status": "disabled"},
            {"status": "connected"},
        ]});
        let servers = servers_of(&body);
        assert_eq!(servers.len(), 4, "the nameless entry is skipped");
        let good = &servers[0];
        assert_eq!(good.status, McpStatus::Connected);
        assert_eq!(good.server_info.as_deref(), Some("spike 0"));
        assert_eq!(good.target, "python3 server.py -v");
        assert_eq!(good.tools.len(), 3);
        assert!(good.tools[1].read_only && !good.tools[1].destructive);
        assert!(good.tools[2].destructive);
        assert_eq!(servers[1].error.as_deref(), Some("ENOENT"));
        assert_eq!(servers[1].target, "/nonexistent");
        assert_eq!(servers[2].status, McpStatus::NeedsAuth);
        assert_eq!(servers[2].target, "https://x/y");
        assert!(servers[2].is_claude_ai());
        assert_eq!(servers[3].status, McpStatus::Disabled);
        assert!(servers[3].tools.is_empty());
        assert!(servers_of(&json!({})).is_empty());
    }

    #[test]
    fn an_answer_carries_its_request_and_the_right_outcome() {
        let mut calls = McpCalls::default();
        let (_, status) = calls.begin(McpRequest::Status);
        let (_, auth) = calls.begin(McpRequest::Authenticate {
            server: "a".into(),
            redirect_uri: "u".into(),
        });
        let (_, toggle) = calls.begin(McpRequest::Toggle {
            server: "a".into(),
            enabled: true,
        });
        // Out of order, as Claude Code answers.
        let ev = calls.answer(&id_of(&toggle), false, &json!("Server not found: a"));
        assert_eq!(
            ev,
            Some(DriverEvent::McpReply {
                request: McpRequest::Toggle {
                    server: "a".into(),
                    enabled: true
                },
                outcome: McpOutcome::Failed("Server not found: a".into()),
            })
        );
        let ev = calls.answer(
            &id_of(&auth),
            true,
            &json!({"authUrl": "https://login", "requiresUserAction": true, "callbackExpected": false}),
        );
        assert!(matches!(
            ev,
            Some(DriverEvent::McpReply {
                outcome: McpOutcome::AuthUrl { ref url, callback_expected: false },
                ..
            }) if url == "https://login"
        ));
        let ev = calls.answer(&id_of(&status), true, &json!({"mcpServers": []}));
        assert_eq!(
            ev,
            Some(DriverEvent::McpReply {
                request: McpRequest::Status,
                outcome: McpOutcome::Status(Vec::new()),
            })
        );
        assert_eq!(calls.answer("dt_3", true, &Value::Null), None);
        assert_eq!(calls.answer(&id_of(&status), true, &Value::Null), None);
    }
}

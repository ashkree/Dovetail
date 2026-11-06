//! Needs you: raising inbox items and acting on their answers (M7, M9).
//!
//! Each item remembers what waits on it ([`Waiter`]): a held permission request in a session, a
//! held `ask_user` call, a `start_slice` call waiting for approval, or a worktree removal waiting
//! for confirmation. Answers go straight to whatever waits, so one held item never blocks
//! another (stream rule 3).

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::{
    AgentId, Event, InboxId, InboxItem, InboxKind, PermissionAnswer, RuleSuggestion,
};
use dovetail_driver::{ControlAnswer, ControlId, PermissionRequest};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::db::now_ms;
use crate::error::SessionError;
use crate::service::{Core, InboxEntry};
use crate::slice::{SliceRequest, SliceStart};

/// What waits on an inbox item's answer.
pub(crate) enum Waiter {
    /// Nothing: a finished turn or an error, only acknowledged.
    None,
    /// A `can_use_tool` request held by the session.
    Control {
        control: ControlId,
        /// The rules Claude Code offered, as sent, for "always allow".
        suggestions: Vec<Value>,
    },
    /// A blocking `ask_user` call, held until the answer.
    BlockingQuestion {
        reply: oneshot::Sender<SocketResponseBody>,
    },
    /// A non-blocking `ask_user`: the answer goes to the session as a message.
    Question { question: String },
    /// A `start_slice` waiting for approval; the call already returned `pending`, and the
    /// answer goes to the coordinator as a message (ADR 0004).
    StartSlice { request: SliceRequest },
    /// A slice archive waiting for confirmation that unsaved work may go.
    RemoveWorktree,
    /// An `ask_user` question from before a restart: its call is gone, so the answer goes to
    /// the agent as a normal message (M10, decided 2026-11-06).
    RestoredQuestion { question: String },
    /// A worktree no agent owns, found by the sweep on open (M10): `y` removes it, `d` keeps
    /// it for good.
    RemoveOrphan { found: crate::restore::Orphan },
}

impl Waiter {
    /// Whether the agent is held until the answer (it shows as needing you).
    pub(crate) fn holds_agent(&self) -> bool {
        matches!(
            self,
            Waiter::Control { .. } | Waiter::BlockingQuestion { .. }
        )
    }
}

/// A suggestion in words for the inbox hint: the rule text for `addRules`
/// (`Bash(cargo test:*)`), `switch to acceptEdits` for `setMode`, `add directory /x` for
/// `addDirectories`; never a bare type name (live check 2026-11-06).
fn suggestion_label(raw: &Value) -> String {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let list = |key: &str| -> Vec<String> {
        raw.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let rules: Vec<String> = raw
        .get("rules")
        .and_then(Value::as_array)
        .map(|rules| {
            rules
                .iter()
                .map(|r| {
                    let tool = text(r, "toolName").unwrap_or_else(|| "a tool".into());
                    match text(r, "ruleContent") {
                        Some(content) => format!("{tool}({content})"),
                        None => tool,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let kind = text(raw, "type").unwrap_or_default();
    match kind.as_str() {
        "setMode" => match text(raw, "mode").as_deref() {
            Some("default") => "switch to manual mode".into(),
            Some(mode) => format!("switch to {mode}"),
            None => "switch permission mode".into(),
        },
        "addDirectories" => format!("add directory {}", list("directories").join(", ")),
        "removeDirectories" => format!("remove directory {}", list("directories").join(", ")),
        _ if !rules.is_empty() => rules.join(", "),
        _ => {
            // An unknown kind: its name in words (`someNewKind` → `some new kind`).
            let mut words = String::new();
            for c in kind.chars() {
                if c.is_uppercase() && !words.is_empty() {
                    words.push(' ');
                }
                words.extend(c.to_lowercase());
            }
            if words.is_empty() {
                "the offered rule".into()
            } else {
                words
            }
        }
    }
}

/// A suggestion as Dovetail sends it back: kept for this session only, because any other
/// destination (`localSettings`, `projectSettings`) writes into the repo, and nothing may.
fn session_only(mut raw: Value) -> Value {
    if let Some(obj) = raw.as_object_mut()
        && obj.contains_key("destination")
    {
        obj.insert("destination".into(), Value::String("session".into()));
    }
    raw
}

fn tool_result(text: impl Into<String>) -> SocketResponseBody {
    SocketResponseBody::ToolResult {
        text: text.into(),
        is_error: false,
    }
}

impl Core {
    /// Puts an item in Needs you, stores it, and updates the agent's status.
    pub(crate) fn raise(&mut self, agent: &AgentId, kind: InboxKind, waiter: Waiter) -> InboxId {
        let item = InboxItem {
            id: InboxId(self.next_inbox),
            agent: agent.clone(),
            at_ms: now_ms(),
            kind,
        };
        self.next_inbox += 1;
        let id = item.id;
        self.persist("an inbox item", self.db.insert_inbox(item.clone()));
        self.emit(Event::InboxUpserted(item.clone()));
        self.inbox.insert(id, InboxEntry { item, waiter });
        self.refresh_status(agent);
        id
    }

    /// Takes an item out of Needs you. The caller acts on its waiter.
    pub(crate) fn resolve(&mut self, id: InboxId, answer: Option<String>) -> Option<InboxEntry> {
        let entry = self.inbox.remove(&id)?;
        self.persist("an inbox answer", self.db.resolve_inbox(id, answer));
        self.emit(Event::InboxResolved { id });
        self.refresh_status(&entry.item.agent);
        Some(entry)
    }

    /// The agent's turn ended: at most one such item per agent, the newest (ADR 0001).
    pub(crate) fn finished_turn(&mut self, agent: &AgentId, preview: String) {
        let old: Vec<InboxId> = self
            .inbox
            .values()
            .filter(|p| {
                &p.item.agent == agent && matches!(p.item.kind, InboxKind::FinishedTurn { .. })
            })
            .map(|p| p.item.id)
            .collect();
        for id in old {
            self.resolve(id, None);
        }
        self.raise(agent, InboxKind::FinishedTurn { preview }, Waiter::None);
    }

    pub(crate) fn permission_request(&mut self, agent: &AgentId, request: PermissionRequest) {
        let suggestions = request
            .suggestions
            .iter()
            .map(|raw| RuleSuggestion {
                label: suggestion_label(raw),
                raw: raw.clone(),
            })
            .collect();
        let mut reason = request.decision_reason.or(request.description);
        if let Some(sub) = &request.agent_id {
            // M11 attributes it to the subagent's own node.
            reason = Some(format!(
                "from subagent {sub}{}",
                reason.map(|r| format!(": {r}")).unwrap_or_default()
            ));
        }
        let kind = InboxKind::Permission {
            tool: request.display_name.unwrap_or(request.tool_name),
            input: request.input,
            reason,
            suggestions,
        };
        self.raise(
            agent,
            kind,
            Waiter::Control {
                control: request.id,
                suggestions: request.suggestions,
            },
        );
    }

    /// Claude Code withdrew a request (`control_cancel_request`).
    pub(crate) fn permission_cancelled(&mut self, agent: &AgentId, control: &ControlId) {
        let found = self.inbox.values().find_map(|p| match &p.waiter {
            Waiter::Control { control: c, .. } if &p.item.agent == agent && c == control => {
                Some(p.item.id)
            }
            _ => None,
        });
        if let Some(id) = found {
            self.resolve(id, Some("cancelled".into()));
        }
    }

    /// A session ended: its held requests can no longer be answered.
    pub(crate) fn drop_session_items(&mut self, agent: &AgentId) {
        let held: Vec<InboxId> = self
            .inbox
            .values()
            .filter(|p| &p.item.agent == agent && matches!(p.waiter, Waiter::Control { .. }))
            .map(|p| p.item.id)
            .collect();
        for id in held {
            self.resolve(id, Some("session ended".into()));
        }
    }

    /// `y`, `A`, `d`, `D` on a Permission item.
    pub(crate) async fn answer_permission(&mut self, id: InboxId, answer: PermissionAnswer) {
        let Some(entry) = self.inbox.get(&id) else {
            self.fail(format!("no inbox item {}", id.0));
            return;
        };
        if !matches!(entry.item.kind, InboxKind::Permission { .. }) {
            self.fail("that item is not a permission request");
            return;
        }
        let label = match &answer {
            PermissionAnswer::Allow => "allowed".to_owned(),
            PermissionAnswer::AllowAlways => "always allowed".to_owned(),
            PermissionAnswer::Deny { message } => {
                format!(
                    "denied{}",
                    message
                        .as_ref()
                        .map(|m| format!(": {m}"))
                        .unwrap_or_default()
                )
            }
        };
        let Some(entry) = self.resolve(id, Some(label)) else {
            return;
        };
        let agent = entry.item.agent.clone();
        match entry.waiter {
            Waiter::Control {
                control,
                suggestions,
            } => {
                let reply = match answer {
                    PermissionAnswer::Allow => ControlAnswer::allow(),
                    PermissionAnswer::AllowAlways if suggestions.is_empty() => {
                        self.notice(&agent, "No rule was offered; allowed once");
                        ControlAnswer::allow()
                    }
                    PermissionAnswer::AllowAlways => ControlAnswer::allow_with_rules(
                        suggestions.into_iter().map(session_only).collect(),
                    ),
                    PermissionAnswer::Deny { message } => {
                        let text = message
                            .clone()
                            .unwrap_or_else(|| "The user denied this.".into());
                        self.notice(
                            &agent,
                            match message {
                                Some(m) => format!("Denied: {m}"),
                                None => "Denied".into(),
                            },
                        );
                        ControlAnswer::deny(text)
                    }
                };
                let sent = match self.live.get(&agent) {
                    Some(live) => live
                        .session
                        .answer_control(control, reply)
                        .map_err(|e| e.to_string()),
                    None => Err("the session has ended".into()),
                };
                if let Err(message) = sent {
                    self.fail(message);
                }
            }
            Waiter::StartSlice { request } => {
                let name = request.name.clone();
                let text = match answer {
                    PermissionAnswer::Deny { message } => {
                        let why = message.map(|m| format!(": {m}")).unwrap_or_default();
                        format!("start_slice {name} denied by the user{why}")
                    }
                    _ => match self.request_slice(request).await {
                        Ok(SliceStart::Started(_)) => format!(
                            "start_slice {name} approved: it is setting up; wait for \"slice \
                             {name} ready\" before you message it"
                        ),
                        Ok(SliceStart::Queued(_)) => format!(
                            "start_slice {name} approved and queued: the maximum number of \
                             slices is running; it starts when one is archived"
                        ),
                        Err(e) => {
                            format!("start_slice {name} approved, but it could not start: {e}")
                        }
                    },
                };
                self.tell_coordinator(text);
            }
            Waiter::RemoveWorktree => match answer {
                PermissionAnswer::Deny { .. } => self.keep_slice(&agent),
                _ => self.remove_worktree(&agent, true),
            },
            Waiter::RemoveOrphan { found } => match answer {
                PermissionAnswer::Deny { .. } => self.keep_orphan(found),
                _ => self.remove_orphan(found),
            },
            _ => {}
        }
        self.refresh_status(&agent);
    }

    /// `r` on a Question item (or any item: B22 sends a normal message to its agent).
    pub(crate) async fn answer_question(&mut self, id: InboxId, text: String) {
        let Some(entry) = self.inbox.get(&id) else {
            self.fail(format!("no inbox item {}", id.0));
            return;
        };
        if matches!(entry.item.kind, InboxKind::Permission { .. }) {
            self.fail("that item is a permission request: y or d");
            return;
        }
        let Some(entry) = self.resolve(id, Some(text.clone())) else {
            return;
        };
        let agent = entry.item.agent;
        match entry.waiter {
            Waiter::BlockingQuestion { reply } => {
                self.add_entry(&agent, dovetail_api::EntryKind::User, text.clone());
                let _ = reply.send(tool_result(format!("The user answered: {text}")));
            }
            Waiter::Question { question } => {
                let message = format!("The user answered your question \"{question}\":\n\n{text}");
                if let Err(e) = self.send_text_as_user(&agent, &message, text) {
                    self.fail(e);
                }
            }
            Waiter::RestoredQuestion { question } => {
                let message = format!("Answer to your question '{question}': {text}");
                if let Err(e) = self.send_text_as_user(&agent, &message, text) {
                    self.fail(e);
                }
            }
            _ => self.prompt(&agent, text).await,
        }
    }

    /// `d` on a non-permission item, or `x`: take it out without an answer.
    pub(crate) async fn dismiss(&mut self, id: InboxId) {
        // Dismissing is idempotent: the UI may dismiss an item that a newer finished turn or an
        // answer already took out, and that is no error to show the user.
        let Some(entry) = self.resolve(id, Some("dismissed".into())) else {
            tracing::debug!(item = id.0, "dismissed an item already gone");
            return;
        };
        let agent = entry.item.agent;
        match entry.waiter {
            Waiter::BlockingQuestion { reply } => {
                let _ = reply.send(tool_result(
                    "The user dismissed the question without answering. Carry on without it, \
                     or ask again later if you must.",
                ));
            }
            Waiter::Control { control, .. } => {
                if let Some(live) = self.live.get(&agent) {
                    let _ = live.session.answer_control(
                        control,
                        ControlAnswer::deny("The user dismissed this request."),
                    );
                }
            }
            Waiter::StartSlice { request } => {
                self.tell_coordinator(format!(
                    "start_slice {} denied: the user dismissed the request",
                    request.name
                ));
            }
            Waiter::RemoveWorktree => self.keep_slice(&agent),
            Waiter::RemoveOrphan { found } => self.keep_orphan(found),
            Waiter::None | Waiter::Question { .. } | Waiter::RestoredQuestion { .. } => {}
        }
    }

    /// Sends `message` to the session but shows the user's own words in the conversation.
    fn send_text_as_user(
        &mut self,
        agent: &AgentId,
        message: &str,
        shown: String,
    ) -> Result<(), SessionError> {
        self.ensure_live(agent)?;
        let Some(live) = self.live.get_mut(agent) else {
            return Err(SessionError::NotRunning(agent.0.clone()));
        };
        live.session
            .send(dovetail_driver::UserInput::text(message))
            .map_err(|source| SessionError::Driver {
                name: agent.0.clone(),
                source,
            })?;
        live.turn.input_sent();
        self.add_entry(agent, dovetail_api::EntryKind::User, shown);
        self.refresh_status(agent);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn suggestions_are_labelled_and_kept_out_of_the_repo() {
        let raw = json!({"behavior": "allow", "destination": "localSettings",
            "rules": [{"ruleContent": "cargo test:*", "toolName": "Bash"}], "type": "addRules"});
        assert_eq!(suggestion_label(&raw), "Bash(cargo test:*)");
        let mode = json!({"type": "setMode", "mode": "acceptEdits", "destination": "session"});
        assert_eq!(suggestion_label(&mode), "switch to acceptEdits");
        let dirs = json!({"type": "addDirectories", "directories": ["/tmp/x"]});
        assert_eq!(suggestion_label(&dirs), "add directory /tmp/x");
        assert_eq!(session_only(raw)["destination"], "session");
    }
}

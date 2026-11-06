//! Opening again after a quit, a kill or a crash (M10, decisions of 2026-11-06).
//!
//! - **Cut-off turns.** An agent stored as working, needing you or setting up was cut off. It
//!   gets one Needs-you item with its last prompt; it is never continued automatically.
//! - **In-flight items.** A pending permission request died with its session: it leaves Needs
//!   you and becomes an Activity note. A pending `ask_user` question stays; answering it sends the
//!   agent a normal message. Finished turns and errors stay as they were.
//! - **Restore.** The coordinator's session resumes at once (if it has run before); slices resume
//!   when first touched (`ensure_live`).
//!
//! The state at quit is kept as it was: statuses are not recomputed while quitting, so a turn
//! running at quit reads as cut off next time.

use dovetail_api::{ActivityKind, AgentId, AgentStatus, EntryKind, Event, InboxItem, InboxKind};

use crate::inbox::Waiter;
use crate::service::{COORDINATOR, Core, InboxEntry};

/// What `load` found that needs acting on once the UI has its snapshot.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    /// Agents whose turn (or setup) was running when the app stopped, with the status stored.
    pub(crate) cut_off: Vec<(AgentId, AgentStatus)>,
    /// Inbox items never resolved.
    pub(crate) inbox: Vec<InboxItem>,
}

/// `Write a.txt`, `Bash cargo test`: the tool and the one thing it acts on.
fn request_summary(tool: &str, input: &serde_json::Value) -> String {
    let field = |k: &str| input.get(k).and_then(|v| v.as_str());
    let target = field("command")
        .map(str::to_owned)
        .or_else(|| {
            field("file_path").map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map_or_else(|| p.to_owned(), |n| n.to_string_lossy().into_owned())
            })
        })
        .or_else(|| field("name").map(str::to_owned));
    match target {
        Some(t) => format!("{tool} {t}"),
        None => tool.to_owned(),
    }
}

impl Core {
    /// Acts on what [`Restored`] holds. Runs after the first snapshot, so every change goes out
    /// as an event.
    pub(crate) async fn reconcile(&mut self, restored: Restored) {
        let mut asking = std::collections::HashSet::new();
        for item in restored.inbox {
            match &item.kind {
                InboxKind::Permission { tool, input, .. } => {
                    self.persist(
                        "an expired request",
                        self.db
                            .resolve_inbox(item.id, Some("expired at restart".into())),
                    );
                    let name = self.name_of(&item.agent);
                    self.activity(
                        ActivityKind::Lifecycle,
                        Some(&item.agent),
                        format!(
                            "{name}: {}: request expired at restart",
                            request_summary(tool, input)
                        ),
                    );
                }
                InboxKind::Question { text, .. } => {
                    asking.insert(item.agent.clone());
                    let question = text.clone();
                    self.restore_item(item, Waiter::RestoredQuestion { question });
                }
                InboxKind::FinishedTurn { .. } | InboxKind::Error { .. } => {
                    self.restore_item(item, Waiter::None);
                }
            }
        }
        for (agent, status) in restored.cut_off {
            if !self.agents.contains_key(&agent) {
                continue;
            }
            // Store the reset status, so the next open does not report it again.
            self.save_agent(&agent);
            // A question still waiting says enough: answering it resumes the agent.
            if asking.contains(&agent) {
                continue;
            }
            let name = self.name_of(&agent);
            let text = if status == AgentStatus::SettingUp {
                format!("{name}'s setup was cut off by a restart; archive it and start it again")
            } else {
                let prompt = self.last_prompt(&agent);
                match prompt {
                    Some(p) => {
                        format!("{name}'s turn was cut off by a restart\n\nLast prompt: {p}")
                    }
                    None => format!("{name}'s turn was cut off by a restart"),
                }
            };
            self.raise(&agent, InboxKind::Error { text }, Waiter::None);
        }
        let ids: Vec<AgentId> = self.agents.keys().cloned().collect();
        for id in &ids {
            self.refresh_status(id);
        }
        // The coordinator comes back at once; its turn is not continued (no input is sent).
        let coordinator = AgentId(COORDINATOR.into());
        let has_run = self
            .agents
            .get(&coordinator)
            .is_some_and(|r| r.transcript_path.is_some());
        if has_run && let Err(e) = self.ensure_live(&coordinator) {
            self.fail(e);
        }
    }

    /// Puts an item read from storage back in Needs you, as it was.
    fn restore_item(&mut self, item: InboxItem, waiter: Waiter) {
        self.emit(Event::InboxUpserted(item.clone()));
        let agent = item.agent.clone();
        self.inbox.insert(item.id, InboxEntry { item, waiter });
        self.refresh_status(&agent);
    }

    /// The agent's most recent prompt from the user, as shown in its conversation.
    fn last_prompt(&self, agent: &AgentId) -> Option<String> {
        self.tails
            .get(agent)?
            .entries
            .iter()
            .rev()
            .find(|e| e.kind == EntryKind::User)
            .map(|e| e.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expired_requests_name_the_tool_and_its_target() {
        assert_eq!(
            request_summary("Write", &json!({"file_path": "/w/a.txt", "content": "x"})),
            "Write a.txt"
        );
        assert_eq!(
            request_summary("Bash", &json!({"command": "cargo test"})),
            "Bash cargo test"
        );
    }
}

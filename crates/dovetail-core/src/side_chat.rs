//! Side chats (ADR 0011): read-only sessions the user opens for a question or some research,
//! in the main checkout, with no worktree and no task. They close like slices
//! ([`Core::close_slice`]). They run in `dontAsk` mode with a read-only allow-list
//! (`session::launch::side_chat_allowed`).

use dovetail_api::{ActivityKind, AgentId, AgentKind, EntryKind, PermissionMode};

use crate::db::{AgentRow, now_ms};
use crate::service::{Core, new_agent};

impl Core {
    /// Starts a side chat named `side-N` and sends it `question`.
    pub(crate) fn start_side_chat(&mut self, question: String) {
        let question = question.trim().to_string();
        if question.is_empty() {
            self.fail("a side chat needs a question");
            return;
        }
        let taken = |name: &str| self.agents.values().any(|r| r.agent.name == name);
        let Some(name) = (1..).map(|n| format!("side-{n}")).find(|name| !taken(name)) else {
            return;
        };
        let id = AgentId(name.clone());
        let mut agent = new_agent(id.clone(), name.clone(), AgentKind::SideChat, None);
        // It runs in `dontAsk`, where nothing outside its allow-list runs (launch).
        agent.permission_mode = PermissionMode::Manual;
        let row = AgentRow {
            agent,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            transcript_path: None,
            task_list_id: Some(self.task_list_id(None, &name)),
            created_ms: now_ms(),
            tool_use_id: None,
            claude_home: None,
            takeover: None,
        };
        self.agents.insert(id.clone(), row);
        self.save_agent(&id);
        let cwd = self.project.repo_path().to_path_buf();
        if let Err(e) = self.launch(&id, cwd, None) {
            self.closed(&id);
            self.fail(e);
            return;
        }
        self.activity(
            ActivityKind::Lifecycle,
            Some(&id),
            format!("side chat {name} started"),
        );
        if let Err(e) = self.send_text(&id, &question, EntryKind::User, question.clone()) {
            self.fail(e);
        }
    }
}

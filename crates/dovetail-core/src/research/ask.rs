//! Reading mode's Ask (ADR 0018): one read-only session per research task, about the task's
//! report and the sources it cites, apart from the coordinator so nothing lands in its context.
//!
//! [`ResearchCommand::StartAsk`](dovetail_api::research::ResearchCommand::StartAsk) makes the agent (`ask-<task>`,
//! [`AgentKind::Ask`]) without starting a process. The first question starts the session in the
//! task's research folder with the side chats' read-only lists and no Dovetail tool suites
//! (`session::launch`), and carries the report and an index of finding and source ids → files;
//! the conversation shows only the question. A new report closes the session
//! ([`Core::close_asks_of`]), so the next question starts on the new one.

use std::path::{Path, PathBuf};

use dovetail_api::activity::ActivityKind;
use dovetail_api::agents::{
    agent::{AgentEvent, AgentId, AgentKind, PermissionMode},
    conversation::{Attachment, EntryKind},
};
use dovetail_api::tracking::task::TaskId;
use dovetail_driver::UserInput;

use crate::db::{AgentRow, now_ms};
use crate::research::files;
use crate::service::{Core, new_agent};

/// Why an Ask session could not start or take a question.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AskError {
    /// No such task.
    #[error("there is no task #{0}")]
    NoTask(i64),
    /// The task has no report to ask about.
    #[error("task #{0} has no report yet, so there is nothing to ask about")]
    NoReport(i64),
    /// The research folder is unknown.
    #[error(transparent)]
    Dir(#[from] files::ResearchDirError),
    /// Storage failed.
    #[error(transparent)]
    Db(#[from] crate::db::DbError),
    /// The session could not start or take the message.
    #[error(transparent)]
    Session(#[from] crate::error::SessionError),
}

/// The agent id of task `task`'s Ask session.
pub(crate) fn ask_id(task: TaskId) -> AgentId {
    AgentId(format!("ask-{}", task.0))
}

impl Core {
    /// `ResearchCommand::StartAsk`: the task's Ask agent, made if there is none. Its upsert tells the
    /// UI it is ready for a question.
    pub(crate) async fn start_ask(&mut self, task: TaskId) {
        let id = ask_id(task);
        if let Some(row) = self.agents.get(&id) {
            let agent = row.agent.clone();
            self.emit(dovetail_api::Event::Agent(AgentEvent::Upserted(agent)));
            return;
        }
        if let Err(e) = self.grounding(task).await {
            self.fail(e);
            return;
        }
        let mut agent = new_agent(id.clone(), id.0.clone(), AgentKind::Ask, Some(task));
        // It runs in `dontAsk` with the read-only lists (launch).
        agent.permission_mode = PermissionMode::Manual;
        let row = AgentRow {
            agent,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            transcript_path: None,
            task_list_id: Some(self.task_list_id(None, &id.0)),
            created_ms: now_ms(),
            tool_use_id: None,
            claude_home: None,
            takeover: None,
            model_choice: None,
            repo: None,
        };
        self.agents.insert(id.clone(), row);
        self.save_agent(&id);
        self.activity(
            ActivityKind::Lifecycle,
            Some(&id),
            format!("ask session for #{} ready", task.0),
        );
    }

    /// A question to an Ask session: starts it in the research folder when it is not running,
    /// and sends the report and index with its first question.
    pub(crate) async fn ask(&mut self, id: &AgentId, text: String, attachments: Vec<Attachment>) {
        if let Err(e) = self.try_ask(id, text, attachments).await {
            self.fail(e);
        }
    }

    async fn try_ask(
        &mut self,
        id: &AgentId,
        text: String,
        attachments: Vec<Attachment>,
    ) -> Result<(), AskError> {
        let task = self
            .agents
            .get(id)
            .and_then(|r| r.agent.task)
            .ok_or_else(|| crate::error::SessionError::NoAgent(id.0.clone()))?;
        let images = crate::session::read_attachments(&attachments)?;
        let (dir, grounding) = self.grounding(task).await?;
        if !self.live.contains_key(id) {
            self.launch(id, dir, None)?;
        }
        let asked = self
            .tails
            .get(id)
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::User));
        let sent = if asked {
            text.clone()
        } else {
            format!("{grounding}\n\n## Question\n\n{text}")
        };
        self.update_agent(id, |row| {
            if row.agent.status == dovetail_api::agents::agent::AgentStatus::Error {
                row.agent.status = dovetail_api::agents::agent::AgentStatus::Idle;
            }
        });
        self.send_input(
            id,
            UserInput { text: sent, images },
            EntryKind::User,
            text,
            attachments,
        )?;
        Ok(())
    }

    /// The research folder of `task` and the text that grounds its Ask session: what it is
    /// for, the index of ids → files, and the report.
    async fn grounding(&self, task: TaskId) -> Result<(PathBuf, String), AskError> {
        let t = self.task(task).await.ok_or(AskError::NoTask(task.0))?;
        let dir = self.research_dir(task, &t.title)?;
        let r = self.db.task_research(task).await?;
        let (_, report) = r.report.clone().ok_or(AskError::NoReport(task.0))?;
        let listing = crate::research::listing(&dir, &r);
        let rel = |p: &Path| {
            p.strip_prefix(&dir)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        };
        let mut index = String::new();
        for f in &listing.findings {
            let round = f
                .round
                .map_or_else(|| "quick finding".to_string(), |n| format!("round {n}"));
            let sources: Vec<String> = f.sources.iter().map(|s| format!("S{}", s.0)).collect();
            index.push_str(&format!(
                "- F{} `{}` ({round}; cites {})\n",
                f.id.0,
                rel(&f.path),
                if sources.is_empty() {
                    "nothing".to_string()
                } else {
                    sources.join(", ")
                }
            ));
        }
        for s in &listing.sources {
            let file = s
                .path
                .as_deref()
                .map_or_else(|| "not cached".to_string(), |p| format!("`{}`", rel(p)));
            index.push_str(&format!(
                "- S{} {} {} ({file})\n",
                s.id.0,
                s.kind.as_str(),
                s.locator
            ));
        }
        let text = format!(
            "You answer questions about the research report of task #{} \"{}\". Your working \
             folder is its research folder: the report is `report.md`, and each finding and \
             source is in the file the index names. Read them to check details. Cite every \
             claim with the markers the report uses, [F<n>] for a finding and [S<n>] for a \
             source, and only ids from this index.\n\n## Index\n\n{index}\n## The report\n\n{report}",
            task.0, t.title
        );
        Ok((dir, text))
    }

    /// Closes the Ask sessions of `task` (a new report makes their grounding stale).
    pub(crate) fn close_asks_of(&mut self, task: TaskId) {
        let id = ask_id(task);
        if self.agents.contains_key(&id) {
            self.close_slice(&id, true);
        }
    }
}

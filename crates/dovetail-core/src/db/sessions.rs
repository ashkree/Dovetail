//! Queries on a project's `state.db`: agents, conversation entries, inbox items, activity and
//! messages.

use std::path::PathBuf;

use dovetail_api::{
    ActivityItem, ActivityKind, Agent, AgentId, AgentKind, AgentMessage, AgentStatus,
    ConversationEntry, EntryId, EntryKind, InboxId, InboxItem, InboxKind, PermissionMode,
    SessionLock, TaskId, WorktreeRef,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::DbError;

/// One stored agent: what the UI sees, plus what only the core needs.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRow {
    /// The agent as the UI sees it.
    pub agent: Agent,
    /// The Claude Code session id Dovetail chose for it.
    pub session_id: Option<String>,
    /// Where Claude Code files its transcript; set once the session has started, so its
    /// presence also means "resume, don't create".
    pub transcript_path: Option<PathBuf>,
    /// `CLAUDE_CODE_TASK_LIST_ID`.
    pub task_list_id: Option<String>,
    /// When it was created, in Unix milliseconds.
    pub created_ms: i64,
}

fn kind_to_text(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Coordinator => "coordinator",
        AgentKind::Slice => "slice",
        AgentKind::Subagent { background: false } => "subagent",
        AgentKind::Subagent { background: true } => "background_subagent",
    }
}

fn kind_from_text(text: &str) -> Result<AgentKind, DbError> {
    Ok(match text {
        "coordinator" => AgentKind::Coordinator,
        "slice" => AgentKind::Slice,
        "subagent" => AgentKind::Subagent { background: false },
        "background_subagent" => AgentKind::Subagent { background: true },
        other => return Err(DbError::Corrupt(format!("agent kind {other:?}"))),
    })
}

fn status_to_text(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::SettingUp => "setting_up",
        AgentStatus::Working => "working",
        AgentStatus::Idle => "idle",
        AgentStatus::NeedsYou => "needs_you",
        AgentStatus::Done => "done",
        AgentStatus::Error => "error",
    }
}

fn status_from_text(text: &str) -> Result<AgentStatus, DbError> {
    Ok(match text {
        "setting_up" => AgentStatus::SettingUp,
        "working" => AgentStatus::Working,
        "idle" => AgentStatus::Idle,
        "needs_you" => AgentStatus::NeedsYou,
        "done" => AgentStatus::Done,
        "error" => AgentStatus::Error,
        other => return Err(DbError::Corrupt(format!("agent status {other:?}"))),
    })
}

fn mode_to_text(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Auto => "auto",
        PermissionMode::AcceptEdits => "accept_edits",
        PermissionMode::Manual => "manual",
    }
}

fn mode_from_text(text: &str) -> Result<PermissionMode, DbError> {
    Ok(match text {
        "auto" => PermissionMode::Auto,
        "accept_edits" => PermissionMode::AcceptEdits,
        "manual" => PermissionMode::Manual,
        other => return Err(DbError::Corrupt(format!("permission mode {other:?}"))),
    })
}

fn entry_kind_to_text(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::User => "user",
        EntryKind::Assistant => "assistant",
        EntryKind::ToolSummary => "tool_summary",
        EntryKind::Notice => "notice",
    }
}

fn entry_kind_from_text(text: &str) -> Result<EntryKind, DbError> {
    Ok(match text {
        "user" => EntryKind::User,
        "assistant" => EntryKind::Assistant,
        "tool_summary" => EntryKind::ToolSummary,
        "notice" => EntryKind::Notice,
        other => return Err(DbError::Corrupt(format!("entry kind {other:?}"))),
    })
}

fn activity_kind_to_text(kind: ActivityKind) -> &'static str {
    match kind {
        ActivityKind::Pr => "pr",
        ActivityKind::Ci => "ci",
        ActivityKind::Message => "message",
        ActivityKind::Lifecycle => "lifecycle",
    }
}

fn path_text(path: &Option<PathBuf>) -> Option<String> {
    path.as_ref().map(|p| p.to_string_lossy().into_owned())
}

pub(super) fn upsert_agent(conn: &mut Connection, row: &AgentRow) -> Result<(), DbError> {
    let a = &row.agent;
    conn.execute(
        "INSERT INTO agents (id, kind, name, parent_id, task_id, session_id, transcript_path, \
         task_list_id, worktree_path, branch, status, status_line, permission_mode, cost_usd, \
         created_ms, last_activity_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16) \
         ON CONFLICT(id) DO UPDATE SET kind = ?2, name = ?3, parent_id = ?4, task_id = ?5, \
         session_id = ?6, transcript_path = ?7, task_list_id = ?8, worktree_path = ?9, \
         branch = ?10, status = ?11, status_line = ?12, permission_mode = ?13, cost_usd = ?14, \
         last_activity_ms = ?16, closed_ms = NULL",
        params![
            a.id.0,
            kind_to_text(a.kind),
            a.name,
            a.parent.as_ref().map(|p| p.0.clone()),
            a.task.map(|t| t.0),
            row.session_id,
            path_text(&row.transcript_path),
            row.task_list_id,
            a.worktree
                .as_ref()
                .map(|w| w.path.to_string_lossy().into_owned()),
            a.worktree.as_ref().map(|w| w.branch.clone()),
            status_to_text(a.status),
            a.status_line,
            mode_to_text(a.permission_mode),
            a.cost_usd,
            row.created_ms,
            a.last_activity_ms,
        ],
    )?;
    Ok(())
}

pub(super) fn load_agents(conn: &mut Connection) -> Result<Vec<AgentRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, parent_id, task_id, session_id, transcript_path, task_list_id, \
         worktree_path, branch, status, status_line, permission_mode, cost_usd, created_ms, \
         last_activity_ms FROM agents WHERE closed_ms IS NULL ORDER BY created_ms, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            (
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
            ),
            r.get::<_, String>(10)?,
            r.get::<_, String>(11)?,
            r.get::<_, String>(12)?,
            r.get::<_, f64>(13)?,
            r.get::<_, Option<i64>>(14)?,
            r.get::<_, Option<i64>>(15)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            kind,
            name,
            parent,
            task,
            session_id,
            transcript,
            list,
            (wt_path, branch),
            status,
            status_line,
            mode,
            cost,
            created,
            last,
        ) = row?;
        let worktree = match (wt_path, branch) {
            (Some(path), Some(branch)) => Some(WorktreeRef {
                path: PathBuf::from(path),
                branch,
            }),
            _ => None,
        };
        out.push(AgentRow {
            agent: Agent {
                id: AgentId(id),
                name,
                kind: kind_from_text(&kind)?,
                parent: parent.map(AgentId),
                task: task.map(TaskId),
                worktree,
                status: status_from_text(&status)?,
                status_line,
                memory_mb: None,
                cost_usd: cost,
                permission_mode: mode_from_text(&mode)?,
                context_pct: None,
                progress: None,
                last_activity_ms: last.unwrap_or(0),
                lock: SessionLock::Headless,
            },
            session_id,
            transcript_path: transcript.map(PathBuf::from),
            task_list_id: list,
            created_ms: created.unwrap_or(0),
        });
    }
    Ok(out)
}

pub(super) fn close_agent(conn: &mut Connection, id: &AgentId, now: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE agents SET closed_ms = ?2 WHERE id = ?1",
        params![id.0, now],
    )?;
    Ok(())
}

pub(super) fn upsert_entry(
    conn: &mut Connection,
    agent: &AgentId,
    entry: &ConversationEntry,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO conversation_entries (id, agent_id, at_ms, kind, text) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET text = ?5",
        params![
            entry.id.0 as i64,
            agent.0,
            entry.at_ms,
            entry_kind_to_text(entry.kind),
            entry.text
        ],
    )?;
    Ok(())
}

pub(super) fn load_entries(
    conn: &mut Connection,
    agent: &AgentId,
    before: Option<EntryId>,
    limit: usize,
) -> Result<(Vec<ConversationEntry>, bool), DbError> {
    let before = before.map_or(i64::MAX, |b| b.0 as i64);
    let mut stmt = conn.prepare(
        "SELECT id, at_ms, kind, text FROM conversation_entries \
         WHERE agent_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![agent.0, before, limit as i64 + 1], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, at_ms, kind, text) = row?;
        entries.push(ConversationEntry {
            id: EntryId(id as u64),
            at_ms,
            kind: entry_kind_from_text(&kind)?,
            text,
        });
    }
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    entries.reverse();
    Ok((entries, has_more))
}

/// The highest ids in use, so the core's counters continue after them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaxIds {
    /// Conversation entries.
    pub entry: u64,
    /// Inbox items.
    pub inbox: u64,
    /// Activity items.
    pub activity: u64,
    /// Agent messages.
    pub message: u64,
}

pub(super) fn max_ids(conn: &mut Connection) -> Result<MaxIds, DbError> {
    let max = |table: &str| -> Result<u64, DbError> {
        let n: Option<i64> = conn
            .query_row(&format!("SELECT MAX(id) FROM {table}"), [], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(n.unwrap_or(0).max(0) as u64)
    };
    Ok(MaxIds {
        entry: max("conversation_entries")?,
        inbox: max("inbox_items")?,
        activity: max("activity")?,
        message: max("messages")?,
    })
}

pub(super) fn insert_inbox(conn: &mut Connection, item: &InboxItem) -> Result<(), DbError> {
    let (kind, blocking) = match &item.kind {
        InboxKind::Permission { .. } => ("permission", false),
        InboxKind::Question { blocking, .. } => ("question", *blocking),
        InboxKind::FinishedTurn { .. } => ("finished_turn", false),
        InboxKind::Error { .. } => ("error", false),
    };
    let payload = serde_json::to_string(&item.kind).map_err(|e| DbError::Corrupt(e.to_string()))?;
    conn.execute(
        "INSERT OR REPLACE INTO inbox_items (id, agent_id, kind, payload, blocking, created_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            item.id.0 as i64,
            item.agent.0,
            kind,
            payload,
            blocking,
            item.at_ms
        ],
    )?;
    Ok(())
}

pub(super) fn resolve_inbox(
    conn: &mut Connection,
    id: InboxId,
    answer: Option<&str>,
    now: i64,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE inbox_items SET resolved_ms = ?2, answer = ?3 WHERE id = ?1",
        params![id.0 as i64, now, answer],
    )?;
    Ok(())
}

pub(super) fn add_activity(conn: &mut Connection, item: &ActivityItem) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO activity (id, at_ms, kind, agent_id, text) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            item.id as i64,
            item.at_ms,
            activity_kind_to_text(item.kind),
            item.agent.as_ref().map(|a| a.0.clone()),
            item.text
        ],
    )?;
    Ok(())
}

pub(super) fn add_message(
    conn: &mut Connection,
    message: &AgentMessage,
    msg_id: Option<&str>,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO messages (id, at_ms, from_agent, to_agent, text, msg_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            message.id as i64,
            message.at_ms,
            message.from.0,
            message.to.0,
            message.text,
            msg_id
        ],
    )?;
    Ok(())
}

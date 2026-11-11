//! Queries on a project's `state.db`: agents, conversation entries, inbox items, activity and
//! messages.

use std::path::PathBuf;

use dovetail_api::{
    ActivityItem, Agent, AgentId, AgentKind, AgentMessage, Attachment, ConversationEntry, EntryId,
    InboxId, InboxItem, InboxKind, SessionLock, SubagentProgress, TaskId, WorktreeRef,
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
    /// Subagents only: the `Agent` tool use that launched it. Its events carry this as
    /// `parent_tool_use_id`.
    pub tool_use_id: Option<String>,
    /// The Claude Code config directory (profile) its session runs under, set at each launch;
    /// `None` for the default home of before profiles (ADR 0009). Subagents copy their parent's.
    pub claude_home: Option<PathBuf>,
    /// Its take-over lock while the user works in native `claude` (M14); `agent.lock` says
    /// `TakenOver` exactly when this is set.
    pub takeover: Option<crate::takeover::TakeoverLock>,
}

fn kind_to_text(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Coordinator => "coordinator",
        AgentKind::Slice => "slice",
        AgentKind::SideChat => "side_chat",
        AgentKind::Ask => "ask",
        AgentKind::Subagent { background: false } => "subagent",
        AgentKind::Subagent { background: true } => "background_subagent",
    }
}

fn kind_from_text(text: &str) -> Result<AgentKind, DbError> {
    Ok(match text {
        "coordinator" => AgentKind::Coordinator,
        "slice" => AgentKind::Slice,
        "side_chat" => AgentKind::SideChat,
        "ask" => AgentKind::Ask,
        "subagent" => AgentKind::Subagent { background: false },
        "background_subagent" => AgentKind::Subagent { background: true },
        other => return Err(DbError::Corrupt(format!("agent kind {other:?}"))),
    })
}

/// A unit enum's text in the database: its serde name in the API (`setting_up`,
/// `accept_edits`, `tool_summary`, ...), so the API is the one source of these names.
fn to_text<T: serde::Serialize>(value: &T) -> Result<String, DbError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(DbError::Corrupt("a value with no text form".into())),
    }
}

fn from_text<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T, DbError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| DbError::Corrupt(format!("{what} {text:?}")))
}

/// An entry's attachments as stored: a JSON array, or NULL when there are none.
fn attachments_text(attachments: &[Attachment]) -> Result<Option<String>, DbError> {
    if attachments.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(attachments)
        .map(Some)
        .map_err(|e| DbError::Corrupt(format!("attachments that do not serialise: {e}")))
}

/// A subagent's progress as stored: JSON, or NULL for sessions.
fn progress_text(progress: &Option<SubagentProgress>) -> Result<Option<String>, DbError> {
    progress
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| DbError::Corrupt(format!("progress that does not serialise: {e}")))
}

/// A take-over lock as stored: JSON, or NULL when the session runs headless.
fn takeover_text(lock: &Option<crate::takeover::TakeoverLock>) -> Result<Option<String>, DbError> {
    lock.as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| DbError::Corrupt(format!("a take-over lock that does not serialise: {e}")))
}

fn path_text(path: &Option<PathBuf>) -> Option<String> {
    path.as_ref().map(|p| p.to_string_lossy().into_owned())
}

pub(super) fn upsert_agent(conn: &mut Connection, row: &AgentRow) -> Result<(), DbError> {
    let a = &row.agent;
    conn.execute(
        "INSERT INTO agents (id, kind, name, parent_id, task_id, session_id, transcript_path, \
         task_list_id, worktree_path, branch, status, status_line, permission_mode, cost_usd, \
         created_ms, last_activity_ms, tool_use_id, output_file, progress, subagent_tokens, \
         claude_home, takeover) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
         ?18, ?19, ?20, ?21, ?22) \
         ON CONFLICT(id) DO UPDATE SET kind = ?2, name = ?3, parent_id = ?4, task_id = ?5, \
         session_id = ?6, transcript_path = ?7, task_list_id = ?8, worktree_path = ?9, \
         branch = ?10, status = ?11, status_line = ?12, permission_mode = ?13, cost_usd = ?14, \
         last_activity_ms = ?16, tool_use_id = ?17, output_file = ?18, progress = ?19, \
         subagent_tokens = ?20, claude_home = ?21, takeover = ?22, closed_ms = NULL",
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
            to_text(&a.status)?,
            a.status_line,
            to_text(&a.permission_mode)?,
            a.cost_usd,
            row.created_ms,
            a.last_activity_ms,
            row.tool_use_id,
            a.progress
                .as_ref()
                .and_then(|p| p.output_file.as_ref())
                .map(|f| f.to_string_lossy().into_owned()),
            progress_text(&a.progress)?,
            i64::try_from(a.subagent_tokens).unwrap_or(i64::MAX),
            path_text(&row.claude_home),
            takeover_text(&row.takeover)?,
        ],
    )?;
    Ok(())
}

pub(super) fn load_agents(conn: &mut Connection) -> Result<Vec<AgentRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, parent_id, task_id, session_id, transcript_path, task_list_id, \
         worktree_path, branch, status, status_line, permission_mode, cost_usd, created_ms, \
         last_activity_ms, tool_use_id, progress, subagent_tokens, claude_home, takeover FROM agents \
         WHERE closed_ms IS NULL \
         ORDER BY created_ms, id",
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
            (
                r.get::<_, Option<String>>(16)?,
                r.get::<_, Option<String>>(17)?,
                r.get::<_, i64>(18)?,
                r.get::<_, Option<String>>(19)?,
                r.get::<_, Option<String>>(20)?,
            ),
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
            (tool_use_id, progress, subagent_tokens, claude_home, takeover),
        ) = row?;
        let takeover = takeover
            .map(|t| {
                serde_json::from_str::<crate::takeover::TakeoverLock>(&t)
                    .map_err(|e| DbError::Corrupt(format!("take-over lock: {e}")))
            })
            .transpose()?;
        let progress = progress
            .map(|p| {
                serde_json::from_str::<SubagentProgress>(&p)
                    .map_err(|e| DbError::Corrupt(format!("subagent progress: {e}")))
            })
            .transpose()?;
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
                status: from_text("agent status", &status)?,
                status_line,
                memory_mb: None,
                cost_usd: cost,
                permission_mode: from_text("permission mode", &mode)?,
                context_pct: None,
                progress,
                last_activity_ms: last.unwrap_or(0),
                lock: if takeover.is_some() {
                    SessionLock::TakenOver
                } else {
                    SessionLock::Headless
                },
                model: None,
                subagent_tokens: u64::try_from(subagent_tokens).unwrap_or(0),
            },
            session_id,
            transcript_path: transcript.map(PathBuf::from),
            task_list_id: list,
            created_ms: created.unwrap_or(0),
            tool_use_id,
            claude_home: claude_home.map(PathBuf::from),
            takeover,
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

/// Image files named by an agent's conversation entries.
pub(super) fn agent_attachments(
    conn: &Connection,
    agent: &AgentId,
) -> Result<Vec<PathBuf>, DbError> {
    attachment_paths(
        conn,
        "SELECT attachments FROM conversation_entries \
         WHERE agent_id = ?1 AND attachments IS NOT NULL",
        params![agent.0],
    )
}

/// Image files named by the conversations of agents that are not archived.
pub(super) fn live_attachments(conn: &Connection) -> Result<Vec<PathBuf>, DbError> {
    attachment_paths(
        conn,
        "SELECT e.attachments FROM conversation_entries e \
         JOIN agents a ON a.id = e.agent_id \
         WHERE a.closed_ms IS NULL AND e.attachments IS NOT NULL",
        [],
    )
}

fn attachment_paths(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<PathBuf>, DbError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| r.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for json in rows {
        // A row that does not parse names nothing we could delete; load_entries reports it.
        if let Ok(list) = serde_json::from_str::<Vec<Attachment>>(&json?) {
            paths.extend(list.into_iter().map(|a| a.path));
        }
    }
    Ok(paths)
}

pub(super) fn upsert_entry(
    conn: &mut Connection,
    agent: &AgentId,
    entry: &ConversationEntry,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO conversation_entries (id, agent_id, at_ms, kind, text, attachments) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(id) DO UPDATE SET text = ?5",
        params![
            entry.id.0 as i64,
            agent.0,
            entry.at_ms,
            to_text(&entry.kind)?,
            entry.text,
            attachments_text(&entry.attachments)?
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
        "SELECT id, at_ms, kind, text, attachments FROM conversation_entries \
         WHERE agent_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![agent.0, before, limit as i64 + 1], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, at_ms, kind, text, attachments) = row?;
        let attachments = match attachments {
            Some(json) => serde_json::from_str(&json)
                .map_err(|_| DbError::Corrupt(format!("attachments {json:?}")))?,
            None => Vec::new(),
        };
        entries.push(ConversationEntry {
            id: EntryId(id as u64),
            at_ms,
            kind: from_text("entry kind", &kind)?,
            text,
            attachments,
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
        // `core_ask_user` never blocks (ADR 0007); the column stays for old rows.
        InboxKind::Question { .. } => ("question", false),
        InboxKind::FinishedTurn { .. } => ("finished_turn", false),
        InboxKind::Error { .. } => ("error", false),
        InboxKind::Proposal { .. } => ("proposal", false),
        InboxKind::UnfiledFinding { .. } => ("unfiled_finding", false),
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
            to_text(&item.kind)?,
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

/// A slice start waiting for room under `max_slices` (ADR 0004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedSlice {
    /// Row id; the queue runs in id order.
    pub id: i64,
    /// The task it works on.
    pub task: Option<TaskId>,
    /// Slice name.
    pub name: String,
    /// First prompt.
    pub brief: String,
    /// Started by the coordinator's `project_start_slice` (it is told if the start fails).
    pub by_coordinator: bool,
}

pub(super) fn enqueue_slice(
    conn: &mut Connection,
    task: Option<TaskId>,
    name: &str,
    brief: &str,
    by_coordinator: bool,
    now: i64,
) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO slice_queue (task_id, name, brief, by_coordinator, created_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task.map(|t| t.0), name, brief, by_coordinator, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub(super) fn dequeue_slice(conn: &mut Connection, id: i64) -> Result<(), DbError> {
    conn.execute("DELETE FROM slice_queue WHERE id = ?1", [id])?;
    Ok(())
}

pub(super) fn load_queue(conn: &mut Connection) -> Result<Vec<QueuedSlice>, DbError> {
    let mut stmt = conn
        .prepare("SELECT id, task_id, name, brief, by_coordinator FROM slice_queue ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(QueuedSlice {
            id: r.get(0)?,
            task: r.get::<_, Option<i64>>(1)?.map(TaskId),
            name: r.get(2)?,
            brief: r.get(3)?,
            by_coordinator: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(super) fn load_open_inbox(conn: &mut Connection) -> Result<Vec<InboxItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, payload, created_ms FROM inbox_items \
         WHERE resolved_ms IS NULL ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (id, agent, payload, at_ms) = row?;
        let kind: InboxKind = serde_json::from_str(&payload)
            .map_err(|e| DbError::Corrupt(format!("inbox item {id}: {e}")))?;
        items.push(InboxItem {
            id: InboxId(id as u64),
            agent: AgentId(agent),
            at_ms,
            kind,
        });
    }
    Ok(items)
}

pub(super) fn add_note(conn: &mut Connection, text: &str, now: i64) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO coordinator_notes (text, created_ms) VALUES (?1, ?2)",
        params![text, now],
    )?;
    Ok(())
}

pub(super) fn load_notes(conn: &mut Connection) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT text FROM coordinator_notes ORDER BY id")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(super) fn clear_notes(conn: &mut Connection) -> Result<(), DbError> {
    conn.execute("DELETE FROM coordinator_notes", [])?;
    Ok(())
}

pub(super) fn keep_worktree(
    conn: &mut Connection,
    path: &std::path::Path,
    now: i64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO kept_worktrees (path, kept_ms) VALUES (?1, ?2)",
        params![path.to_string_lossy(), now],
    )?;
    Ok(())
}

pub(super) fn kept_worktrees(conn: &mut Connection) -> Result<Vec<PathBuf>, DbError> {
    let mut stmt = conn.prepare("SELECT path FROM kept_worktrees")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(PathBuf::from)
        .collect())
}

// --- native to-do attribution (M11) ---

pub(super) fn set_todo_owner(
    conn: &mut Connection,
    list: &str,
    item: &str,
    agent: &str,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO todo_items (list_id, item_id, agent_id) VALUES (?1, ?2, ?3)",
        params![list, item, agent],
    )?;
    Ok(())
}

pub(super) fn load_todo_owners(
    conn: &mut Connection,
) -> Result<Vec<(String, String, AgentId)>, DbError> {
    let mut stmt = conn.prepare("SELECT list_id, item_id, agent_id FROM todo_items")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            AgentId(r.get::<_, String>(2)?),
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(super) fn forget_todo_list(conn: &mut Connection, list: &str) -> Result<(), DbError> {
    conn.execute("DELETE FROM todo_items WHERE list_id = ?1", params![list])?;
    Ok(())
}

//! Queries on a project's `state.db`: research rounds, findings and sources (R1,
//! `docs/research.md` §3). Paths are stored relative to the task's research folder, which the
//! core resolves (`research::files`).

use dovetail_api::research::{FindingId, SourceId, SourceKind};
use dovetail_api::{agent::AgentId, task::TaskId};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::DbError;

/// A research round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRow {
    /// Row id.
    pub id: i64,
    /// The task.
    pub task: TaskId,
    /// 1-based within the task.
    pub n: u32,
    /// When it was opened.
    pub opened_ms: i64,
    /// When it was closed; `None` while open.
    pub closed_ms: Option<i64>,
    /// The user's criteria.
    pub brief: String,
    /// `rounds/01-2026-11-10`, relative to the task's folder.
    pub dir: String,
    /// Requested while another round was open; opens when it closes.
    pub queued: bool,
}

/// A source as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    /// Row id.
    pub id: SourceId,
    /// Web page, file or search.
    pub kind: SourceKind,
    /// URL, absolute path, or query.
    pub locator: String,
    /// The tool that reached it.
    pub tool: String,
    /// WebFetch's prompt or WebSearch's query.
    pub query: Option<String>,
    /// What the tool returned to the agent.
    pub content: String,
    /// Written to `sources/<id>.md`.
    pub cached: bool,
    /// The tool call failed.
    pub is_error: bool,
    /// When first fetched.
    pub fetched_ms: i64,
}

/// A finding as stored, with the ids of its sources in the order it reached them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRow {
    /// Row id.
    pub id: FindingId,
    /// The task.
    pub task: TaskId,
    /// Its round's number; `None` for a quick finding.
    pub round: Option<u32>,
    /// 1-based within its round, or among the task's quick findings.
    pub n: u32,
    /// The subagent.
    pub agent: AgentId,
    /// File-name slug.
    pub label: String,
    /// The `Agent` call's description.
    pub description: Option<String>,
    /// Relative to the task's folder.
    pub path: String,
    /// The full report.
    pub body: String,
    /// Which stream field the body came from.
    pub body_from: String,
    /// `general-purpose`, ...
    pub subagent_type: Option<String>,
    /// When the subagent was launched.
    pub started_ms: Option<i64>,
    /// When it was filed.
    pub created_ms: i64,
    /// Tokens the subagent used, when reported.
    pub tokens: Option<u64>,
    /// Its sources, in order.
    pub sources: Vec<SourceId>,
}

/// A source to store with a new finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSource {
    /// Web page, file or search.
    pub kind: SourceKind,
    /// URL, absolute path, or query.
    pub locator: String,
    /// The tool.
    pub tool: String,
    /// WebFetch's prompt or WebSearch's query.
    pub query: Option<String>,
    /// What the tool returned.
    pub content: String,
    /// Written to a file.
    pub cached: bool,
    /// The call failed.
    pub is_error: bool,
    /// Dedup key with the locator.
    pub content_hash: String,
    /// The stream's tool use id.
    pub tool_use_id: String,
}

/// A finding to file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFinding {
    /// The task; `None`: an unfiled finding (a coordinator subagent with no round open).
    pub task: Option<TaskId>,
    /// The round's row id; `None` for a quick finding.
    pub round: Option<i64>,
    /// The subagent.
    pub agent: AgentId,
    /// File-name slug.
    pub label: String,
    /// The `Agent` call's description.
    pub description: Option<String>,
    /// The report.
    pub body: String,
    /// Where it came from.
    pub body_from: String,
    /// Subagent type.
    pub subagent_type: Option<String>,
    /// When launched.
    pub started_ms: Option<i64>,
    /// Tokens used.
    pub tokens: Option<u64>,
    /// What it fetched or read, in order.
    pub sources: Vec<NewSource>,
}

/// One task's research, in full.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskResearch {
    /// Rounds, oldest first.
    pub rounds: Vec<RoundRow>,
    /// Findings, oldest first.
    pub findings: Vec<FindingRow>,
    /// Every source the findings cite, by id.
    pub sources: Vec<SourceRow>,
    /// The latest report: when, and its body.
    pub report: Option<(i64, String)>,
}

/// Counts for the snapshot's summary of one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    /// The task.
    pub task: TaskId,
    /// Its rounds with their finding counts.
    pub rounds: Vec<(RoundRow, u32)>,
    /// All findings.
    pub findings: u32,
    /// Findings with no round.
    pub quick_findings: u32,
    /// Distinct sources cited.
    pub sources: u32,
    /// The latest report's time.
    pub report_ms: Option<i64>,
    /// `tasks.research_kind`.
    pub kind: Option<String>,
}

const ROUND_COLUMNS: &str = "id, task_id, n, opened_ms, closed_ms, brief, dir, queued";

fn round(row: &Row<'_>) -> rusqlite::Result<RoundRow> {
    Ok(RoundRow {
        id: row.get(0)?,
        task: TaskId(row.get(1)?),
        n: row.get(2)?,
        opened_ms: row.get(3)?,
        closed_ms: row.get(4)?,
        brief: row.get(5)?,
        dir: row.get(6)?,
        queued: row.get(7)?,
    })
}

const SOURCE_COLUMNS: &str =
    "s.id, s.kind, s.locator, s.tool, s.query, s.content, s.cached, s.is_error, s.fetched_ms";

fn source(row: &Row<'_>) -> rusqlite::Result<SourceRow> {
    let kind: String = row.get(1)?;
    let kind = SourceKind::parse(&kind).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            Type::Text,
            format!("source kind {kind:?}").into(),
        )
    })?;
    Ok(SourceRow {
        id: SourceId(row.get(0)?),
        kind,
        locator: row.get(2)?,
        tool: row.get(3)?,
        query: row.get(4)?,
        content: row.get(5)?,
        cached: row.get(6)?,
        is_error: row.get(7)?,
        fetched_ms: row.get(8)?,
    })
}

const FINDING_COLUMNS: &str = "f.id, f.task_id, r.n, f.n, f.agent_id, f.label, f.path, f.body, \
     f.body_from, f.subagent_type, f.started_ms, f.created_ms, f.tokens, f.description";

fn finding(row: &Row<'_>) -> rusqlite::Result<FindingRow> {
    Ok(FindingRow {
        id: FindingId(row.get(0)?),
        task: TaskId(row.get(1)?),
        round: row.get(2)?,
        n: row.get(3)?,
        agent: AgentId(row.get(4)?),
        label: row.get(5)?,
        path: row.get(6)?,
        body: row.get(7)?,
        body_from: row.get(8)?,
        subagent_type: row.get(9)?,
        started_ms: row.get(10)?,
        created_ms: row.get(11)?,
        tokens: row.get::<_, Option<i64>>(12)?.map(|t| t.max(0) as u64),
        description: row.get(13)?,
        sources: Vec::new(),
    })
}

/// The round still open, if any (at most one is).
pub(super) fn open_round(conn: &Connection) -> Result<Option<RoundRow>, DbError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {ROUND_COLUMNS} FROM research_rounds WHERE closed_ms IS NULL AND queued = 0"
            ),
            [],
            round,
        )
        .optional()?)
}

/// Requests round `n + 1` of `task`: open at once, or queued (`queued` on the row) while any
/// round is open or waiting. Refused for an unknown or archived task or an empty brief.
pub(super) fn start_round(
    conn: &mut Connection,
    task: TaskId,
    brief: &str,
    now: i64,
) -> Result<RoundRow, DbError> {
    let brief = brief.trim();
    if brief.is_empty() {
        return Err(DbError::Refused("a round needs a brief".into()));
    }
    let tx = conn.transaction()?;
    let status: Option<String> = tx
        .query_row("SELECT status FROM tasks WHERE id = ?1", [task.0], |r| {
            r.get(0)
        })
        .optional()?;
    match status.as_deref() {
        None => return Err(DbError::NotFound(format!("there is no task #{}", task.0))),
        Some("archived") => {
            return Err(DbError::Refused(format!("task #{} is archived", task.0)));
        }
        Some(_) => {}
    }
    let queued: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM research_rounds WHERE closed_ms IS NULL)",
        [],
        |r| r.get(0),
    )?;
    let n: u32 = tx.query_row(
        "SELECT COALESCE(MAX(n), 0) + 1 FROM research_rounds WHERE task_id = ?1",
        [task.0],
        |r| r.get(0),
    )?;
    let date = crate::journal::date_of(now)
        .map(|d| d.to_string())
        .unwrap_or_else(|| "undated".into());
    let dir = format!("rounds/{n:02}-{date}");
    tx.execute(
        "INSERT INTO research_rounds (task_id, n, opened_ms, brief, dir, queued) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![task.0, n, now, brief, dir, queued],
    )?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(RoundRow {
        id,
        task,
        n,
        opened_ms: now,
        closed_ms: None,
        brief: brief.to_owned(),
        dir,
        queued,
    })
}

/// The rounds waiting for the open one, oldest request first.
pub(super) fn queued_rounds(conn: &Connection) -> Result<Vec<RoundRow>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ROUND_COLUMNS} FROM research_rounds WHERE queued = 1 AND closed_ms IS NULL \
         ORDER BY id"
    ))?;
    Ok(stmt
        .query_map([], round)?
        .collect::<rusqlite::Result<_>>()?)
}

/// Closes the open round of `task` and opens the next queued one, if any. Resolves to the closed
/// round and the one that opened.
pub(super) fn close_round(
    conn: &mut Connection,
    task: TaskId,
    now: i64,
) -> Result<(RoundRow, Option<RoundRow>), DbError> {
    let tx = conn.transaction()?;
    let open = open_round(&tx)?
        .filter(|r| r.task == task)
        .ok_or_else(|| DbError::NotFound(format!("task #{} has no open round", task.0)))?;
    tx.execute(
        "UPDATE research_rounds SET closed_ms = ?2 WHERE id = ?1",
        params![open.id, now],
    )?;
    let next = promote_next(&tx, now)?;
    tx.commit()?;
    Ok((
        RoundRow {
            closed_ms: Some(now),
            ..open
        },
        next,
    ))
}

/// Opens the oldest queued round, when no round is open.
fn promote_next(conn: &Connection, now: i64) -> Result<Option<RoundRow>, DbError> {
    if open_round(conn)?.is_some() {
        return Ok(None);
    }
    let Some(next) = queued_rounds(conn)?.into_iter().next() else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE research_rounds SET queued = 0, opened_ms = ?2 WHERE id = ?1",
        params![next.id, now],
    )?;
    Ok(Some(RoundRow {
        queued: false,
        opened_ms: now,
        ..next
    }))
}

/// Deletes a queued round of `task`: round `n`, or the one requested last.
pub(super) fn drop_round(
    conn: &Connection,
    task: TaskId,
    n: Option<u32>,
) -> Result<RoundRow, DbError> {
    let mut queued: Vec<RoundRow> = queued_rounds(conn)?
        .into_iter()
        .filter(|r| r.task == task)
        .collect();
    let found = match n {
        Some(n) => match queued.iter().position(|r| r.n == n) {
            Some(i) => queued.swap_remove(i),
            None => {
                return Err(DbError::NotFound(format!(
                    "round {n} of task #{} is not queued (only a queued round can be dropped)",
                    task.0
                )));
            }
        },
        None => queued
            .pop()
            .ok_or_else(|| DbError::NotFound(format!("task #{} has no queued round", task.0)))?,
    };
    conn.execute("DELETE FROM research_rounds WHERE id = ?1", [found.id])?;
    Ok(found)
}

/// Deletes every queued round of `task` (archiving it). Resolves to them.
pub(super) fn drop_queued_for(conn: &Connection, task: TaskId) -> Result<Vec<RoundRow>, DbError> {
    let mine: Vec<RoundRow> = queued_rounds(conn)?
        .into_iter()
        .filter(|r| r.task == task)
        .collect();
    for r in &mine {
        conn.execute("DELETE FROM research_rounds WHERE id = ?1", [r.id])?;
    }
    Ok(mine)
}

/// Files a finding: the row, its sources (one row per locator and content, shared by every
/// finding that reached the same thing) and the links. Resolves to the stored finding and the
/// rows of its sources.
pub(super) fn file_finding(
    conn: &mut Connection,
    new: NewFinding,
    now: i64,
) -> Result<(FindingRow, Vec<SourceRow>), DbError> {
    let Some(task) = new.task else {
        return Err(DbError::Refused(
            "a finding for a task needs the task".into(),
        ));
    };
    let tx = conn.transaction()?;
    let (round_n, round_dir): (Option<u32>, Option<String>) = match new.round {
        Some(id) => {
            let (n, dir) = tx.query_row(
                "SELECT n, dir FROM research_rounds WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            (Some(n), Some(dir))
        }
        None => (None, None),
    };
    let n: u32 = tx.query_row(
        "SELECT COALESCE(MAX(n), 0) + 1 FROM findings WHERE task_id = ?1 AND round_id IS ?2",
        params![task.0, new.round],
        |r| r.get(0),
    )?;
    let file = format!("findings/{n:02}-{}.md", new.label);
    let path = match &round_dir {
        Some(dir) => format!("{dir}/{file}"),
        None => file,
    };
    tx.execute(
        "INSERT INTO findings (task_id, round_id, n, agent_id, label, path, body, body_from, \
         subagent_type, started_ms, created_ms, tokens, description) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            task.0,
            new.round,
            n,
            new.agent.0,
            new.label,
            path,
            new.body,
            new.body_from,
            new.subagent_type,
            new.started_ms,
            now,
            new.tokens.map(|t| t as i64),
            new.description,
        ],
    )?;
    let id = FindingId(tx.last_insert_rowid());
    let (ids, sources) = store_sources(&tx, &new.sources, now, |source, tool_use_id| {
        tx.execute(
            "INSERT OR IGNORE INTO finding_sources (finding_id, source_id, tool_use_id) \
             VALUES (?1, ?2, ?3)",
            params![id.0, source, tool_use_id],
        )
        .map(drop)
    })?;
    tx.commit()?;
    let row = FindingRow {
        id,
        task,
        round: round_n,
        n,
        agent: new.agent,
        label: new.label,
        description: new.description,
        path,
        body: new.body,
        body_from: new.body_from,
        subagent_type: new.subagent_type,
        started_ms: new.started_ms,
        created_ms: now,
        tokens: new.tokens,
        sources: ids,
    };
    Ok((row, sources))
}

/// Stores a finding's sources (one row per locator and content, shared by every finding that
/// reached the same thing), links each through `link(source id, tool use id)`, and resolves to
/// the distinct source ids in order and their rows.
fn store_sources(
    conn: &Connection,
    new: &[NewSource],
    now: i64,
    link: impl Fn(i64, &str) -> rusqlite::Result<()>,
) -> Result<(Vec<SourceId>, Vec<SourceRow>), DbError> {
    let mut ids = Vec::new();
    for s in new {
        conn.execute(
            "INSERT OR IGNORE INTO sources (kind, locator, tool, query, content, cached, is_error, \
             content_hash, fetched_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                s.kind.as_str(),
                s.locator,
                s.tool,
                s.query,
                s.content,
                s.cached,
                s.is_error,
                s.content_hash,
                now
            ],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM sources WHERE locator = ?1 AND content_hash = ?2",
            params![s.locator, s.content_hash],
            |r| r.get(0),
        )?;
        link(id, &s.tool_use_id)?;
        if !ids.contains(&SourceId(id)) {
            ids.push(SourceId(id));
        }
    }
    let mut rows = Vec::new();
    for sid in &ids {
        rows.push(conn.query_row(
            &format!("SELECT {SOURCE_COLUMNS} FROM sources s WHERE s.id = ?1"),
            [sid.0],
            source,
        )?);
    }
    Ok((ids, rows))
}

/// Replaces a finding's body with one from a better source.
pub(super) fn set_finding_body(
    conn: &Connection,
    id: FindingId,
    body: &str,
    from: &str,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE findings SET body = ?2, body_from = ?3 WHERE id = ?1",
        params![id.0, body, from],
    )?;
    Ok(())
}

/// Ids of the tasks that have any research (or a research kind), in id order.
fn research_tasks(conn: &Connection) -> Result<Vec<TaskId>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT task_id FROM research_rounds UNION SELECT task_id FROM findings \
         UNION SELECT task_id FROM reports \
         UNION SELECT id FROM tasks WHERE research_kind IS NOT NULL ORDER BY 1",
    )?;
    let rows = stmt.query_map([], |r| r.get(0).map(TaskId))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The summary counts of every task with research.
pub(super) fn load_summaries(conn: &Connection) -> Result<Vec<SummaryRow>, DbError> {
    research_tasks(conn)?
        .into_iter()
        .map(|t| load_summary(conn, t))
        .collect()
}

/// The summary counts of one task (empty when it has no research).
pub(super) fn load_summary(conn: &Connection, task: TaskId) -> Result<SummaryRow, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ROUND_COLUMNS}, (SELECT COUNT(*) FROM findings f WHERE f.round_id = r.id) \
         FROM research_rounds r WHERE task_id = ?1 ORDER BY n"
    ))?;
    let rounds = stmt
        .query_map([task.0], |r| Ok((round(r)?, r.get::<_, u32>(8)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (findings, quick_findings): (u32, u32) = conn.query_row(
        "SELECT COUNT(*), COUNT(*) - COUNT(round_id) FROM findings WHERE task_id = ?1",
        [task.0],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let sources: u32 = conn.query_row(
        "SELECT COUNT(DISTINCT fs.source_id) FROM finding_sources fs \
         JOIN findings f ON f.id = fs.finding_id WHERE f.task_id = ?1",
        [task.0],
        |r| r.get(0),
    )?;
    let report_ms: Option<i64> = conn.query_row(
        "SELECT MAX(at_ms) FROM reports WHERE task_id = ?1",
        [task.0],
        |r| r.get(0),
    )?;
    let kind = research_kind(conn, task)?;
    Ok(SummaryRow {
        task,
        rounds,
        findings,
        quick_findings,
        sources,
        report_ms,
        kind,
    })
}

/// Everything about one task's research.
pub(super) fn load_task(conn: &Connection, task: TaskId) -> Result<TaskResearch, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ROUND_COLUMNS} FROM research_rounds WHERE task_id = ?1 ORDER BY n"
    ))?;
    let rounds = stmt
        .query_map([task.0], round)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {FINDING_COLUMNS} FROM findings f LEFT JOIN research_rounds r ON r.id = f.round_id \
         WHERE f.task_id = ?1 ORDER BY f.id"
    ))?;
    let mut findings = stmt
        .query_map([task.0], finding)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut links =
        conn.prepare("SELECT source_id FROM finding_sources WHERE finding_id = ?1 ORDER BY rowid")?;
    for f in &mut findings {
        for id in links.query_map([f.id.0], |r| r.get(0).map(SourceId))? {
            let id = id?;
            if !f.sources.contains(&id) {
                f.sources.push(id);
            }
        }
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT {SOURCE_COLUMNS} FROM sources s \
         JOIN finding_sources fs ON fs.source_id = s.id JOIN findings f ON f.id = fs.finding_id \
         WHERE f.task_id = ?1 ORDER BY s.id"
    ))?;
    let sources = stmt
        .query_map([task.0], source)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let report = conn
        .query_row(
            "SELECT at_ms, body FROM reports WHERE task_id = ?1 ORDER BY at_ms DESC, id DESC LIMIT 1",
            [task.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(TaskResearch {
        rounds,
        findings,
        sources,
        report,
    })
}

/// Every task's research in full, for rebuilding missing files at start.
pub(super) fn load_all(conn: &Connection) -> Result<Vec<(TaskId, TaskResearch)>, DbError> {
    research_tasks(conn)?
        .into_iter()
        .map(|t| Ok((t, load_task(conn, t)?)))
        .collect()
}

/// Stores a report (`research_write_report`). The latest by time is the task's `report.md`.
pub(super) fn insert_report(
    conn: &Connection,
    task: TaskId,
    round: Option<i64>,
    body: &str,
    flags: (u32, u32),
    now: i64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO reports (task_id, round_id, at_ms, body, uncited, unlabelled_figures) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![task.0, round, now, body, flags.0, flags.1],
    )?;
    Ok(())
}

/// The task's `research_kind`: `question`, `decision` or none.
pub(super) fn research_kind(conn: &Connection, task: TaskId) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row(
            "SELECT research_kind FROM tasks WHERE id = ?1",
            [task.0],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

/// Sets the task's `research_kind`.
pub(super) fn set_research_kind(
    conn: &Connection,
    task: TaskId,
    kind: Option<&str>,
) -> Result<(), DbError> {
    let changed = conn.execute(
        "UPDATE tasks SET research_kind = ?2 WHERE id = ?1",
        params![task.0, kind],
    )?;
    if changed == 0 {
        return Err(DbError::NotFound(format!("there is no task #{}", task.0)));
    }
    Ok(())
}

// --- unfiled findings -------------------------------------------------------------------------

/// The columns of `unfiled_findings`, in [`unfiled`]'s order.
const UNFILED_COLUMNS: &str = "id, agent_id, label, path, body, body_from, subagent_type, \
     started_ms, created_ms, tokens, description";

/// An unfiled finding as a [`FindingRow`] with `task` 0 (no task), no round and `n` 0.
fn unfiled(row: &Row<'_>) -> rusqlite::Result<FindingRow> {
    Ok(FindingRow {
        id: FindingId(row.get(0)?),
        task: TaskId(0),
        round: None,
        n: 0,
        agent: AgentId(row.get(1)?),
        label: row.get(2)?,
        path: row.get(3)?,
        body: row.get(4)?,
        body_from: row.get(5)?,
        subagent_type: row.get(6)?,
        started_ms: row.get(7)?,
        created_ms: row.get(8)?,
        tokens: row.get::<_, Option<i64>>(9)?.map(|t| t.max(0) as u64),
        description: row.get(10)?,
        sources: Vec::new(),
    })
}

/// The sources of unfiled finding `id`, in the order it reached them.
fn unfiled_sources(conn: &Connection, id: i64) -> Result<(Vec<SourceId>, Vec<SourceRow>), DbError> {
    let mut stmt = conn.prepare(
        "SELECT source_id FROM unfiled_finding_sources WHERE finding_id = ?1 ORDER BY rowid",
    )?;
    let mut ids: Vec<SourceId> = Vec::new();
    for sid in stmt.query_map([id], |r| r.get(0).map(SourceId))? {
        let sid = sid?;
        if !ids.contains(&sid) {
            ids.push(sid);
        }
    }
    let mut rows = Vec::new();
    for sid in &ids {
        rows.push(conn.query_row(
            &format!("SELECT {SOURCE_COLUMNS} FROM sources s WHERE s.id = ?1"),
            [sid.0],
            source,
        )?);
    }
    Ok((ids, rows))
}

/// Stores a finding with no task (`new.task` is ignored). Its file goes to
/// `findings/<id>-<label>.md` under the research root's `unfiled/` folder.
pub(super) fn file_unfiled(
    conn: &mut Connection,
    new: NewFinding,
    now: i64,
) -> Result<(FindingRow, Vec<SourceRow>), DbError> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO unfiled_findings (agent_id, label, description, path, body, body_from, \
         subagent_type, started_ms, created_ms, tokens) \
         VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new.agent.0,
            new.label,
            new.description,
            new.body,
            new.body_from,
            new.subagent_type,
            new.started_ms,
            now,
            new.tokens.map(|t| t as i64),
        ],
    )?;
    let id = tx.last_insert_rowid();
    let path = format!("findings/{id:02}-{}.md", new.label);
    tx.execute(
        "UPDATE unfiled_findings SET path = ?2 WHERE id = ?1",
        params![id, path],
    )?;
    let (ids, sources) = store_sources(&tx, &new.sources, now, |source, tool_use_id| {
        tx.execute(
            "INSERT OR IGNORE INTO unfiled_finding_sources (finding_id, source_id, tool_use_id) \
             VALUES (?1, ?2, ?3)",
            params![id, source, tool_use_id],
        )
        .map(drop)
    })?;
    tx.commit()?;
    let row = FindingRow {
        id: FindingId(id),
        task: TaskId(0),
        round: None,
        n: 0,
        agent: new.agent,
        label: new.label,
        description: new.description,
        path,
        body: new.body,
        body_from: new.body_from,
        subagent_type: new.subagent_type,
        started_ms: new.started_ms,
        created_ms: now,
        tokens: new.tokens,
        sources: ids,
    };
    Ok((row, sources))
}

/// One unfiled finding with its sources; `None` when it is gone (filed or dropped).
pub(super) fn load_unfiled(
    conn: &Connection,
    id: i64,
) -> Result<Option<(FindingRow, Vec<SourceRow>)>, DbError> {
    let found = conn
        .query_row(
            &format!("SELECT {UNFILED_COLUMNS} FROM unfiled_findings WHERE id = ?1"),
            [id],
            unfiled,
        )
        .optional()?;
    let Some(mut row) = found else {
        return Ok(None);
    };
    let (ids, sources) = unfiled_sources(conn, id)?;
    row.sources = ids;
    Ok(Some((row, sources)))
}

/// Every unfiled finding with its sources, for rebuilding missing files at start.
pub(super) fn load_all_unfiled(
    conn: &Connection,
) -> Result<Vec<(FindingRow, Vec<SourceRow>)>, DbError> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM unfiled_findings ORDER BY id")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut out = Vec::new();
    for id in ids {
        if let Some(found) = load_unfiled(conn, id)? {
            out.push(found);
        }
    }
    Ok(out)
}

/// Replaces an unfiled finding's body with one from a better source. Resolves to whether it was
/// still there (it may have been filed or dropped meanwhile).
pub(super) fn set_unfiled_body(
    conn: &Connection,
    id: i64,
    body: &str,
    from: &str,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE unfiled_findings SET body = ?2, body_from = ?3 WHERE id = ?1",
        params![id, body, from],
    )?;
    Ok(changed > 0)
}

/// What leaves the database when an unfiled finding is filed or dropped: the finding, and the
/// sources no other unfiled finding still cites (their files under `unfiled/` can go).
#[derive(Debug)]
pub struct Removed {
    /// The finding as it was.
    pub finding: FindingRow,
    /// Sources only it cited.
    pub orphans: Vec<SourceRow>,
}

fn remove_unfiled(conn: &Connection, id: i64) -> Result<Option<Removed>, DbError> {
    let Some((finding, sources)) = load_unfiled(conn, id)? else {
        return Ok(None);
    };
    conn.execute(
        "DELETE FROM unfiled_finding_sources WHERE finding_id = ?1",
        [id],
    )?;
    conn.execute("DELETE FROM unfiled_findings WHERE id = ?1", [id])?;
    let mut orphans = Vec::new();
    for s in sources {
        let others: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM unfiled_finding_sources WHERE source_id = ?1)",
            [s.id.0],
            |r| r.get(0),
        )?;
        if !others {
            orphans.push(s);
        }
    }
    Ok(Some(Removed { finding, orphans }))
}

/// Deletes an unfiled finding (`d`). `None` when it is already gone.
pub(super) fn drop_unfiled(conn: &mut Connection, id: i64) -> Result<Option<Removed>, DbError> {
    let tx = conn.transaction()?;
    let removed = remove_unfiled(&tx, id)?;
    tx.commit()?;
    Ok(removed)
}

/// Files an unfiled finding under `task` as a quick finding: a `findings` row with the same
/// body and sources and a new id, the unfiled row gone. Resolves to the new finding, its sources
/// and what was removed from `unfiled/`.
pub(super) fn file_unfiled_to_task(
    conn: &mut Connection,
    id: i64,
    task: TaskId,
) -> Result<(FindingRow, Vec<SourceRow>, Removed), DbError> {
    let tx = conn.transaction()?;
    let status: Option<String> = tx
        .query_row("SELECT status FROM tasks WHERE id = ?1", [task.0], |r| {
            r.get(0)
        })
        .optional()?;
    match status.as_deref() {
        None => return Err(DbError::NotFound(format!("there is no task #{}", task.0))),
        Some("archived") => {
            return Err(DbError::Refused(format!("task #{} is archived", task.0)));
        }
        Some(_) => {}
    }
    let (old, sources) = load_unfiled(&tx, id)?
        .ok_or_else(|| DbError::NotFound("that finding is already filed or dropped".into()))?;
    let n: u32 = tx.query_row(
        "SELECT COALESCE(MAX(n), 0) + 1 FROM findings WHERE task_id = ?1 AND round_id IS NULL",
        [task.0],
        |r| r.get(0),
    )?;
    let path = format!("findings/{n:02}-{}.md", old.label);
    tx.execute(
        "INSERT INTO findings (task_id, round_id, n, agent_id, label, path, body, body_from, \
         subagent_type, started_ms, created_ms, tokens, description) \
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            task.0,
            n,
            old.agent.0,
            old.label,
            path,
            old.body,
            old.body_from,
            old.subagent_type,
            old.started_ms,
            old.created_ms,
            old.tokens.map(|t| t as i64),
            old.description,
        ],
    )?;
    let new_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT OR IGNORE INTO finding_sources (finding_id, source_id, tool_use_id) \
         SELECT ?1, source_id, tool_use_id FROM unfiled_finding_sources WHERE finding_id = ?2",
        params![new_id, id],
    )?;
    let removed = remove_unfiled(&tx, id)?
        .ok_or_else(|| DbError::NotFound("that finding is already filed or dropped".into()))?;
    tx.commit()?;
    let row = FindingRow {
        id: FindingId(new_id),
        task,
        round: None,
        n,
        path,
        ..old
    };
    Ok((row, sources, removed))
}

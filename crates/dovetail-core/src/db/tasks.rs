//! Queries on a project's `state.db`: tasks, their dependencies and relations, and decisions.

use std::collections::BTreeMap;

use dovetail_api::{
    Alternative, Bucket, Decision, DecisionAuthor, DecisionId, NewDecision, NewTask, PLevel, Task,
    TaskChange, TaskId, TaskStatus,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::DbError;

fn status_to_text(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "todo",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Done => "done",
        TaskStatus::Archived => "archived",
    }
}

fn status_from_text(text: &str) -> Result<TaskStatus, DbError> {
    Ok(match text {
        "todo" => TaskStatus::Todo,
        "in_progress" => TaskStatus::InProgress,
        "done" => TaskStatus::Done,
        "archived" => TaskStatus::Archived,
        other => return Err(DbError::Corrupt(format!("task status {other:?}"))),
    })
}

fn bucket_to_text(bucket: Bucket) -> &'static str {
    match bucket {
        Bucket::Now => "now",
        Bucket::Next => "next",
        Bucket::Later => "later",
    }
}

fn bucket_from_text(text: &str) -> Result<Bucket, DbError> {
    Ok(match text {
        "now" => Bucket::Now,
        "next" => Bucket::Next,
        "later" => Bucket::Later,
        other => return Err(DbError::Corrupt(format!("task bucket {other:?}"))),
    })
}

fn p_level_to_int(level: PLevel) -> i64 {
    match level {
        PLevel::P0 => 0,
        PLevel::P1 => 1,
        PLevel::P2 => 2,
        PLevel::P3 => 3,
    }
}

fn p_level_from_int(n: i64) -> Result<PLevel, DbError> {
    Ok(match n {
        0 => PLevel::P0,
        1 => PLevel::P1,
        2 => PLevel::P2,
        3 => PLevel::P3,
        other => return Err(DbError::Corrupt(format!("p-level {other}"))),
    })
}

/// Every task, with dependencies and relations filled in, in id order.
pub(super) fn load_tasks(conn: &mut Connection) -> Result<Vec<Task>, DbError> {
    let mut deps: BTreeMap<i64, Vec<TaskId>> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT task_id, depends_on FROM task_deps ORDER BY depends_on")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (task, dep) = row?;
        deps.entry(task).or_default().push(TaskId(dep));
    }
    let mut related: BTreeMap<i64, Vec<TaskId>> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT a, b FROM task_related ORDER BY a, b")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (a, b) = row?;
        related.entry(a).or_default().push(TaskId(b));
        related.entry(b).or_default().push(TaskId(a));
    }
    let mut stmt = conn.prepare(
        "SELECT id, title, status, p_level, bucket, sort_order, notes, pr, follow_up_of, \
         coordinator_created FROM tasks ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, Option<u32>>(7)?,
            r.get::<_, Option<i64>>(8)?,
            r.get::<_, bool>(9)?,
        ))
    })?;
    let mut tasks = Vec::new();
    for row in rows {
        let (id, title, status, p_level, bucket, order, notes, pr, follow_up_of, created) = row?;
        let mut related_ids = related.remove(&id).unwrap_or_default();
        related_ids.sort();
        tasks.push(Task {
            id: TaskId(id),
            title,
            status: status_from_text(&status)?,
            p_level: p_level.map(p_level_from_int).transpose()?,
            bucket: bucket.as_deref().map(bucket_from_text).transpose()?,
            order,
            deps: deps.remove(&id).unwrap_or_default(),
            notes,
            pr,
            follow_up_of: follow_up_of.map(TaskId),
            coordinator_created: created,
            related: related_ids,
        });
    }
    Ok(tasks)
}

/// The tasks with these ids, in id order. Ids that do not exist are skipped.
fn tasks_by_ids(conn: &mut Connection, ids: &[TaskId]) -> Result<Vec<Task>, DbError> {
    let mut tasks = load_tasks(conn)?;
    tasks.retain(|t| ids.contains(&t.id));
    // Keep the caller's order, which puts the main task first.
    tasks.sort_by_key(|t| ids.iter().position(|id| *id == t.id));
    Ok(tasks)
}

fn exists(tx: &Transaction<'_>, id: TaskId) -> Result<bool, DbError> {
    Ok(tx
        .query_row("SELECT 1 FROM tasks WHERE id = ?1", [id.0], |_| Ok(()))
        .optional()?
        .is_some())
}

fn next_order(tx: &Transaction<'_>, bucket: Option<Bucket>) -> Result<i64, DbError> {
    Ok(tx.query_row(
        "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM tasks WHERE bucket IS ?1",
        [bucket.map(bucket_to_text)],
        |r| r.get(0),
    )?)
}

fn relate(tx: &Transaction<'_>, a: TaskId, b: TaskId, on: bool) -> Result<bool, DbError> {
    if a == b || !exists(tx, a)? || !exists(tx, b)? {
        return Ok(false);
    }
    let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
    let changed = if on {
        tx.execute(
            "INSERT OR IGNORE INTO task_related (a, b) VALUES (?1, ?2)",
            [lo, hi],
        )?
    } else {
        tx.execute("DELETE FROM task_related WHERE a = ?1 AND b = ?2", [lo, hi])?
    };
    Ok(changed > 0)
}

/// Inserts a task. Resolves to the new task, then the tasks it got related to.
pub(super) fn create_task(
    conn: &mut Connection,
    new: NewTask,
    coordinator_created: bool,
    now_ms: i64,
) -> Result<Vec<Task>, DbError> {
    let tx = conn.transaction()?;
    let order = next_order(&tx, new.bucket)?;
    tx.execute(
        "INSERT INTO tasks (title, status, p_level, bucket, sort_order, notes, pr, follow_up_of, \
         coordinator_created, created_ms, updated_ms) VALUES (?1, 'todo', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            new.title,
            new.p_level.map(p_level_to_int),
            new.bucket.map(bucket_to_text),
            order,
            new.notes,
            new.pr,
            new.follow_up_of.map(|t| t.0),
            coordinator_created,
            now_ms
        ],
    )?;
    let id = TaskId(tx.last_insert_rowid());
    let mut touched = vec![id];
    for other in new.related {
        if relate(&tx, id, other, true)? {
            touched.push(other);
        }
    }
    tx.commit()?;
    tasks_by_ids(conn, &touched)
}

/// Applies a patch in order. Resolves to every task it changed.
pub(super) fn update_task(
    conn: &mut Connection,
    id: TaskId,
    patch: Vec<TaskChange>,
    now_ms: i64,
) -> Result<Vec<Task>, DbError> {
    let tx = conn.transaction()?;
    if !exists(&tx, id)? {
        return Err(DbError::NotFound(format!("no task {}", id.0)));
    }
    let mut touched = vec![id];
    for change in patch {
        match change {
            TaskChange::Title(title) => {
                tx.execute(
                    "UPDATE tasks SET title = ?2 WHERE id = ?1",
                    params![id.0, title],
                )?;
            }
            TaskChange::Status(status) => {
                tx.execute(
                    "UPDATE tasks SET status = ?2 WHERE id = ?1",
                    params![id.0, status_to_text(status)],
                )?;
            }
            TaskChange::PLevel(level) => {
                tx.execute(
                    "UPDATE tasks SET p_level = ?2 WHERE id = ?1",
                    params![id.0, level.map(p_level_to_int)],
                )?;
            }
            TaskChange::Bucket(bucket) => {
                let current: Option<String> =
                    tx.query_row("SELECT bucket FROM tasks WHERE id = ?1", [id.0], |r| {
                        r.get(0)
                    })?;
                let new_text = bucket.map(bucket_to_text);
                if current.as_deref() != new_text {
                    let order = next_order(&tx, bucket)?;
                    tx.execute(
                        "UPDATE tasks SET bucket = ?2, sort_order = ?3 WHERE id = ?1",
                        params![id.0, new_text, order],
                    )?;
                }
            }
            TaskChange::Notes(notes) => {
                tx.execute(
                    "UPDATE tasks SET notes = ?2 WHERE id = ?1",
                    params![id.0, notes],
                )?;
            }
            TaskChange::Deps(deps) => {
                tx.execute("DELETE FROM task_deps WHERE task_id = ?1", [id.0])?;
                for dep in deps.into_iter().filter(|d| *d != id) {
                    tx.execute(
                        "INSERT OR IGNORE INTO task_deps (task_id, depends_on) VALUES (?1, ?2)",
                        [id.0, dep.0],
                    )?;
                }
            }
            TaskChange::Pr(pr) => {
                tx.execute("UPDATE tasks SET pr = ?2 WHERE id = ?1", params![id.0, pr])?;
            }
            TaskChange::Seen => {
                tx.execute(
                    "UPDATE tasks SET coordinator_created = 0 WHERE id = ?1",
                    [id.0],
                )?;
            }
            TaskChange::Relate(other) => {
                if relate(&tx, id, other, true)? {
                    touched.push(other);
                }
            }
            TaskChange::Unrelate(other) => {
                if relate(&tx, id, other, false)? {
                    touched.push(other);
                }
            }
        }
    }
    tx.execute(
        "UPDATE tasks SET updated_ms = ?2 WHERE id = ?1",
        params![id.0, now_ms],
    )?;
    tx.commit()?;
    tasks_by_ids(conn, &touched)
}

/// Moves a task within its bucket and renumbers the bucket from 0. Resolves to the tasks whose
/// position changed.
pub(super) fn reorder(conn: &mut Connection, id: TaskId, delta: i32) -> Result<Vec<Task>, DbError> {
    let tx = conn.transaction()?;
    if !exists(&tx, id)? {
        return Err(DbError::NotFound(format!("no task {}", id.0)));
    }
    let bucket: Option<String> =
        tx.query_row("SELECT bucket FROM tasks WHERE id = ?1", [id.0], |r| {
            r.get(0)
        })?;
    let mut peers: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT id, sort_order FROM tasks WHERE bucket IS ?1 AND status != 'archived' \
             ORDER BY sort_order, id",
        )?;
        stmt.query_map([&bucket], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let Some(from) = peers.iter().position(|(peer, _)| *peer == id.0) else {
        return Ok(Vec::new());
    };
    let to = (from as i64 + i64::from(delta)).clamp(0, peers.len() as i64 - 1) as usize;
    let moved = peers.remove(from);
    peers.insert(to, moved);
    let mut changed = Vec::new();
    for (order, (peer, old)) in peers.into_iter().enumerate() {
        if old != order as i64 {
            tx.execute(
                "UPDATE tasks SET sort_order = ?2 WHERE id = ?1",
                [peer, order as i64],
            )?;
            changed.push(TaskId(peer));
        }
    }
    tx.commit()?;
    tasks_by_ids(conn, &changed)
}

fn author_to_text(by: &DecisionAuthor) -> String {
    match by {
        DecisionAuthor::User => "user".into(),
        DecisionAuthor::Agent(agent) => agent.0.clone(),
    }
}

fn author_from_text(text: String) -> DecisionAuthor {
    if text == "user" {
        DecisionAuthor::User
    } else {
        DecisionAuthor::Agent(dovetail_api::AgentId(text))
    }
}

/// Appends a decision-log entry.
pub(super) fn log_decision(
    conn: &mut Connection,
    new: NewDecision,
    by: DecisionAuthor,
    now_ms: i64,
) -> Result<Decision, DbError> {
    let tx = conn.transaction()?;
    if !exists(&tx, new.task)? {
        return Err(DbError::NotFound(format!("no task {}", new.task.0)));
    }
    let alternatives = serde_json::to_string(&new.alternatives)
        .map_err(|e| DbError::Corrupt(format!("alternatives: {e}")))?;
    tx.execute(
        "INSERT INTO decisions (task_id, at_ms, author, text, why, alternatives, supersedes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new.task.0,
            now_ms,
            author_to_text(&by),
            new.text,
            new.why,
            alternatives,
            new.supersedes.map(|d| d.0 as i64)
        ],
    )?;
    let id = DecisionId(tx.last_insert_rowid() as u64);
    tx.commit()?;
    Ok(Decision {
        id,
        task: new.task,
        at_ms: now_ms,
        by,
        text: new.text,
        why: new.why,
        alternatives: new.alternatives,
        supersedes: new.supersedes,
    })
}

/// Every decision, oldest first.
pub(super) fn load_decisions(conn: &mut Connection) -> Result<Vec<Decision>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, at_ms, author, text, why, alternatives, supersedes \
         FROM decisions ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, Option<i64>>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, task, at_ms, author, text, why, alternatives, supersedes) = row?;
        let alternatives: Vec<Alternative> = serde_json::from_str(&alternatives)
            .map_err(|e| DbError::Corrupt(format!("decision {id} alternatives: {e}")))?;
        out.push(Decision {
            id: DecisionId(id as u64),
            task: TaskId(task),
            at_ms,
            by: author_from_text(author),
            text,
            why,
            alternatives,
            supersedes: supersedes.map(|d| DecisionId(d as u64)),
        });
    }
    Ok(out)
}

//! Queries on a project's `state.db`: efforts, their acceptance criteria, and which effort and
//! criterion a task belongs to.

use dovetail_api::tracking::{
    effort::{
        Criterion, CriterionId, CriterionState, Effort, EffortChange, EffortId, EffortStatus,
        NewEffort, UNSORTED_EFFORT, WhereWasI,
    },
    task::{Task, TaskId},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::storage::DbError;
use crate::storage::sessions::{from_text, to_text};

fn no_effort(id: EffortId) -> DbError {
    DbError::NotFound(format!("no effort {}", id.0))
}

fn no_criterion(id: CriterionId) -> DbError {
    DbError::NotFound(format!("no criterion {}", id.0))
}

/// Every effort with its criteria, in id order.
pub(crate) fn load_efforts(conn: &Connection) -> Result<Vec<Effort>, DbError> {
    efforts_where(conn, None)
}

/// One effort with its criteria.
fn load_effort(conn: &Connection, id: EffortId) -> Result<Effort, DbError> {
    efforts_where(conn, Some(id))?
        .pop()
        .ok_or_else(|| no_effort(id))
}

fn efforts_where(conn: &Connection, only: Option<EffortId>) -> Result<Vec<Effort>, DbError> {
    let only = only.map(|e| e.0);
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, question_or_goal, status, where_was_i FROM efforts \
         WHERE ?1 IS NULL OR id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([only], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut efforts = Vec::new();
    for row in rows {
        let (id, name, kind, question_or_goal, status, card) = row?;
        let where_was_i: WhereWasI = serde_json::from_str(&card)
            .map_err(|e| DbError::Corrupt(format!("effort {id} where_was_i: {e}")))?;
        efforts.push(Effort {
            id: EffortId(id),
            name,
            kind: from_text("effort kind", &kind)?,
            question_or_goal,
            status: from_text("effort status", &status)?,
            where_was_i,
            criteria: Vec::new(),
        });
    }
    let mut stmt = conn.prepare(
        "SELECT id, effort_id, text, state, evidence FROM criteria \
         WHERE ?1 IS NULL OR effort_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([only], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (id, effort, text, state, evidence) = row?;
        let criterion = Criterion {
            id: CriterionId(id),
            effort: EffortId(effort),
            text,
            state: from_text("criterion state", &state)?,
            evidence,
        };
        if let Some(owner) = efforts.iter_mut().find(|e| e.id.0 == effort) {
            owner.criteria.push(criterion);
        }
    }
    Ok(efforts)
}

fn effort_exists(conn: &Connection, id: EffortId) -> Result<bool, DbError> {
    Ok(conn
        .query_row("SELECT 1 FROM efforts WHERE id = ?1", [id.0], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Refuses a name that is empty or that another effort has. `name` is already in its stored
/// form (`dovetail_api::tracking::effort::effort_name`).
fn check_name(conn: &Connection, name: &str, own: Option<EffortId>) -> Result<(), DbError> {
    if name.is_empty() {
        return Err(DbError::Refused(
            "an effort name needs at least one letter or digit".into(),
        ));
    }
    let taken: Option<i64> = conn
        .query_row("SELECT id FROM efforts WHERE name = ?1", [name], |r| {
            r.get(0)
        })
        .optional()?;
    match taken {
        Some(id) if Some(EffortId(id)) != own => Err(DbError::Refused(format!(
            "there is already an effort named {name}"
        ))),
        _ => Ok(()),
    }
}

/// Inserts an effort. `new.name` is already in its stored form.
pub(crate) fn create_effort(
    conn: &mut Connection,
    new: NewEffort,
    now_ms: i64,
) -> Result<Effort, DbError> {
    check_name(conn, &new.name, None)?;
    conn.execute(
        "INSERT INTO efforts (name, kind, question_or_goal, created_ms, updated_ms) \
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![new.name, to_text(&new.kind)?, new.question_or_goal, now_ms],
    )?;
    load_effort(conn, EffortId(conn.last_insert_rowid()))
}

/// Renames an effort. `name` is already in its stored form. Unsorted keeps its name.
pub(crate) fn rename_effort(
    conn: &mut Connection,
    id: EffortId,
    name: String,
    now_ms: i64,
) -> Result<Effort, DbError> {
    if !effort_exists(conn, id)? {
        return Err(no_effort(id));
    }
    if id == UNSORTED_EFFORT {
        return Err(DbError::Refused("Unsorted cannot be renamed".into()));
    }
    check_name(conn, &name, Some(id))?;
    conn.execute(
        "UPDATE efforts SET name = ?2, updated_ms = ?3 WHERE id = ?1",
        params![id.0, name, now_ms],
    )?;
    load_effort(conn, id)
}

/// Applies a patch in order. Unsorted cannot be closed.
pub(crate) fn update_effort(
    conn: &mut Connection,
    id: EffortId,
    patch: Vec<EffortChange>,
    now_ms: i64,
) -> Result<Effort, DbError> {
    let tx = conn.transaction()?;
    if !effort_exists(&tx, id)? {
        return Err(no_effort(id));
    }
    for change in patch {
        match change {
            EffortChange::Kind(kind) => {
                tx.execute(
                    "UPDATE efforts SET kind = ?2 WHERE id = ?1",
                    params![id.0, to_text(&kind)?],
                )?;
            }
            EffortChange::QuestionOrGoal(text) => {
                tx.execute(
                    "UPDATE efforts SET question_or_goal = ?2 WHERE id = ?1",
                    params![id.0, text],
                )?;
            }
            EffortChange::Status(status) => {
                if id == UNSORTED_EFFORT && status == EffortStatus::Closed {
                    return Err(DbError::Refused("Unsorted cannot be closed".into()));
                }
                tx.execute(
                    "UPDATE efforts SET status = ?2 WHERE id = ?1",
                    params![id.0, to_text(&status)?],
                )?;
            }
        }
    }
    tx.execute(
        "UPDATE efforts SET updated_ms = ?2 WHERE id = ?1",
        params![id.0, now_ms],
    )?;
    tx.commit()?;
    load_effort(conn, id)
}

/// Replaces an effort's "where was I" card.
pub(crate) fn set_where_was_i(
    conn: &mut Connection,
    id: EffortId,
    card: &WhereWasI,
    now_ms: i64,
) -> Result<Effort, DbError> {
    let json =
        serde_json::to_string(card).map_err(|e| DbError::Corrupt(format!("where_was_i: {e}")))?;
    let changed = conn.execute(
        "UPDATE efforts SET where_was_i = ?2, updated_ms = ?3 WHERE id = ?1",
        params![id.0, json, now_ms],
    )?;
    if changed == 0 {
        return Err(no_effort(id));
    }
    load_effort(conn, id)
}

/// Adds an open criterion to an effort.
pub(crate) fn add_criterion(
    conn: &mut Connection,
    effort: EffortId,
    text: String,
    now_ms: i64,
) -> Result<Effort, DbError> {
    if !effort_exists(conn, effort)? {
        return Err(no_effort(effort));
    }
    if text.trim().is_empty() {
        return Err(DbError::Refused("a criterion needs some text".into()));
    }
    conn.execute(
        "INSERT INTO criteria (effort_id, text, created_ms) VALUES (?1, ?2, ?3)",
        params![effort.0, text.trim(), now_ms],
    )?;
    load_effort(conn, effort)
}

/// What to do to a criterion's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CriterionStep {
    /// An agent (or the user) says it is met: open or claimed becomes claimed, with evidence.
    Claim {
        /// What the claim rests on.
        evidence: String,
    },
    /// The user approves it: it becomes met.
    Approve,
    /// The user sends it back: claimed or met becomes open; the evidence stays on record.
    Return,
}

/// Moves a criterion from one state to the next, refusing steps that make no sense. Resolves to
/// its effort.
pub(crate) fn step_criterion(
    conn: &mut Connection,
    id: CriterionId,
    step: CriterionStep,
) -> Result<Effort, DbError> {
    let tx = conn.transaction()?;
    let found: Option<(i64, String)> = tx
        .query_row(
            "SELECT effort_id, state FROM criteria WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((effort, state)) = found else {
        return Err(no_criterion(id));
    };
    let state: CriterionState = from_text("criterion state", &state)?;
    let refused = |what: &str| Err(DbError::Refused(format!("criterion {} {what}", id.0)));
    match step {
        CriterionStep::Claim { evidence } => {
            if state == CriterionState::Met {
                return refused("is already met");
            }
            if evidence.trim().is_empty() {
                return refused("cannot be claimed without evidence");
            }
            tx.execute(
                "UPDATE criteria SET state = 'claimed', evidence = ?2 WHERE id = ?1",
                params![id.0, evidence.trim()],
            )?;
        }
        CriterionStep::Approve => {
            tx.execute("UPDATE criteria SET state = 'met' WHERE id = ?1", [id.0])?;
        }
        CriterionStep::Return => {
            if state == CriterionState::Open {
                return refused("is open: there is nothing to send back");
            }
            tx.execute("UPDATE criteria SET state = 'open' WHERE id = ?1", [id.0])?;
        }
    }
    tx.commit()?;
    load_effort(conn, EffortId(effort))
}

/// Checks that a task may be put in `effort` serving `criterion`: the effort exists and is open,
/// and the criterion is one of its own.
pub(crate) fn check_place(
    tx: &Transaction<'_>,
    effort: EffortId,
    criterion: Option<CriterionId>,
) -> Result<(), DbError> {
    let status: Option<String> = tx
        .query_row(
            "SELECT status FROM efforts WHERE id = ?1",
            [effort.0],
            |r| r.get(0),
        )
        .optional()?;
    let Some(status) = status else {
        return Err(no_effort(effort));
    };
    if from_text::<EffortStatus>("effort status", &status)? == EffortStatus::Closed {
        return Err(DbError::Refused(format!(
            "effort {} is closed: open it again first",
            effort.0
        )));
    }
    if let Some(criterion) = criterion {
        let owner: Option<i64> = tx
            .query_row(
                "SELECT effort_id FROM criteria WHERE id = ?1",
                [criterion.0],
                |r| r.get(0),
            )
            .optional()?;
        match owner {
            None => return Err(no_criterion(criterion)),
            Some(owner) if owner != effort.0 => {
                return Err(DbError::Refused(format!(
                    "criterion {} belongs to effort {owner}, not effort {}",
                    criterion.0, effort.0
                )));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Puts a task in an effort, serving `criterion` or none. Resolves to the task.
pub(crate) fn move_task(
    conn: &mut Connection,
    id: TaskId,
    effort: EffortId,
    criterion: Option<CriterionId>,
    now_ms: i64,
) -> Result<Vec<Task>, DbError> {
    let tx = conn.transaction()?;
    check_place(&tx, effort, criterion)?;
    let changed = tx.execute(
        "UPDATE tasks SET effort_id = ?2, criterion_id = ?3, updated_ms = ?4 WHERE id = ?1",
        params![id.0, effort.0, criterion.map(|c| c.0), now_ms],
    )?;
    if changed == 0 {
        return Err(DbError::NotFound(format!("no task {}", id.0)));
    }
    tx.commit()?;
    super::tasks::tasks_by_ids(conn, &[id])
}

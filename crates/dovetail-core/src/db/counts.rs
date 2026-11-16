//! What Home shows about a project, read from its `state.db` without migrating or writing it
//! (ADR 0021): a project that is not open may be on an older schema, or open in another process.

use std::path::Path;

use dovetail_api::{ProjectCounts, UNSORTED_EFFORT};
use rusqlite::{Connection, OpenFlags};

use super::DbError;

/// The open slices of the `state.db` at `path` that work in repo `repo`; `first`: it is the
/// project's first repo, which slices with no repo stored (from before ADR 0021) use. A missing
/// file has none.
pub(super) fn slices_in_repo(path: &Path, repo: i64, first: bool) -> Result<Vec<String>, DbError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT name FROM agents WHERE kind = 'slice' AND closed_ms IS NULL \
         AND (repo_id = ?1 OR (repo_id IS NULL AND ?2)) ORDER BY name",
    )?;
    let names = stmt
        .query_map(rusqlite::params![repo, first], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    Ok(names)
}

/// The counts of the `state.db` at `path`. A missing file is all zero; so is one too old to have
/// efforts (the caller logs the error and shows zero).
pub(super) fn read(path: &Path) -> Result<ProjectCounts, DbError> {
    if !path.exists() {
        return Ok(ProjectCounts::default());
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let count = |sql: &str| -> Result<u32, DbError> {
        Ok(conn.query_row(sql, [UNSORTED_EFFORT.0], |r| r.get::<_, u32>(0))?)
    };
    Ok(ProjectCounts {
        efforts: count("SELECT COUNT(*) FROM efforts WHERE status = 'open' AND id != ?1")?,
        running: count(
            "SELECT COUNT(DISTINCT e.id) FROM efforts e \
             JOIN tasks t ON t.effort_id = e.id \
             JOIN agents a ON a.task_id = t.id \
             WHERE e.status = 'open' AND e.id != ?1 AND a.kind = 'slice' AND a.closed_ms IS NULL",
        )?,
        needs_you: count(
            "SELECT (SELECT COUNT(*) FROM inbox_items WHERE resolved_ms IS NULL) \
             + (SELECT COUNT(*) FROM criteria WHERE state = 'claimed') + 0 * ?1",
        )?,
    })
}

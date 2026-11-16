//! What Home shows about a project, read from its `state.db` without migrating or writing it
//! (ADR 0021): a project that is not open may be on an older schema, or open in another process.

use std::path::Path;

use dovetail_api::{ProjectCounts, UNSORTED_EFFORT};
use rusqlite::{Connection, OpenFlags};

use super::DbError;

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

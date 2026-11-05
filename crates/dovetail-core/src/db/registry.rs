//! Queries on `registry.db`.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::DbError;
use crate::projects::Project;

const COLUMNS: &str = "id, name, repo_path, git_common_dir, last_opened_ms";

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        repo_path: PathBuf::from(row.get::<_, String>(2)?),
        git_common_dir: PathBuf::from(row.get::<_, String>(3)?),
        last_opened_ms: row.get(4)?,
    })
}

pub(super) fn find(conn: &Connection, git_common_dir: &Path) -> Result<Option<Project>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM projects WHERE git_common_dir = ?1"),
            [git_common_dir.to_string_lossy()],
            row_to_project,
        )
        .optional()?)
}

pub(super) fn list(conn: &Connection) -> Result<Vec<Project>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM projects ORDER BY COALESCE(last_opened_ms, 0) DESC, id"
    ))?;
    let rows = stmt.query_map([], row_to_project)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(super) fn register(
    conn: &mut Connection,
    slug: &str,
    name: &str,
    repo_path: &Path,
    git_common_dir: &Path,
    now_ms: i64,
) -> Result<Project, DbError> {
    let tx = conn.transaction()?;
    if let Some(existing) = find(&tx, git_common_dir)? {
        return Ok(existing);
    }
    let mut id = slug.to_owned();
    let mut n = 2;
    while tx
        .query_row("SELECT 1 FROM projects WHERE id = ?1", [&id], |_| Ok(()))
        .optional()?
        .is_some()
    {
        id = format!("{slug}-{n}");
        n += 1;
    }
    tx.execute(
        &format!("INSERT INTO projects ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
        params![
            id,
            name,
            repo_path.to_string_lossy(),
            git_common_dir.to_string_lossy(),
            now_ms
        ],
    )?;
    tx.commit()?;
    Ok(Project {
        id,
        name: name.to_owned(),
        repo_path: repo_path.to_owned(),
        git_common_dir: git_common_dir.to_owned(),
        last_opened_ms: Some(now_ms),
    })
}

pub(super) fn touch(conn: &Connection, id: &str, now_ms: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE projects SET last_opened_ms = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

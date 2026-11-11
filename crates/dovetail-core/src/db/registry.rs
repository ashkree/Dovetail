//! Queries on `registry.db`.

use std::path::{Path, PathBuf};

use dovetail_api::ProjectKind;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::DbError;
use crate::projects::Project;

const COLUMNS: &str = "id, name, kind, repo_path, git_common_dir, last_opened_ms";

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    let kind: String = row.get(2)?;
    let kind = ProjectKind::parse(&kind).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            format!("unknown project kind {kind}").into(),
        )
    })?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        kind,
        repo_path: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
        git_common_dir: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
        last_opened_ms: row.get(5)?,
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

/// A code project without git, by its folder.
pub(super) fn find_folder(conn: &Connection, path: &Path) -> Result<Option<Project>, DbError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM projects \
                 WHERE kind = 'code' AND git_common_dir IS NULL AND repo_path = ?1"
            ),
            [path.to_string_lossy()],
            row_to_project,
        )
        .optional()?)
}

/// The project with this id.
pub(super) fn get(conn: &Connection, id: &str) -> Result<Option<Project>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM projects WHERE id = ?1"),
            [id],
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

/// `slug`, numbered (`slug-2`, ...) when another project has it.
fn free_id(conn: &Connection, slug: &str) -> Result<String, DbError> {
    let mut id = slug.to_owned();
    let mut n = 2;
    while conn
        .query_row("SELECT 1 FROM projects WHERE id = ?1", [&id], |_| Ok(()))
        .optional()?
        .is_some()
    {
        id = format!("{slug}-{n}");
        n += 1;
    }
    Ok(id)
}

pub(super) fn register(
    conn: &mut Connection,
    slug: &str,
    name: &str,
    repo_path: &Path,
    git_common_dir: Option<&Path>,
    now_ms: i64,
) -> Result<Project, DbError> {
    let tx = conn.transaction()?;
    let existing = match git_common_dir {
        Some(dir) => find(&tx, dir)?,
        None => find_folder(&tx, repo_path)?,
    };
    if let Some(existing) = existing {
        return Ok(existing);
    }
    let id = free_id(&tx, slug)?;
    tx.execute(
        &format!("INSERT INTO projects ({COLUMNS}) VALUES (?1, ?2, 'code', ?3, ?4, ?5)"),
        params![
            id,
            name,
            repo_path.to_string_lossy(),
            git_common_dir.map(|d| d.to_string_lossy().into_owned()),
            now_ms
        ],
    )?;
    tx.commit()?;
    Ok(Project {
        id,
        name: name.to_owned(),
        kind: ProjectKind::Code,
        repo_path: Some(repo_path.to_owned()),
        git_common_dir: git_common_dir.map(Path::to_path_buf),
        last_opened_ms: Some(now_ms),
    })
}

/// Registers a research project: no repo, no git dir. `None` when a research project of this
/// name exists already.
pub(super) fn register_research(
    conn: &mut Connection,
    slug: &str,
    name: &str,
    now_ms: i64,
) -> Result<Option<Project>, DbError> {
    let tx = conn.transaction()?;
    let taken = tx
        .query_row(
            "SELECT 1 FROM projects WHERE kind = 'research' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if taken {
        return Ok(None);
    }
    let id = free_id(&tx, slug)?;
    tx.execute(
        &format!("INSERT INTO projects ({COLUMNS}) VALUES (?1, ?2, 'research', NULL, NULL, ?3)"),
        params![id, name, now_ms],
    )?;
    tx.commit()?;
    Ok(Some(Project {
        id,
        name: name.to_owned(),
        kind: ProjectKind::Research,
        repo_path: None,
        git_common_dir: None,
        last_opened_ms: Some(now_ms),
    }))
}

/// Records that a folder project is now a git repo.
pub(super) fn set_git_dir(
    conn: &Connection,
    id: &str,
    git_common_dir: &Path,
) -> Result<Project, DbError> {
    conn.execute(
        "UPDATE projects SET git_common_dir = ?2 WHERE id = ?1 AND kind = 'code'",
        params![id, git_common_dir.to_string_lossy()],
    )?;
    get(conn, id)?.ok_or_else(|| DbError::NotFound(format!("no project {id}")))
}

pub(super) fn touch(conn: &Connection, id: &str, now_ms: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE projects SET last_opened_ms = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

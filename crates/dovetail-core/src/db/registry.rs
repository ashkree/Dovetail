//! Queries on `registry.db`: projects and their repos (ADR 0021).

use std::path::{Path, PathBuf};

use dovetail_api::{project::RepoId, tangent::TANGENTS_PROJECT};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::DbError;
use crate::projects::{Project, Repo};

/// The model list saved for `driver`.
pub(crate) fn driver_models(conn: &Connection, driver: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row(
            "SELECT models FROM driver_models WHERE driver = ?1",
            [driver],
            |r| r.get(0),
        )
        .optional()?)
}

/// Saves (or replaces) the model list of `driver`.
pub(crate) fn save_driver_models(
    conn: &Connection,
    driver: &str,
    models: &str,
    now: i64,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO driver_models (driver, models, saved_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(driver) DO UPDATE SET models = ?2, saved_ms = ?3",
        params![driver, models, now],
    )?;
    Ok(())
}

const COLUMNS: &str = "id, name, account, last_opened_ms";
const REPO_COLUMNS: &str = "id, name, path, git_common_dir";

/// Tangents' display name.
pub(crate) const TANGENTS_NAME: &str = "Tangents";

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        account: row.get(2)?,
        repos: Vec::new(),
        last_opened_ms: row.get(3)?,
    })
}

fn row_to_repo(row: &Row<'_>) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: RepoId(row.get(0)?),
        name: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        git_common_dir: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
    })
}

/// `project` with its repos filled in, in the order they were added.
fn with_repos(conn: &Connection, mut project: Project) -> Result<Project, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REPO_COLUMNS} FROM repos WHERE project_id = ?1 ORDER BY id"
    ))?;
    project.repos = stmt
        .query_map([&project.id], row_to_repo)?
        .collect::<Result<_, _>>()?;
    Ok(project)
}

/// The project that owns the repo matching `filter` (an SQL condition on `repos`, with `?1`).
fn by_repo(conn: &Connection, filter: &str, value: &str) -> Result<Option<Project>, DbError> {
    let id: Option<String> = conn
        .query_row(
            &format!("SELECT project_id FROM repos WHERE {filter}"),
            [value],
            |r| r.get(0),
        )
        .optional()?;
    match id {
        Some(id) => get(conn, &id),
        None => Ok(None),
    }
}

/// The project with a repo whose git common dir is `git_common_dir`.
pub(super) fn find(conn: &Connection, git_common_dir: &Path) -> Result<Option<Project>, DbError> {
    by_repo(
        conn,
        "git_common_dir = ?1",
        &git_common_dir.to_string_lossy(),
    )
}

/// The project with a repo at `path` that has no git (a folder project, ADR 0005).
pub(super) fn find_folder(conn: &Connection, path: &Path) -> Result<Option<Project>, DbError> {
    by_repo(
        conn,
        "git_common_dir IS NULL AND path = ?1",
        &path.to_string_lossy(),
    )
}

/// The project with this id.
pub(super) fn get(conn: &Connection, id: &str) -> Result<Option<Project>, DbError> {
    let project = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM projects WHERE id = ?1"),
            [id],
            row_to_project,
        )
        .optional()?;
    project.map(|p| with_repos(conn, p)).transpose()
}

/// The project with this id, or [`DbError::NotFound`].
fn require(conn: &Connection, id: &str) -> Result<Project, DbError> {
    get(conn, id)?.ok_or_else(|| DbError::NotFound(format!("no project {id}")))
}

pub(super) fn list(conn: &Connection) -> Result<Vec<Project>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM projects ORDER BY COALESCE(last_opened_ms, 0) DESC, id"
    ))?;
    let projects: Vec<Project> = stmt
        .query_map([], row_to_project)?
        .collect::<Result<_, _>>()?;
    projects.into_iter().map(|p| with_repos(conn, p)).collect()
}

/// `slug`, numbered (`slug-2`, ...) when another project has it. Tangents' id is reserved
/// (ADR 0017): a project of the user's never gets it, even before Tangents is created.
fn free_id(conn: &Connection, slug: &str) -> Result<String, DbError> {
    let mut id = slug.to_owned();
    let mut n = 2;
    while id == TANGENTS_PROJECT
        || conn
            .query_row("SELECT 1 FROM projects WHERE id = ?1", [&id], |_| Ok(()))
            .optional()?
            .is_some()
    {
        id = format!("{slug}-{n}");
        n += 1;
    }
    Ok(id)
}

fn insert_project(
    conn: &Connection,
    id: &str,
    name: &str,
    account: Option<&str>,
    now_ms: i64,
) -> Result<(), DbError> {
    conn.execute(
        &format!("INSERT INTO projects ({COLUMNS}) VALUES (?1, ?2, ?3, ?4)"),
        params![id, name, account, now_ms],
    )?;
    Ok(())
}

/// A repo to add: what [`add_repo`] stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRepo {
    /// Lower kebab case, unique in the project.
    pub name: String,
    /// The main checkout, or a folder without git.
    pub path: PathBuf,
    /// The repo's git common dir; `None` for a folder without git.
    pub git_common_dir: Option<PathBuf>,
}

/// Adds `repo` to project `id` inside `conn`'s open transaction. Refused when another project
/// (or this one) has the same repo, or this project a repo of the same name or path.
fn insert_repo(conn: &Connection, id: &str, repo: &NewRepo, now_ms: i64) -> Result<(), DbError> {
    let existing = match &repo.git_common_dir {
        Some(dir) => find(conn, dir)?,
        None => find_folder(conn, &repo.path)?,
    };
    if let Some(owner) = existing {
        return Err(DbError::Refused(format!(
            "{} is already a repo of project {}",
            repo.path.display(),
            owner.id
        )));
    }
    let taken = conn
        .query_row(
            "SELECT 1 FROM repos WHERE project_id = ?1 AND (name = ?2 OR path = ?3)",
            params![id, repo.name, repo.path.to_string_lossy()],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if taken {
        return Err(DbError::Refused(format!(
            "project {id} already has a repo named {} or at {}",
            repo.name,
            repo.path.display()
        )));
    }
    conn.execute(
        "INSERT INTO repos (project_id, name, path, git_common_dir, added_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id,
            repo.name,
            repo.path.to_string_lossy(),
            repo.git_common_dir
                .as_ref()
                .map(|d| d.to_string_lossy().into_owned()),
            now_ms
        ],
    )?;
    Ok(())
}

/// Registers a folder as a project with one repo (`:project add`, `project_add`), or returns the
/// project that already has it.
pub(super) fn register(
    conn: &mut Connection,
    slug: &str,
    name: &str,
    repo: &NewRepo,
    now_ms: i64,
) -> Result<Project, DbError> {
    let tx = conn.transaction()?;
    let existing = match &repo.git_common_dir {
        Some(dir) => find(&tx, dir)?,
        None => find_folder(&tx, &repo.path)?,
    };
    if let Some(existing) = existing {
        return Ok(existing);
    }
    let id = free_id(&tx, slug)?;
    insert_project(&tx, &id, name, None, now_ms)?;
    insert_repo(&tx, &id, repo, now_ms)?;
    let project = require(&tx, &id)?;
    tx.commit()?;
    Ok(project)
}

/// Creates a project from Home (ADR 0021): a name, an account, and maybe a first repo.
pub(super) fn create(
    conn: &mut Connection,
    slug: &str,
    name: &str,
    account: Option<&str>,
    repo: Option<&NewRepo>,
    now_ms: i64,
) -> Result<Project, DbError> {
    let tx = conn.transaction()?;
    let id = free_id(&tx, slug)?;
    insert_project(&tx, &id, name, account, now_ms)?;
    if let Some(repo) = repo {
        insert_repo(&tx, &id, repo, now_ms)?;
    }
    let project = require(&tx, &id)?;
    tx.commit()?;
    Ok(project)
}

/// Registers a research project: no repos. `None` when a project with no repos of this name
/// exists already.
pub(super) fn register_research(
    conn: &mut Connection,
    slug: &str,
    name: &str,
    now_ms: i64,
) -> Result<Option<Project>, DbError> {
    let tx = conn.transaction()?;
    let taken = tx
        .query_row(
            "SELECT 1 FROM projects p WHERE p.name = ?1 \
             AND NOT EXISTS (SELECT 1 FROM repos r WHERE r.project_id = p.id)",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if taken {
        return Ok(None);
    }
    let id = free_id(&tx, slug)?;
    insert_project(&tx, &id, name, None, now_ms)?;
    let project = require(&tx, &id)?;
    tx.commit()?;
    Ok(Some(project))
}

/// Tangents, the built-in research project (ADR 0017): registered under its reserved id when
/// missing, never touched when present. A project with repos under that id (registered before
/// the id was reserved) is refused, so tangents never write into a user's project.
pub(super) fn ensure_tangents(conn: &mut Connection, now_ms: i64) -> Result<Project, DbError> {
    let tx = conn.transaction()?;
    if get(&tx, TANGENTS_PROJECT)?.is_none() {
        insert_project(&tx, TANGENTS_PROJECT, TANGENTS_NAME, None, now_ms)?;
    }
    let project = require(&tx, TANGENTS_PROJECT)?;
    tx.commit()?;
    if let Some(repo) = project.repos.first() {
        return Err(DbError::Refused(format!(
            "the project `{TANGENTS_PROJECT}` is not Tangents (it is a code project in {}): \
             tangents are off until it is renamed",
            repo.path.display()
        )));
    }
    Ok(project)
}

/// Adds a repo to project `id`.
pub(super) fn add_repo(
    conn: &mut Connection,
    id: &str,
    repo: &NewRepo,
    now_ms: i64,
) -> Result<Project, DbError> {
    let tx = conn.transaction()?;
    require(&tx, id)?;
    insert_repo(&tx, id, repo, now_ms)?;
    let project = require(&tx, id)?;
    tx.commit()?;
    Ok(project)
}

/// Removes repo `repo` from project `id`. The caller has checked that no slice works in it.
pub(super) fn remove_repo(conn: &Connection, id: &str, repo: RepoId) -> Result<Project, DbError> {
    let removed = conn.execute(
        "DELETE FROM repos WHERE id = ?1 AND project_id = ?2",
        params![repo.0, id],
    )?;
    if removed == 0 {
        return Err(DbError::NotFound(format!(
            "project {id} has no repo {}",
            repo.0
        )));
    }
    require(conn, id)
}

/// Renames project `id`; its id stays.
pub(super) fn rename(conn: &Connection, id: &str, name: &str) -> Result<Project, DbError> {
    conn.execute(
        "UPDATE projects SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    require(conn, id)
}

/// Sets or clears project `id`'s account.
pub(super) fn set_account(
    conn: &Connection,
    id: &str,
    account: Option<&str>,
) -> Result<Project, DbError> {
    conn.execute(
        "UPDATE projects SET account = ?2 WHERE id = ?1",
        params![id, account],
    )?;
    require(conn, id)
}

/// Records that a repo without git (a folder project's) is now a git repo.
pub(super) fn set_git_dir(
    conn: &Connection,
    repo: RepoId,
    git_common_dir: &Path,
) -> Result<Project, DbError> {
    conn.execute(
        "UPDATE repos SET git_common_dir = ?2 WHERE id = ?1",
        params![repo.0, git_common_dir.to_string_lossy()],
    )?;
    let id: String = conn.query_row(
        "SELECT project_id FROM repos WHERE id = ?1",
        [repo.0],
        |r| r.get(0),
    )?;
    require(conn, &id)
}

pub(super) fn touch(conn: &Connection, id: &str, now_ms: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE projects SET last_opened_ms = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

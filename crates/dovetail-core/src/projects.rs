//! Project identity and the registry (plan B9, B25).
//!
//! A project is a git repo, identified by `git rev-parse --path-format=absolute --git-common-dir`
//! so that a worktree of a repo is the same project as its main checkout. Its id is the slug of
//! the repo folder, numbered (`name-2`) when another repo already took it.
//!
//! These functions block on the DB thread, so call them before the tokio runtime starts (binary
//! start-up, `dovetail add`), never from async code.

use std::path::{Path, PathBuf};
use std::process::Command;

use dovetail_api::{ProjectId, ProjectSummary};

use crate::db::{Db, DbError};

/// A registered project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Slug of the repo folder, numbered on collision.
    pub id: String,
    /// Display name: the repo folder's name.
    pub name: String,
    /// The main checkout.
    pub repo_path: PathBuf,
    /// The repo's git common dir, which identifies it.
    pub git_common_dir: PathBuf,
    /// When it was last opened, in Unix milliseconds.
    pub last_opened_ms: Option<i64>,
}

impl Project {
    /// The project as the UI sees it.
    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId(self.id.clone()),
            name: self.name.clone(),
            repo_path: self.repo_path.clone(),
        }
    }
}

/// Errors from project lookup and registration.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    /// The directory is not inside a git repo.
    #[error("{0} is not inside a git repository")]
    NotARepo(PathBuf),
    /// `git` could not be run.
    #[error("running git: {0}")]
    Git(std::io::Error),
    /// Not in a repo, and no project has been opened before.
    #[error("not in a git repository, and no project is registered yet")]
    NoProject,
    /// The registry failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// Where a directory's repo lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The repo's git common dir.
    pub git_common_dir: PathBuf,
    /// The main checkout (the common dir's parent; a bare repo is its own checkout).
    pub repo_path: PathBuf,
}

/// Lower-case, `-`-separated form of a folder name; `project` if nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "project".into()
    } else {
        trimmed.to_owned()
    }
}

/// Finds the repo that contains `dir`.
pub fn identify(dir: &Path) -> Result<Identity, ProjectError> {
    let output = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(dir)
        .output()
        .map_err(ProjectError::Git)?;
    if !output.status.success() {
        return Err(ProjectError::NotARepo(dir.to_owned()));
    }
    let common = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let repo_path = if common.file_name().is_some_and(|n| n == ".git") {
        common
            .parent()
            .map_or_else(|| common.clone(), Path::to_path_buf)
    } else {
        common.clone()
    };
    Ok(Identity {
        git_common_dir: common,
        repo_path,
    })
}

/// Registers the repo at `identity`, or returns the project already registered for it.
pub fn register(db: &Db, identity: &Identity) -> Result<Project, ProjectError> {
    let name = identity
        .repo_path
        .file_name()
        .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
    Ok(db
        .register_project(
            slug(&name),
            name,
            identity.repo_path.clone(),
            identity.git_common_dir.clone(),
        )
        .wait()?)
}

/// Every registered project, most recently opened first.
pub fn list(db: &Db) -> Result<Vec<Project>, ProjectError> {
    Ok(db.list_projects().wait()?)
}

/// The project for the directory the user is in (B25): its repo, registered on first sight
/// (`registered` is true then); outside any git repo, the most recently opened project.
/// Marks the result as opened now.
pub fn resolve(db: &Db, dir: &Path) -> Result<(Project, bool), ProjectError> {
    let (project, registered) = match identify(dir) {
        Ok(identity) => match db.find_project(identity.git_common_dir.clone()).wait()? {
            Some(project) => (project, false),
            None => (register(db, &identity)?, true),
        },
        Err(ProjectError::NotARepo(_)) => (
            list(db)?
                .into_iter()
                .next()
                .ok_or(ProjectError::NoProject)?,
            false,
        ),
        Err(other) => return Err(other),
    };
    db.touch_project(project.id.clone()).wait()?;
    Ok((project, registered))
}

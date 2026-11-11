//! Project identity and the registry (plan B9, ADR 0005).
//!
//! A project is registered only when the user asks (`:project add`, or the coordinator's
//! `project_add`). Which folder is the project ([`locate`]): inside a git repo, the repo,
//! identified by `git rev-parse --path-format=absolute --git-common-dir` so that a worktree is the
//! same project as its main checkout; otherwise the nearest registered ancestor folder; otherwise
//! the folder itself, as a [`Candidate`]. Its id is the slug of the folder name, numbered
//! (`name-2`) when another project already took it.
//!
//! These functions block on the DB thread or run `git`, so call them before the tokio runtime
//! starts or from a blocking task, never from async code directly.

use std::path::{Path, PathBuf};
use std::process::Command;

use dovetail_api::{ProjectId, ProjectKind, ProjectSummary};

use crate::db::{Db, DbError};
use crate::paths::Paths;

/// A registered project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Slug of the folder (or of a research project's name), numbered on collision.
    pub id: String,
    /// Display name: the folder's name, or the name a research project was given.
    pub name: String,
    /// Code or research. Code paths check this, never whether a folder exists.
    pub kind: ProjectKind,
    /// The main checkout, or the folder of a project without git; `None` for a research
    /// project, which has no repo.
    pub repo_path: Option<PathBuf>,
    /// The repo's git common dir, which identifies it; `None` for a folder without git.
    pub git_common_dir: Option<PathBuf>,
    /// When it was last opened, in Unix milliseconds.
    pub last_opened_ms: Option<i64>,
}

impl Project {
    /// A research project: no repo, no worktrees, no slices, no git.
    pub fn is_research(&self) -> bool {
        self.kind == ProjectKind::Research
    }

    /// The folder its sessions run in: the repo or folder, or for a research project its own
    /// state folder (`Paths::project_dir`).
    pub fn folder(&self, paths: &Paths) -> PathBuf {
        match &self.repo_path {
            Some(repo) if !self.is_research() => repo.clone(),
            _ => paths.project_dir(&self.id),
        }
    }

    /// The project as the UI sees it.
    pub fn summary(&self, paths: &Paths) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId(self.id.clone()),
            name: self.name.clone(),
            repo_path: self.folder(paths),
            kind: self.kind,
        }
    }
}

/// A folder that could become a project but is not registered yet (ADR 0005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Display name: the folder's name.
    pub name: String,
    /// The main checkout, or the folder itself.
    pub repo_path: PathBuf,
    /// The repo's git common dir, when the folder is in a git repo.
    pub git_common_dir: Option<PathBuf>,
}

impl Candidate {
    /// The id it would get if nothing else had taken it.
    pub fn slug(&self) -> String {
        slug(&self.name)
    }

    /// The candidate as the UI sees it, under its would-be id.
    pub fn summary(&self) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId(self.slug()),
            name: self.name.clone(),
            repo_path: self.repo_path.clone(),
            kind: ProjectKind::Code,
        }
    }
}

/// What [`locate`] found for a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The directory belongs to a registered project.
    Registered(Project),
    /// It does not; this is what `:project add` would register.
    Candidate(Candidate),
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
    /// The directory could not be read.
    #[error("{path}: {source}")]
    Io {
        /// The directory.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The directory is not part of a registered project (`dovetail add`).
    #[error(
        "{0} is not a Dovetail project yet. Open `dovetail` there and run `:project add`, \
         or ask the coordinator to add it"
    )]
    NotRegistered(PathBuf),
    /// No registered project has this id (`dovetail --project <id>`).
    #[error("there is no project `{0}`; open dovetail and pick one with <leader>pp")]
    UnknownId(String),
    /// A research project needs a name.
    #[error("a research project needs a name: :project new <name> --research")]
    NoName,
    /// A research project of this name exists already.
    #[error("there is already a research project named `{0}`")]
    NameTaken(String),
    /// A research project's state folder could not be made.
    #[error("creating {path}: {source}")]
    Folder {
        /// The folder.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
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

/// Finds the repo that contains `dir`. The one place that knows how to find a main checkout:
/// worktree code calls this too.
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

/// Whether `dir` is inside a git repo now. A folder project that gains a repo later gets
/// worktrees for its new slices (ADR 0005).
pub fn has_git(dir: &Path) -> bool {
    identify(dir).is_ok()
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned())
}

/// Finds the project `dir` belongs to, registering nothing (ADR 0005). A registered project is
/// marked as opened now.
pub fn locate(db: &Db, dir: &Path) -> Result<Resolution, ProjectError> {
    let dir = dir.canonicalize().map_err(|source| ProjectError::Io {
        path: dir.to_owned(),
        source,
    })?;
    let found = match identify(&dir) {
        Ok(identity) => match db.find_project(identity.git_common_dir.clone()).wait()? {
            Some(project) => Some(project),
            // A folder project that has since become a repo keeps its id.
            None => match db.find_folder(identity.repo_path.clone()).wait()? {
                Some(project) => Some(
                    db.set_git_dir(project.id.clone(), identity.git_common_dir.clone())
                        .wait()?,
                ),
                None => {
                    return Ok(Resolution::Candidate(Candidate {
                        name: folder_name(&identity.repo_path),
                        repo_path: identity.repo_path,
                        git_common_dir: Some(identity.git_common_dir),
                    }));
                }
            },
        },
        // Research projects have no folder of the user's, so a directory never belongs to one.
        Err(ProjectError::NotARepo(_)) => db
            .list_projects()
            .wait()?
            .into_iter()
            .filter_map(|p| {
                let depth = p.repo_path.as_ref().filter(|r| dir.starts_with(r))?;
                Some((depth.components().count(), p))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, p)| p),
        Err(other) => return Err(other),
    };
    match found {
        Some(project) => {
            db.touch_project(project.id.clone()).wait()?;
            Ok(Resolution::Registered(project))
        }
        None => Ok(Resolution::Candidate(Candidate {
            name: folder_name(&dir),
            repo_path: dir,
            git_common_dir: None,
        })),
    }
}

/// The registered project for `dir`, or [`ProjectError::NotRegistered`].
pub fn require(db: &Db, dir: &Path) -> Result<Project, ProjectError> {
    match locate(db, dir)? {
        Resolution::Registered(project) => Ok(project),
        Resolution::Candidate(candidate) => Err(ProjectError::NotRegistered(candidate.repo_path)),
    }
}

/// Registers a candidate, or returns the project already registered for its repo or folder.
pub fn register(db: &Db, candidate: &Candidate) -> Result<Project, ProjectError> {
    Ok(db
        .register_project(
            candidate.slug(),
            candidate.name.clone(),
            candidate.repo_path.clone(),
            candidate.git_common_dir.clone(),
        )
        .wait()?)
}

/// The registered project `id`, marked as opened now (`dovetail --project <id>`, and the
/// project switcher, which starts the app again on it).
pub fn open(db: &Db, id: &str) -> Result<Project, ProjectError> {
    let project = db
        .get_project(id.to_owned())
        .wait()?
        .ok_or_else(|| ProjectError::UnknownId(id.to_owned()))?;
    db.touch_project(project.id.clone()).wait()?;
    Ok(project)
}

/// Every registered project, most recently opened first.
pub fn list(db: &Db) -> Result<Vec<Project>, ProjectError> {
    Ok(db.list_projects().wait()?)
}

//! Project identity and the registry (plan B9, ADR 0005, ADR 0021).
//!
//! A project is a name with zero or more repos and one account (ADR 0021). The GUI creates
//! projects and adds repos from Home; the frozen TUI still registers the folder it runs in
//! (`:project add`, or the coordinator's `project_add`) as a project with one repo. Which project
//! a folder belongs to ([`locate`]): inside a git repo, the project with that repo, identified by
//! `git rev-parse --path-format=absolute --git-common-dir` so that a worktree is the same project
//! as its main checkout; otherwise the project with the nearest folder repo; otherwise the folder
//! itself, as a [`Candidate`]. An id is the slug of the name, numbered (`name-2`) when another
//! project already took it.
//!
//! These functions block on the DB thread or run `git`, so call them before the tokio runtime
//! starts or from a blocking task, never from async code directly.

use std::path::{Path, PathBuf};
use std::process::Command;

use dovetail_api::project::{ProjectId, ProjectKind, ProjectSummary, RepoId, RepoSummary};

use crate::db::{Db, DbError, NewRepo};
use crate::paths::Paths;

/// One repo of a project (ADR 0021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Registry id: what a slice stores.
    pub id: RepoId,
    /// Lower kebab case, unique in its project.
    pub name: String,
    /// The main checkout, or a folder without git (an old folder project, ADR 0005).
    pub path: PathBuf,
    /// The repo's git common dir, which identifies it; `None` for a folder without git.
    pub git_common_dir: Option<PathBuf>,
}

impl Repo {
    /// The repo as the UI sees it.
    pub fn summary(&self) -> RepoSummary {
        RepoSummary {
            id: self.id,
            name: self.name.clone(),
            path: self.path.clone(),
        }
    }
}

/// A registered project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Slug of the name, numbered on collision. Names its state folder, socket and recipe.
    pub id: String,
    /// Display name.
    pub name: String,
    /// The Claude profile its sessions run under; `None`: the recipe's `profile`, else the
    /// default Claude home.
    pub account: Option<String>,
    /// Its repos, in the order they were added. None: a research-only project.
    pub repos: Vec<Repo>,
    /// When it was last opened, in Unix milliseconds.
    pub last_opened_ms: Option<i64>,
}

impl Project {
    /// A project with one repo (id 1), as `:project add` registers a folder; for callers that
    /// build one without the registry, such as tests.
    pub fn with_repo(
        id: impl Into<String>,
        name: impl Into<String>,
        path: PathBuf,
        git_common_dir: Option<PathBuf>,
    ) -> Project {
        let id = id.into();
        Project {
            repos: vec![Repo {
                id: RepoId(1),
                name: slug(&id),
                path,
                git_common_dir,
            }],
            id,
            name: name.into(),
            account: None,
            last_opened_ms: None,
        }
    }

    /// A research-only project: no repos, so no worktrees, no slices, no git (ADR 0021 point 2).
    pub fn is_research(&self) -> bool {
        self.repos.is_empty()
    }

    /// The kind the frozen TUI shows, computed from the repos.
    pub fn kind(&self) -> ProjectKind {
        if self.is_research() {
            ProjectKind::Research
        } else {
            ProjectKind::Code
        }
    }

    /// The first repo added: what single-repo paths (`:docs`, `:upgrade`, the research folder)
    /// use until a repo can be named there.
    pub fn first_repo(&self) -> Option<&Repo> {
        self.repos.first()
    }

    /// The repo with this id.
    pub fn repo(&self, id: RepoId) -> Option<&Repo> {
        self.repos.iter().find(|r| r.id == id)
    }

    /// The repo called `name`.
    pub fn repo_named(&self, name: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// The coordinator's working folder (ADR 0021 point 3).
    pub fn home(&self, paths: &Paths) -> PathBuf {
        paths.project_home(&self.id)
    }

    /// The project as the UI sees it, with zero counts ([`Db::project_counts`] fills them).
    pub fn summary(&self, paths: &Paths) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId(self.id.clone()),
            name: self.name.clone(),
            repo_path: self
                .first_repo()
                .map_or_else(|| paths.project_dir(&self.id), |r| r.path.clone()),
            kind: self.kind(),
            repos: self.repos.iter().map(Repo::summary).collect(),
            account: self.account.clone(),
            last_opened_ms: self.last_opened_ms,
            counts: Default::default(),
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
            ..ProjectSummary::default()
        }
    }

    /// The one repo it would be registered with, named like the project.
    pub fn new_repo(&self) -> NewRepo {
        NewRepo {
            name: self.slug(),
            path: self.repo_path.clone(),
            git_common_dir: self.git_common_dir.clone(),
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
    /// The account is no profile in `config.toml` (ADR 0009).
    #[error("config.toml lists no profile `{0}`: add it with `dovetail login {0}`")]
    UnknownAccount(String),
    /// A repo still has open slices (ADR 0021 point 4).
    #[error("the repo has open slices ({0}): archive them first")]
    RepoInUse(String),
    /// A project needs a name (ADR 0021: Home's new-project form).
    #[error("a project needs a name")]
    EmptyName,
    /// A repo name has nothing left once made kebab case.
    #[error("`{0}` is not a usable repo name: use letters, digits and dashes")]
    BadRepoName(String),
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

/// Finds the project `dir` belongs to, registering nothing (ADR 0005), through the `repos`
/// table (ADR 0021 point 7). A registered project is marked as opened now.
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
                Some(project) => {
                    let repo = project
                        .repos
                        .iter()
                        .find(|r| r.path == identity.repo_path)
                        .map(|r| r.id);
                    match repo {
                        Some(repo) => Some(
                            db.set_git_dir(repo, identity.git_common_dir.clone())
                                .wait()?,
                        ),
                        None => Some(project),
                    }
                }
                None => {
                    return Ok(Resolution::Candidate(Candidate {
                        name: folder_name(&identity.repo_path),
                        repo_path: identity.repo_path,
                        git_common_dir: Some(identity.git_common_dir),
                    }));
                }
            },
        },
        // A research project has no folder of the user's, so a directory never belongs to one.
        Err(ProjectError::NotARepo(_)) => db
            .list_projects()
            .wait()?
            .into_iter()
            .filter_map(|p| {
                let depth = p
                    .repos
                    .iter()
                    .filter(|r| r.git_common_dir.is_none() && dir.starts_with(&r.path))
                    .map(|r| r.path.components().count())
                    .max()?;
                Some((depth, p))
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

/// The repo at `path` (a folder inside a git repo) as it would be added, called `name` (made
/// kebab case) or after its folder. Blocks on `git`.
pub fn repo_at(path: &Path, name: Option<&str>) -> Result<NewRepo, ProjectError> {
    let dir = path.canonicalize().map_err(|source| ProjectError::Io {
        path: path.to_owned(),
        source,
    })?;
    let identity = identify(&dir)?;
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(given) if given.chars().any(|c| c.is_ascii_alphanumeric()) => slug(given),
        Some(given) => return Err(ProjectError::BadRepoName(given.to_owned())),
        None => slug(&folder_name(&identity.repo_path)),
    };
    Ok(NewRepo {
        name,
        path: identity.repo_path,
        git_common_dir: Some(identity.git_common_dir),
    })
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
            candidate.new_repo(),
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

/// The registered project `id`, without marking it opened (`dovetail add --project <id>`).
pub fn find(db: &Db, id: &str) -> Result<Project, ProjectError> {
    db.get_project(id.to_owned())
        .wait()?
        .ok_or_else(|| ProjectError::UnknownId(id.to_owned()))
}

/// Every registered project, most recently opened first.
pub fn list(db: &Db) -> Result<Vec<Project>, ProjectError> {
    Ok(db.list_projects().wait()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_repo(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git init");
        dir.canonicalize().unwrap()
    }

    /// ADR 0021 point 7: `dovetail` run in a folder of any of a project's repos opens that
    /// project; a repo of no project is a candidate.
    #[test]
    fn a_folder_opens_the_project_of_its_repo() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            data: dir.path().join("data"),
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            runtime: dir.path().join("run"),
        };
        let api = git_repo(&dir.path().join("shop-api"));
        let web = git_repo(&dir.path().join("shop-web"));
        let other = git_repo(&dir.path().join("other"));
        let db = Db::open(&paths, None).unwrap();
        let Resolution::Candidate(candidate) = locate(&db, &api).unwrap() else {
            panic!("not registered yet");
        };
        let shop = register(&db, &candidate).unwrap();
        assert_eq!(shop.repos.len(), 1);
        db.add_repo(shop.id.clone(), repo_at(&web, None).unwrap())
            .wait()
            .unwrap();

        for folder in [api.join("src"), web.join("src")] {
            match locate(&db, &folder).unwrap() {
                Resolution::Registered(p) => assert_eq!(p.id, shop.id),
                Resolution::Candidate(c) => panic!("{} not found: {c:?}", folder.display()),
            }
        }
        assert!(matches!(
            locate(&db, &other).unwrap(),
            Resolution::Candidate(_)
        ));
        assert!(matches!(
            repo_at(&dir.path().join("data"), None),
            Err(ProjectError::NotARepo(_)) | Err(ProjectError::Io { .. })
        ));
    }
}

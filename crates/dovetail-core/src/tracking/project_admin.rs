//! Creating and changing projects (ADR 0021): the five project commands, shared by Home and
//! the core. Each checks its input, then makes one registry request; the caller waits for it
//! (Home blocks, the core awaits) and sends [`dovetail_api::tracking::project::ProjectEvent::Upserted`].
//!
//! Only the core knows what runs, so the checks that need it (a repo with a live slice cannot
//! be removed) and the reactions (the coordinator restarted with its new repos) live in the
//! core, in `service.rs`.

use std::path::{Path, PathBuf};

use dovetail_api::Command;
use dovetail_api::tracking::project::{ProjectCommand, ProjectId, RepoId};

use crate::config::Config;
use crate::paths::Paths;
use crate::projects::{self, Project, ProjectError};
use crate::recipe;
use crate::storage::{Db, Pending};

/// One of the five project commands, parsed out of a [`Command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectOp {
    Create {
        name: String,
        account: Option<String>,
        repo: Option<PathBuf>,
    },
    Rename {
        id: ProjectId,
        name: String,
    },
    SetAccount {
        id: ProjectId,
        account: Option<String>,
    },
    AddRepo {
        id: ProjectId,
        path: PathBuf,
        name: Option<String>,
    },
    RemoveRepo {
        id: ProjectId,
        repo: RepoId,
    },
}

impl ProjectOp {
    /// Whether `command` is one of the five project commands.
    pub(crate) fn matches(command: &Command) -> bool {
        matches!(
            command,
            Command::Project(ProjectCommand::Create { .. })
                | Command::Project(ProjectCommand::Rename { .. })
                | Command::Project(ProjectCommand::SetAccount { .. })
                | Command::Project(ProjectCommand::AddRepo { .. })
                | Command::Project(ProjectCommand::RemoveRepo { .. })
        )
    }

    /// The project command in `command`; `None` for any other command.
    pub(crate) fn parse(command: Command) -> Option<ProjectOp> {
        Some(match command {
            Command::Project(ProjectCommand::Create {
                name,
                account,
                repo,
            }) => ProjectOp::Create {
                name,
                account,
                repo,
            },
            Command::Project(ProjectCommand::Rename { id, name }) => ProjectOp::Rename { id, name },
            Command::Project(ProjectCommand::SetAccount { id, account }) => {
                ProjectOp::SetAccount { id, account }
            }
            Command::Project(ProjectCommand::AddRepo {
                project,
                path,
                name,
            }) => ProjectOp::AddRepo {
                id: project,
                path,
                name,
            },
            Command::Project(ProjectCommand::RemoveRepo { project, repo }) => {
                ProjectOp::RemoveRepo { id: project, repo }
            }
            _ => return None,
        })
    }

    /// The project it changes; `None` for a new one.
    pub(crate) fn project(&self) -> Option<&ProjectId> {
        match self {
            ProjectOp::Create { .. } => None,
            ProjectOp::Rename { id, .. }
            | ProjectOp::SetAccount { id, .. }
            | ProjectOp::AddRepo { id, .. }
            | ProjectOp::RemoveRepo { id, .. } => Some(id),
        }
    }

    /// Checks the input and starts the registry request. Blocks: it runs `git` for a repo path
    /// and reads the project's `state.db` for slices in a repo to remove, so the core calls it
    /// on a blocking thread. `config` lists the accounts there are.
    pub(crate) fn start(
        self,
        db: &Db,
        paths: &Paths,
        config: &Config,
    ) -> Result<Pending<Project>, ProjectError> {
        let account = |account: Option<String>| -> Result<Option<String>, ProjectError> {
            match account
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty())
            {
                Some(a) if !config.profiles.contains_key(&a) => {
                    Err(ProjectError::UnknownAccount(a))
                }
                other => Ok(other),
            }
        };
        Ok(match self {
            ProjectOp::Create {
                name,
                account: wanted,
                repo,
            } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err(ProjectError::EmptyName);
                }
                let account = account(wanted)?;
                let repo = repo
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| projects::repo_at(&expand_home(&p), None))
                    .transpose()?;
                db.create_project(projects::slug(&name), name, account, repo)
            }
            ProjectOp::Rename { id, name } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err(ProjectError::EmptyName);
                }
                db.rename_project(id.0, name)
            }
            ProjectOp::SetAccount {
                id,
                account: wanted,
            } => db.set_project_account(id.0, account(wanted)?),
            ProjectOp::AddRepo { id, path, name } => {
                let repo = projects::repo_at(&expand_home(&path), name.as_deref())?;
                db.add_repo(id.0, repo)
            }
            ProjectOp::RemoveRepo { id, repo } => {
                // A repo a slice still works in stays (ADR 0021 point 4), whether or not the
                // project is open.
                let first = db
                    .get_project(id.0.clone())
                    .wait()?
                    .and_then(|p| p.first_repo().map(|r| r.id))
                    == Some(repo);
                let busy = db
                    .slices_in_repo(paths.state_db(&id.0), repo.0, first)
                    .wait()?;
                if !busy.is_empty() {
                    return Err(ProjectError::RepoInUse(busy.join(", ")));
                }
                db.remove_repo(id.0, repo)
            }
        })
    }
}

/// `~/x` as the user typed it into a form: the home folder joined with `x`.
fn expand_home(path: &Path) -> PathBuf {
    match (path.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => path.to_owned(),
    }
}

/// Writes a new project's starter recipe (`~/.config/dovetail/projects/<id>.toml`) unless one
/// is there: a recipe is never overwritten. The path when it was written now.
pub(crate) fn write_starter_recipe(
    paths: &Paths,
    config: &Config,
    project: &Project,
) -> std::io::Result<Option<PathBuf>> {
    let path = paths.recipe(&project.id);
    let profiles: Vec<&str> = config.profiles.keys().map(String::as_str).collect();
    let text = recipe::starter(&project.name, &profiles);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    match file {
        Ok(mut file) => {
            std::io::Write::write_all(&mut file, text.as_bytes())?;
            Ok(Some(path))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
    }
}

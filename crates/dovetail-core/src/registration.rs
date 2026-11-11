//! Registering the open folder as a project (ADR 0005), and the project state the core runs
//! on before and after.

use std::path::PathBuf;
use std::sync::Arc;

use dovetail_api::{ActivityKind, AgentId, ProjectSummary};

use crate::error::RegisterError;
use crate::paths::Paths;
use crate::projects::{Candidate, Project};
use crate::recipe;
use crate::service::{Core, CoreHandler, serve};
use crate::socket::{self, SocketHandler};

/// The warning shown until the folder is a project.
pub(crate) use dovetail_api::UNREGISTERED_WARNING;

/// The open folder: a registered project, or one that is not a project yet (ADR 0005). It
/// carries its key, so registering switches everything named after it at once.
#[derive(Debug, Clone)]
pub(crate) enum ProjectState {
    Registered(Project),
    Unregistered {
        candidate: Candidate,
        /// `new-<pid>` until registration.
        key: String,
    },
}

impl ProjectState {
    /// The project id, or `new-<pid>` before registration. Names run directories, worktree
    /// directories, hook outcomes, to-do list ids and the worktree hook's `DOVETAIL_PROJECT`.
    pub(crate) fn key(&self) -> &str {
        match self {
            ProjectState::Registered(p) => &p.id,
            ProjectState::Unregistered { key, .. } => key,
        }
    }

    pub(crate) fn summary(&self, paths: &Paths) -> ProjectSummary {
        match self {
            ProjectState::Registered(p) => p.summary(paths),
            ProjectState::Unregistered { candidate, .. } => candidate.summary(),
        }
    }

    /// The folder sessions run in: the repo or folder, or a research project's state folder.
    pub(crate) fn folder(&self, paths: &Paths) -> PathBuf {
        match self {
            ProjectState::Registered(p) => p.folder(paths),
            ProjectState::Unregistered { candidate, .. } => candidate.repo_path.clone(),
        }
    }

    /// A research project: code paths that need a repo (slices, worktrees, git) check this.
    pub(crate) fn is_research(&self) -> bool {
        self.registered().is_some_and(Project::is_research)
    }

    pub(crate) fn registered(&self) -> Option<&Project> {
        match self {
            ProjectState::Registered(p) => Some(p),
            ProjectState::Unregistered { .. } => None,
        }
    }
}

impl Core {
    /// `:project new <name> --research` (decided 2026-11-11): registers a research project and
    /// makes its state folder, where its sessions will run. The open project stays open; the
    /// fresh snapshot lists the new one in the switcher.
    pub(crate) async fn new_research_project(&mut self, name: String) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return self.fail(crate::projects::ProjectError::NoName.to_string());
        }
        let project = match self
            .db
            .register_research(crate::projects::slug(&name), name.clone())
            .await
        {
            Ok(Some(project)) => project,
            Ok(None) => {
                return self.fail(crate::projects::ProjectError::NameTaken(name).to_string());
            }
            Err(e) => return self.fail(e.to_string()),
        };
        let path = self.paths.project_dir(&project.id);
        if let Err(source) = std::fs::create_dir_all(&path) {
            let error = crate::projects::ProjectError::Folder { path, source };
            return self.fail(error.to_string());
        }
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!(
                "research project {} created as {}: <leader>pp opens it",
                project.name, project.id
            ),
        );
        // The project list changed: there is no event for that, so resend the state.
        self.send_snapshot(false).await;
    }

    /// The user picked another project in the switcher (B24): remember it for the binary, tell
    /// the UI, and quit the way `:q` does (sessions end and resume when the project is opened
    /// again).
    pub(crate) async fn open_other_project(&mut self, id: dovetail_api::ProjectId) {
        if self.quitting.is_some() {
            return;
        }
        if id.0 == self.project.key() {
            return self.fail(format!("{} is already open", id.0));
        }
        match self.db.get_project(id.0.clone()).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return self.fail(crate::projects::ProjectError::UnknownId(id.0).to_string());
            }
            Err(e) => return self.fail(e.to_string()),
        }
        let recorded = match self.switch_to.lock() {
            Ok(mut slot) => {
                *slot = Some(id.0.clone());
                true
            }
            Err(_) => false,
        };
        if !recorded {
            return self.fail("could not record the project to open");
        }
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!("opening project {}", id.0),
        );
        self.emit(dovetail_api::Event::OpeningProject { id });
        self.begin_quit();
    }

    /// Registers the open folder (ADR 0005): writes the in-memory state to the new project's
    /// `state.db`, writes a starter recipe when the project has none, listens on its socket, and
    /// sends a fresh snapshot. Returns the project and the starter recipe's path when one was
    /// written now (`None` when it was registered already or a recipe existed).
    pub(crate) async fn register_project(
        &mut self,
    ) -> Result<(Project, Option<PathBuf>), RegisterError> {
        let candidate = match &self.project {
            ProjectState::Registered(p) => return Ok((p.clone(), None)),
            ProjectState::Unregistered { candidate, .. } => candidate.clone(),
        };
        let project = self
            .db
            .register_project(
                candidate.slug(),
                candidate.name.clone(),
                candidate.repo_path.clone(),
                candidate.git_common_dir.clone(),
            )
            .await?;
        self.db
            .persist_state(self.paths.state_db(&project.id))
            .await?;
        let path = self.paths.socket(&project.id);
        match socket::bind(&path) {
            Ok(listener) => {
                let handler: Arc<dyn SocketHandler> = Arc::new(CoreHandler {
                    inputs: self.inputs.clone(),
                });
                tokio::spawn(serve(listener, handler));
                self.extra_sockets.push(path);
            }
            Err(e) => tracing::warn!("project socket: {e}"),
        }
        let old_lists = format!("dovetail-{}-", self.project.key());
        self.project = ProjectState::Registered(project.clone());
        let starter = self.write_starter_recipe(&project);
        // A recipe may already pick a profile; sessions that ran before keep theirs until they
        // next start (ADR 0009).
        self.claude = crate::service::project_claude_home(&self.paths, &self.config, &self.project);
        self.profile_error_notice();
        // An agent that never ran takes the project id in its to-do list id. One that has run
        // keeps the id its process uses (its lists live there), across every later resume.
        let new_lists = format!("dovetail-{}-", project.id);
        let ids: Vec<AgentId> = self
            .agents
            .values()
            .filter(|r| r.transcript_path.is_none() && !self.live.contains_key(&r.agent.id))
            .map(|r| r.agent.id.clone())
            .collect();
        for id in ids {
            self.update_agent(&id, |row| {
                if let Some(list) = &mut row.task_list_id
                    && let Some(rest) = list.strip_prefix(&old_lists)
                {
                    *list = format!("{new_lists}{rest}");
                }
            });
        }
        self.warnings.retain(|w| w.id.0 != UNREGISTERED_WARNING);
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!(
                "{} added as project {}",
                project.folder(&self.paths).display(),
                project.id
            ),
        );
        // The project itself changed: there is no event for that, so resend the state.
        self.send_snapshot(false).await;
        Ok((project, starter))
    }

    /// Writes `~/.config/dovetail/projects/<id>.toml` from [`recipe::starter`] unless a file is
    /// there already (task #31): it is never overwritten. The path when written; a failure is
    /// noted in the activity log and registration goes on without it.
    fn write_starter_recipe(&mut self, project: &Project) -> Option<PathBuf> {
        let path = self.paths.recipe(&project.id);
        let profiles: Vec<&str> = self.config.profiles.keys().map(String::as_str).collect();
        let text = recipe::starter(&project.name, &profiles);
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
            })
            .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
        match written {
            Ok(()) => Some(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(e) => {
                self.activity(
                    ActivityKind::Lifecycle,
                    None,
                    format!("could not write the starter recipe {}: {e}", path.display()),
                );
                None
            }
        }
    }

    /// The profile picker after `:project add` chose `profile` (task #31): uncomments it in the
    /// starter recipe just written. Sessions that already run keep their account until they next
    /// start (ADR 0009).
    pub(crate) fn set_recipe_profile(&mut self, id: dovetail_api::ProjectId, profile: String) {
        let Some(project) = self.project.registered().filter(|p| p.id == id.0).cloned() else {
            return self.fail(format!("{} is not the open project", id.0));
        };
        if !self.config.profiles.contains_key(&profile) {
            return self.fail(format!("config.toml lists no profile {profile}"));
        }
        let path = self.paths.recipe(&project.id);
        let updated = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| recipe::set_profile_text(&text, &profile));
        let Some(text) = updated else {
            return self.fail(format!(
                "could not set the profile: add `profile = \"{profile}\"` to {} by hand",
                path.display()
            ));
        };
        if let Err(e) = std::fs::write(&path, text) {
            return self.fail(format!("could not write {}: {e}", path.display()));
        }
        self.claude = crate::service::project_claude_home(&self.paths, &self.config, &self.project);
        self.profile_error_notice();
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!("{} now runs under profile {profile}", project.id),
        );
    }
}

//! Registering the open folder as a project (ADR 0005), and the project state the core runs
//! on before and after.

use std::path::PathBuf;
use std::sync::Arc;

use dovetail_api::activity::ActivityKind;
use dovetail_api::agents::agent::{AgentId, AgentKind};
use dovetail_api::tracking::project::{ProjectEvent, ProjectSummary, RepoId};

use crate::db::DbError;
use crate::paths::Paths;
use crate::project_admin::{self, ProjectOp};
use crate::projects::{Candidate, Project, ProjectError, Repo};
use crate::recipe;
use crate::service::{Core, CoreHandler, serve};
use crate::socket::{self, SocketHandler};

/// Registering the open folder as a project (ADR 0005).
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// Storage failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// The warning shown until the folder is a project.
pub(crate) use dovetail_api::activity::status::UNREGISTERED_WARNING;

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

    /// Where the coordinator, side chats and Ask run (ADR 0021 point 3): a registered project's
    /// `home/` folder, every repo attached with `--add-dir`; for a folder not registered yet
    /// (TUI only), the folder itself.
    pub(crate) fn coordinator_folder(&self, paths: &Paths) -> PathBuf {
        match self {
            ProjectState::Registered(p) => p.home(paths),
            ProjectState::Unregistered { candidate, .. } => candidate.repo_path.clone(),
        }
    }

    /// The folders attached to the coordinator with `--add-dir`: every repo of a registered
    /// project, none for an unregistered folder (it runs in it).
    pub(crate) fn attached(&self) -> Vec<PathBuf> {
        self.registered()
            .map(|p| p.repos.iter().map(|r| r.path.clone()).collect())
            .unwrap_or_default()
    }

    /// The repo `id`, or with `None` the first one. An unregistered folder is its own one repo
    /// (id 0, never stored).
    pub(crate) fn repo(&self, id: Option<RepoId>) -> Option<Repo> {
        match self {
            ProjectState::Registered(p) => match id {
                Some(id) => p.repo(id),
                None => p.first_repo(),
            }
            .cloned(),
            ProjectState::Unregistered { candidate, .. } => Some(Repo {
                id: RepoId(0),
                name: candidate.slug(),
                path: candidate.repo_path.clone(),
                git_common_dir: candidate.git_common_dir.clone(),
            }),
        }
    }

    /// Every repo: a registered project's, or the unregistered folder as its one repo.
    pub(crate) fn repos(&self) -> Vec<Repo> {
        match self {
            ProjectState::Registered(p) => p.repos.clone(),
            ProjectState::Unregistered { .. } => self.repo(None).into_iter().collect(),
        }
    }

    /// The first repo's folder: `:docs`, `:upgrade` and the research folder use it until a repo
    /// can be named there. `None` for a research project.
    pub(crate) fn first_repo_path(&self) -> Option<PathBuf> {
        self.repo(None).map(|r| r.path)
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
    /// Every registered project as the UI lists it, with Home's counts (ADR 0021).
    pub(crate) async fn project_summaries(&self) -> Result<Vec<ProjectSummary>, DbError> {
        let projects = self.db.list_projects().await?;
        let mut out = Vec::with_capacity(projects.len());
        for project in &projects {
            out.push(self.project_summary(project).await);
        }
        Ok(out)
    }

    /// One project as the UI lists it, with its counts (zero, logged, when they cannot be read).
    pub(crate) async fn project_summary(&self, project: &Project) -> ProjectSummary {
        let mut summary = project.summary(&self.paths);
        match self
            .db
            .project_counts(self.paths.state_db(&project.id))
            .await
        {
            Ok(counts) => summary.counts = counts,
            Err(e) => tracing::warn!(project = %project.id, "reading its counts: {e}"),
        }
        summary
    }

    /// One of the five project commands (ADR 0021). Any project may be changed; when it is the
    /// open one, the core takes the change in at once ([`Core::open_project_changed`]).
    pub(crate) async fn project_op(&mut self, op: ProjectOp) {
        let open = self.project.registered().map(|p| p.id.clone());
        let is_open = op
            .project()
            .is_some_and(|id| open.as_deref() == Some(id.0.as_str()));
        if is_open && let ProjectOp::RemoveRepo { repo, .. } = &op {
            let busy: Vec<String> = self
                .agents
                .values()
                .filter(|r| r.agent.kind == AgentKind::Slice)
                .filter(|r| self.repo_of(&r.agent.id).is_some_and(|x| x.id == *repo))
                .map(|r| r.agent.name.clone())
                .collect();
            if !busy.is_empty() {
                return self.fail(ProjectError::RepoInUse(busy.join(", ")).to_string());
            }
        }
        let creating = matches!(op, ProjectOp::Create { .. });
        // `start` runs git and reads files: off the loop's thread.
        let (db, paths, config) = (self.db.clone(), self.paths.clone(), self.config.clone());
        let started = tokio::task::spawn_blocking(move || op.start(&db, &paths, &config)).await;
        let pending = match started {
            Ok(Ok(pending)) => pending,
            Ok(Err(e)) => return self.fail(e.to_string()),
            Err(e) => return self.fail(format!("the project change did not run: {e}")),
        };
        let project = match pending.await {
            Ok(project) => project,
            Err(e) => return self.fail(e.to_string()),
        };
        if creating {
            match project_admin::write_starter_recipe(&self.paths, &self.config, &project) {
                Ok(_) => {}
                Err(e) => tracing::warn!(project = %project.id, "writing its starter recipe: {e}"),
            }
            self.activity(
                ActivityKind::Lifecycle,
                None,
                format!("project {} created", project.name),
            );
        }
        let summary = self.project_summary(&project).await;
        self.emit(dovetail_api::Event::Project(ProjectEvent::Upserted(
            summary,
        )));
        if is_open {
            self.open_project_changed(project);
        }
    }

    /// The open project changed in the registry: its account takes effect at each session's
    /// next start, and its repos at the coordinator's (ADR 0021 point 4).
    fn open_project_changed(&mut self, new: Project) {
        let Some(old) = self.project.registered().cloned() else {
            return;
        };
        self.project = ProjectState::Registered(new.clone());
        if old.account != new.account {
            self.claude =
                crate::service::project_claude_home(&self.paths, &self.config, &self.project);
            self.profile_error_notice();
            self.activity(
                ActivityKind::Lifecycle,
                None,
                format!(
                    "{} now runs under {}; running sessions keep theirs until they next start",
                    new.name,
                    new.account.as_deref().unwrap_or("the default account")
                ),
            );
        }
        let mut lines = Vec::new();
        for repo in new.repos.iter().filter(|r| old.repo(r.id).is_none()) {
            lines.push(format!(
                "repo `{}` attached at {}",
                repo.name,
                repo.path.display()
            ));
        }
        for repo in old.repos.iter().filter(|r| new.repo(r.id).is_none()) {
            lines.push(format!(
                "repo `{}` detached ({})",
                repo.name,
                repo.path.display()
            ));
        }
        if !lines.is_empty() {
            let text = format!(
                "Dovetail: {}. Start slices with `project_start_slice` and name the repo with \
                 `repo` when there are several.",
                lines.join("; ")
            );
            for line in &lines {
                self.activity(ActivityKind::Lifecycle, None, line.clone());
            }
            self.reattach_coordinator(text);
        }
    }

    /// Gets the coordinator onto the project's current repos: an idle one is restarted at once
    /// (a resume, with the new `--add-dir` flags) and told `note`; a working one when its turn
    /// ends; one not running hears it at its next start, which has the new flags anyway.
    pub(crate) fn reattach_coordinator(&mut self, note: String) {
        let id = AgentId(crate::service::COORDINATOR.into());
        let Some(live) = self.live.get_mut(&id) else {
            self.hold_for_coordinator(note);
            return;
        };
        // A second change before the restart adds to the note rather than replace it.
        self.reattach = Some(match self.reattach.take() {
            Some(earlier) => format!("{earlier}\n{note}"),
            None => note,
        });
        if !live.turn.busy() && self.quitting.is_none() {
            live.stopping = true;
            live.session.shutdown(crate::shutdown::QUIT_GRACE);
        }
    }

    /// The coordinator's turn ended: a pending reattach stops it now.
    pub(crate) fn reattach_after_turn(&mut self) {
        if self.reattach.is_none() || self.quitting.is_some() {
            return;
        }
        let id = AgentId(crate::service::COORDINATOR.into());
        if let Some(live) = self.live.get_mut(&id)
            && !live.turn.busy()
            && !live.stopping
        {
            live.stopping = true;
            live.session.shutdown(crate::shutdown::QUIT_GRACE);
        }
    }

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
    pub(crate) async fn open_other_project(
        &mut self,
        id: dovetail_api::tracking::project::ProjectId,
    ) {
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
        self.emit(dovetail_api::Event::Project(ProjectEvent::Opening { id }));
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
                candidate.new_repo(),
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
                candidate.repo_path.display(),
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
        match project_admin::write_starter_recipe(&self.paths, &self.config, project) {
            Ok(written) => written,
            Err(e) => {
                let path = self.paths.recipe(&project.id);
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
    pub(crate) fn set_recipe_profile(
        &mut self,
        id: dovetail_api::tracking::project::ProjectId,
        profile: String,
    ) {
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

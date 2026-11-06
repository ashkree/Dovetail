//! Registering the open folder as a project (ADR 0005), and the project state the core runs
//! on before and after.

use std::sync::Arc;

use dovetail_api::{ActivityKind, AgentId, ProjectSummary};

use crate::error::RegisterError;
use crate::projects::{Candidate, Project};
use crate::service::{Core, CoreHandler, serve};
use crate::socket::{self, SocketHandler};

/// The status-bar warning shown until the folder is a project.
pub(crate) const UNREGISTERED_WARNING: &str = "unregistered";

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

    pub(crate) fn summary(&self) -> ProjectSummary {
        match self {
            ProjectState::Registered(p) => p.summary(),
            ProjectState::Unregistered { candidate, .. } => candidate.summary(),
        }
    }

    pub(crate) fn repo_path(&self) -> &std::path::Path {
        match self {
            ProjectState::Registered(p) => &p.repo_path,
            ProjectState::Unregistered { candidate, .. } => &candidate.repo_path,
        }
    }

    pub(crate) fn registered(&self) -> Option<&Project> {
        match self {
            ProjectState::Registered(p) => Some(p),
            ProjectState::Unregistered { .. } => None,
        }
    }
}

impl Core {
    /// Registers the open folder (ADR 0005): writes the in-memory state to the new project's
    /// `state.db`, listens on its socket, and sends a fresh snapshot.
    pub(crate) async fn register_project(&mut self) -> Result<Project, RegisterError> {
        let candidate = match &self.project {
            ProjectState::Registered(p) => return Ok(p.clone()),
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
        // To-do lists named after the old key move to the project id for later launches.
        let new_lists = format!("dovetail-{}-", project.id);
        let ids: Vec<AgentId> = self.agents.keys().cloned().collect();
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
                project.repo_path.display(),
                project.id
            ),
        );
        // The project itself changed: there is no event for that, so resend the state.
        self.send_snapshot(false).await;
        Ok(project)
    }
}

//! Reading mode's `<leader>fr` (ADR 0018, decided 2026-11-11 later): another registered
//! project's library, read-only. Its `state.db` is opened read-only on the DB thread for each
//! request (WAL makes that safe while that project's own Dovetail runs), and its research files
//! and repo docs are only read. Nothing of that project is written, and the open project does
//! not change. Loaded when asked; no live refresh.

use std::path::PathBuf;

use dovetail_api::Event;
use dovetail_api::research::{
    ResearchContent, ResearchWhat,
    library::{BrowseWhat, BrowsedDoc, BrowsedListing, BrowsedProject, LibraryEvent},
};
use dovetail_api::tracking::project::ProjectId;

use crate::projects::Project;
use crate::research::{LoadError, content_of, files, listing, root_of, summary};
use crate::service::Core;

impl Core {
    /// The registered project `id`, or a failure for the UI.
    async fn browsed(&mut self, id: &ProjectId) -> Option<Project> {
        match self.db.get_project(id.0.clone()).await {
            Ok(Some(project)) => Some(project),
            Ok(None) => {
                self.fail(format!("there is no project {}", id.0));
                None
            }
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }

    /// `LibraryCommand::BrowseProject`.
    pub(crate) async fn browse_project(&mut self, id: ProjectId) {
        let Some(project) = self.browsed(&id).await else {
            return;
        };
        let state = match self.db.browse_state(self.paths.state_db(&project.id)).await {
            Ok(state) => state.unwrap_or_default(),
            Err(e) => return self.fail(format!("browsing {}: {e}", project.name)),
        };
        // A project whose research root cannot be found still shows its decisions and docs.
        let root = root_of(&self.paths, &project).ok();
        let title = |task| {
            state
                .tasks
                .iter()
                .find(|t| t.id == task)
                .map_or("task", |t| t.title.as_str())
        };
        let mut research = Vec::new();
        let mut listings = Vec::new();
        if let Some(root) = &root {
            for row in state.summaries {
                let dir = files::task_dir(root, row.task, title(row.task));
                research.push(summary(row, dir));
            }
            for (task, r) in &state.research {
                let dir = files::task_dir(root, *task, title(*task));
                listings.push(BrowsedListing {
                    task: *task,
                    listing: listing(&dir, r),
                });
            }
        }
        let docs = docs_repo(&project)
            .map(|repo| crate::docs::list(&repo))
            .unwrap_or_default();
        self.emit(Event::Library(LibraryEvent::ProjectBrowsed(Box::new(
            BrowsedProject {
                project: project.summary(&self.paths),
                tasks: state.tasks,
                decisions: state.decisions,
                research,
                listings,
                docs,
            },
        ))));
    }

    /// `LibraryCommand::LoadBrowsed`.
    pub(crate) async fn load_browsed(&mut self, id: ProjectId, what: BrowseWhat) {
        let Some(project) = self.browsed(&id).await else {
            return;
        };
        let document = match &what {
            BrowseWhat::Doc(path) => {
                let Some(repo) = docs_repo(&project) else {
                    return self.fail(format!("{} has no repo docs", project.name));
                };
                crate::docs::load(&repo, path).map_err(|e| e.to_string())
            }
            BrowseWhat::Research {
                what: ResearchWhat::Listing,
                ..
            } => Err("a browsed project's listings come with it".to_owned()),
            BrowseWhat::Research { task, what } => self
                .browsed_research(&project, *task, what)
                .await
                .map_err(|e| e.to_string()),
        };
        match document {
            Ok(document) => self.emit(Event::Library(LibraryEvent::BrowsedLoaded {
                doc: BrowsedDoc { project: id, what },
                document,
            })),
            Err(message) => self.fail(message),
        }
    }

    async fn browsed_research(
        &self,
        project: &Project,
        task: dovetail_api::tracking::task::TaskId,
        what: &ResearchWhat,
    ) -> Result<dovetail_api::research::ResearchDocument, LoadError> {
        let path = self.paths.state_db(&project.id);
        let (t, r) = self.db.browse_task(path, task).await?.ok_or_else(|| {
            LoadError::Missing(format!("{} has no task #{}", project.name, task.0))
        })?;
        let root = root_of(&self.paths, project)?;
        let dir = files::task_dir(&root, task, &t.title);
        match content_of(&dir, task, &t.title, &r, what)? {
            ResearchContent::Document(document) => Ok(document),
            ResearchContent::Listing(_) => Err(LoadError::Missing(
                "a browsed project's listings come with it".into(),
            )),
        }
    }
}

/// A code project's first repo, for its `docs/`; `None` for a research project, which has none.
fn docs_repo(project: &Project) -> Option<PathBuf> {
    project.first_repo().map(|r| r.path.clone())
}

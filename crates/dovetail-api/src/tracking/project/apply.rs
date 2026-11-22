//! How `project` events change the snapshot.

use super::ProjectEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: ProjectEvent) {
    match event {
        ProjectEvent::Upserted(project) => {
            if state.project.as_ref().is_some_and(|p| p.id == project.id) {
                state.project = Some(project.clone());
            }
            upsert(&mut state.projects, project, |p| p.id.clone());
        }
        // Nothing to store: the UI acts on these as they pass through.
        ProjectEvent::Opening { .. } | ProjectEvent::RecipeStarted { .. } => {}
    }
}

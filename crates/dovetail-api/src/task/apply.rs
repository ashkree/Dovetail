//! How `task` events change the snapshot.

use super::TaskEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: TaskEvent) {
    match event {
        TaskEvent::Upserted(task) => upsert(&mut state.tasks, task, |t| t.id),
        TaskEvent::Removed(id) => state.tasks.retain(|t| t.id != id),
    }
}

//! How `worktree` events change the snapshot.

use super::WorktreeEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: WorktreeEvent) {
    match event {
        WorktreeEvent::Upserted(info) => upsert(&mut state.worktrees, info, |w| w.agent.clone()),
    }
}

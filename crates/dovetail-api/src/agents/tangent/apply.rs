//! How `tangent` events change the snapshot.

use super::TangentEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: TangentEvent) {
    match event {
        TangentEvent::Upserted(tangent) => upsert(&mut state.tangents, tangent, |t| t.task.id),
        TangentEvent::Removed(id) => state.tangents.retain(|t| t.task.id != id),
        TangentEvent::Loaded { .. } => {}
    }
}

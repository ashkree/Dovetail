//! How `effort` events change the snapshot.

use super::EffortEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: EffortEvent) {
    match event {
        EffortEvent::Upserted(effort) => upsert(&mut state.efforts, effort, |e| e.id),
    }
}

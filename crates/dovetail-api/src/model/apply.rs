//! How `model` events change the snapshot.

use super::ModelEvent;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: ModelEvent) {
    match event {
        ModelEvent::Changed(models) => state.models = models,
    }
}

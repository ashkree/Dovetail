//! How `research` events change the snapshot.

use super::ResearchEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: ResearchEvent) {
    match event {
        ResearchEvent::Upserted(summary) => upsert(&mut state.research, summary, |r| r.task),
        // Nothing to store: the UI shows the content as the event passes through.
        ResearchEvent::Loaded { .. } => {}
    }
}

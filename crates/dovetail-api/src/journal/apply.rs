//! How `journal` events change the snapshot.

use super::JournalEvent;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: JournalEvent) {
    match event {
        JournalEvent::Changed(journal) => state.journal = Some(journal),
    }
}

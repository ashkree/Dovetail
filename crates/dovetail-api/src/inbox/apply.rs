//! How `inbox` events change the snapshot.

use super::InboxEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: InboxEvent) {
    match event {
        InboxEvent::Upserted(item) => upsert(&mut state.inbox, item, |i| i.id),
        InboxEvent::Resolved { id } => state.inbox.retain(|i| i.id != id),
    }
}

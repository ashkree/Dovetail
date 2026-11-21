//! How `usage` events change the snapshot.

use super::UsageEvent;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: UsageEvent) {
    match event {
        UsageEvent::Changed(usage) => state.usage = usage,
    }
}

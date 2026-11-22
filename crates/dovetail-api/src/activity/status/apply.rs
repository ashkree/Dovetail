//! How `status` events change the snapshot.

use super::StatusEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: StatusEvent) {
    match event {
        StatusEvent::TotalsChanged(totals) => state.totals = totals,
        StatusEvent::WarningSet(warning) => upsert(&mut state.warnings, warning, |w| w.id.clone()),
        StatusEvent::WarningCleared(id) => state.warnings.retain(|w| w.id != id),
        StatusEvent::UsageChanged(usage) => state.usage = usage,
        // Nothing to store: the open usage menu takes it in.
        StatusEvent::UsageLoaded(_) => {}
    }
}

//! How `status` events change the snapshot.

use super::StatusEvent;
use crate::apply::upsert;
use crate::model::DriverKind;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: StatusEvent) {
    match event {
        StatusEvent::TotalsChanged(totals) => state.totals = totals,
        StatusEvent::WarningSet(warning) => upsert(&mut state.warnings, warning, |w| w.id.clone()),
        StatusEvent::WarningCleared(id) => state.warnings.retain(|w| w.id != id),
        StatusEvent::UsageChanged(usage) => state.usage = usage,
        StatusEvent::UsageLoaded(reports) => {
            if let Some(agy) = reports.into_iter().find(|r| r.driver == DriverKind::Agy) {
                state.agy_usage = Some(agy);
            }
        }
    }
}

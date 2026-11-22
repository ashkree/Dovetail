//! How `app` events change the snapshot.

use super::AppEvent;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: AppEvent) {
    match event {
        AppEvent::UpgradeBuilding(on) => state.upgrade_building = on,
        // Nothing to store: the UI shows these as they pass through.
        AppEvent::CommandFailed { .. }
        | AppEvent::TerminalLend
        | AppEvent::TerminalReturned { .. }
        | AppEvent::Pasted { .. }
        | AppEvent::UpgradeRestarting => {}
    }
}

//! How `upgrade` events change the snapshot.

use super::UpgradeEvent;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: UpgradeEvent) {
    match event {
        UpgradeEvent::Building(on) => state.upgrade_building = on,
        // Nothing to store: the UI quits as the event passes through.
        UpgradeEvent::Restarting => {}
    }
}

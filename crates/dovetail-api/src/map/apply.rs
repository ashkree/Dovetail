//! How `map` events change the snapshot.

use super::MapEvent;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: MapEvent) {
    match event {
        MapEvent::Upserted(map) => {
            // Most recently drawn first: a redrawn map moves to the front.
            state.maps.retain(|m| m.id != map.id);
            state.maps.insert(0, map);
        }
        // Nothing to store: the UI keeps what it shows.
        MapEvent::Loaded(_) => {}
    }
}

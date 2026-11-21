//! How `decision` events change the snapshot.

use super::DecisionEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: DecisionEvent) {
    match event {
        DecisionEvent::Added(decision) => upsert(&mut state.decisions, decision, |d| d.id),
    }
}

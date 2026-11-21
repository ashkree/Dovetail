//! How `design` events change the snapshot.

use super::DesignEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: DesignEvent) {
    match event {
        DesignEvent::PreviewChanged(preview) => {
            upsert(&mut state.previews, preview, |p| p.agent.clone())
        }
        // A set with no slots is how the core says the slice declared none.
        DesignEvent::VariantsUpserted(set) if set.slots.is_empty() => {
            state.variants.retain(|v| v.agent != set.agent)
        }
        DesignEvent::VariantsUpserted(set) => upsert(&mut state.variants, set, |v| v.agent.clone()),
    }
}

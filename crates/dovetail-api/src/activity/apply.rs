//! How `activity` events change the snapshot.

use super::ActivityEvent;
use crate::apply::push_bounded;
use crate::snapshot::{ACTIVITY_TAIL, MESSAGE_TAIL};
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: ActivityEvent) {
    match event {
        ActivityEvent::Added(item) => push_bounded(&mut state.activity, item, ACTIVITY_TAIL),
        ActivityEvent::MessageAdded(message) => {
            push_bounded(&mut state.messages, message, MESSAGE_TAIL)
        }
    }
}

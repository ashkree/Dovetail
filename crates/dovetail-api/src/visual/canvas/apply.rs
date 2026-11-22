//! How `canvas` events change the snapshot.

use super::CanvasEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: CanvasEvent) {
    match event {
        CanvasEvent::BlockUpserted(block) => upsert(&mut state.blocks, block, |b| b.id),
        CanvasEvent::CardUpserted(card) => upsert(&mut state.cards, card, |c| c.id),
        CanvasEvent::CardRemoved(id) => {
            state.cards.retain(|c| c.id != id);
            state.card_links.retain(|l| l.from != id && l.to != id);
        }
        CanvasEvent::CardLinked(link) => {
            match state
                .card_links
                .iter_mut()
                .find(|l| l.from == link.from && l.to == link.to)
            {
                Some(existing) => *existing = link,
                None => state.card_links.push(link),
            }
        }
        CanvasEvent::CardUnlinked { from, to } => {
            state.card_links.retain(|l| !(l.from == from && l.to == to))
        }
        // Nothing to store: the UI shows the body as the event passes through.
        CanvasEvent::CardLoaded { .. } => {}
    }
}

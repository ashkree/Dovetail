//! The `canvas` feature's events.

use crate::canvas::{Block, Card, CardId, CardLink};
use serde::{Deserialize, Serialize};

/// A change to the client's snapshot, or an answer, in the feature `canvas`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasEvent {
    /// A block was created or renamed (ADR 0027).
    BlockUpserted(Block),
    /// A card was added, placed, moved or changed. It carries the whole card.
    CardUpserted(Card),
    /// A card was deleted or dropped; its links go with it.
    CardRemoved(CardId),
    /// Two cards were linked, or a link was relabelled.
    CardLinked(CardLink),
    /// A link was removed.
    CardUnlinked {
        /// The card it started at.
        from: CardId,
        /// The card it led to.
        to: CardId,
    },
    /// A card's body, in answer to [`super::CanvasCommand::LoadCard`]: the target's text for a reference. Changes
    /// nothing in the snapshot.
    CardLoaded {
        /// The card.
        id: CardId,
        /// The body, as markdown.
        body: String,
    },
}

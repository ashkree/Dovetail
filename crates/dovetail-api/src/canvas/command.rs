//! The `canvas` feature's commands.

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;
use crate::canvas::{BlockId, CardId, Layout, NewCard, Point};
use crate::conversation::EntryId;
use crate::effort::EffortId;

/// Canvases: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasCommand {
    /// Add a block to an effort's canvas (ADR 0027). The core places it in the empty space
    /// nearest `near` (the centre of the user's view), or right of everything without it.
    /// Answers with [`CanvasEvent::BlockUpserted`].
    CreateBlock {
        /// The effort.
        effort: EffortId,
        /// Its title; empty becomes the layout's name.
        title: String,
        /// How it lays out its cards.
        layout: Layout,
        /// Where the user is looking, if anywhere.
        #[serde(default)]
        near: Option<Point>,
    },
    /// Rename a block.
    RenameBlock {
        /// The block.
        id: BlockId,
        /// The new title.
        title: String,
    },
    /// Add a card: into `card.block` where the core finds room (under `link_from` when that card
    /// is in the same block), else into the effort's incoming strip. With `link_from`, the new
    /// card is also linked from that card. The core checks that a reference's target exists and
    /// answers with [`CanvasEvent::CardUpserted`].
    AddCard {
        /// The card.
        card: NewCard,
        /// A card to link from, of the same effort.
        #[serde(default)]
        link_from: Option<CardId>,
    },
    /// Place an incoming card in a block, where the core finds room: under the placed card that
    /// links to it, if any. With no block named: that card's block, else the effort's newest
    /// block, else a new free area. Refused for a card already placed.
    PlaceCard {
        /// The card.
        id: CardId,
        /// The block, if the user chose one.
        #[serde(default)]
        block: Option<BlockId>,
    },
    /// The user has seen a card an agent put into a block (ADR 0027).
    SeeCard {
        /// The card.
        id: CardId,
    },
    /// Move a placed card. Refused for an incoming card.
    MoveCard {
        /// The card.
        id: CardId,
        /// Position in canvas pixels.
        x: i64,
        /// Position in canvas pixels.
        y: i64,
    },
    /// Delete a card and its links. The card's target is not touched.
    DeleteCard {
        /// The card.
        id: CardId,
    },
    /// Drop an incoming card (`x` in the strip). Refused for a placed card.
    DropIncoming {
        /// The card.
        id: CardId,
    },
    /// Link two cards of one effort; linking them again changes the label.
    LinkCards {
        /// Where the link starts.
        from: CardId,
        /// Where it leads.
        to: CardId,
        /// What it says.
        #[serde(default)]
        label: String,
    },
    /// Remove a link.
    UnlinkCards {
        /// Where the link starts.
        from: CardId,
        /// Where it leads.
        to: CardId,
    },
    /// Turn a chat reply into an idea card in an effort's incoming strip, by copying its text
    /// (ADR 0025, point 6). Refused for an unknown entry or one with no text.
    CardFromReply {
        /// Whose conversation.
        agent: AgentId,
        /// The entry.
        entry: EntryId,
        /// The effort the card goes to.
        effort: EffortId,
    },
    /// Fetch a card's body. The core answers with [`CanvasEvent::CardLoaded`].
    LoadCard {
        /// The card.
        id: CardId,
    },
}

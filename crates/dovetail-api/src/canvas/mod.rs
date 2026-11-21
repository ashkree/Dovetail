//! The `canvas` feature: canvases: an effort's blocks and cards.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::CanvasCommand;
pub use event::CanvasEvent;

use serde::{Deserialize, Serialize};
use crate::canvas::geometry;
use crate::decision::Decision;
use crate::effort::EffortId;
use crate::map::Map;
use crate::task::Task;

/// Identifies a block. Matches the `blocks.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockId(pub i64);

/// Identifies a card. Matches the `cards.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CardId(pub i64);

/// How a block lays out its cards and what it is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// A root question and the questions, findings and ideas under it: a card added under
    /// another goes one row below it.
    QuestionTree,
    /// Options side by side: each option heads a column and its evidence goes below it.
    OptionsCompare,
    /// No structure: cards fill a grid row by row.
    FreeArea,
}

/// A block on an effort's canvas: a framed region with a layout, at a position the core chose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Block id.
    pub id: BlockId,
    /// The effort whose canvas it is on.
    pub effort: EffortId,
    /// Its title.
    pub title: String,
    /// How it lays out its cards.
    pub layout: Layout,
    /// Its top-left corner on the canvas, in canvas pixels.
    pub x: i64,
    /// Its top-left corner on the canvas, in canvas pixels.
    pub y: i64,
}

/// A point on a canvas, in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    /// Rightwards.
    pub x: i64,
    /// Downwards.
    pub y: i64,
}

/// Sizes the core places by and the GUI draws with, in canvas pixels. A card's cell in a block is
/// `CARD_W + GAP` wide and `CARD_H + GAP` high, starting `PAD` from the block's left edge and
/// `HEADER` from its top.
pub mod geometry {
    /// A card's width.
    pub const CARD_W: i64 = 250;
    /// A card's height, as placed (a long card may draw taller).
    pub const CARD_H: i64 = 100;
    /// Space between two cards.
    pub const GAP: i64 = 40;
    /// The block's title band.
    pub const HEADER: i64 = 40;
    /// Space between a block's frame and its cards.
    pub const PAD: i64 = 20;
    /// Space between two blocks.
    pub const BLOCK_GAP: i64 = 80;
    /// Columns a free area or a row of a question tree fills before it wraps.
    pub const COLUMNS: i64 = 4;
}

/// What a card is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    /// A research finding (a reference).
    Finding,
    /// A research source (a reference).
    Source,
    /// A thought worth keeping; its words are in [`Card::text`].
    Idea,
    /// A logged decision (a reference).
    Decision,
    /// A question to answer; its words are in [`Card::text`].
    Question,
    /// A map; shows its latest version (a reference).
    Map,
    /// A task (a reference).
    Task,
}

impl CardKind {
    /// Whether a card of this kind keeps its own words rather than pointing at something.
    pub fn has_own_text(self) -> bool {
        matches!(self, CardKind::Idea | CardKind::Question)
    }
}

/// What a [`CardRef`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// A row of `findings`.
    Finding,
    /// A row of `sources`.
    Source,
    /// A row of `decisions`.
    Decision,
    /// A row of `maps` (its id is text).
    Map,
    /// A row of `tasks`.
    Task,
}

impl RefKind {
    /// The card kind a reference of this kind makes.
    pub fn card_kind(self) -> CardKind {
        match self {
            RefKind::Finding => CardKind::Finding,
            RefKind::Source => CardKind::Source,
            RefKind::Decision => CardKind::Decision,
            RefKind::Map => CardKind::Map,
            RefKind::Task => CardKind::Task,
        }
    }
}

/// What a card points at: the kind of row and its id, as text because maps have text ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CardRef {
    /// Which table.
    pub kind: RefKind,
    /// The row's id.
    pub id: String,
}

/// One card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    /// Card id.
    pub id: CardId,
    /// The effort it belongs to.
    pub effort: EffortId,
    /// The block it is placed in; `None`: it is in the effort's incoming strip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockId>,
    /// What it is.
    pub kind: CardKind,
    /// Position on the canvas, in canvas pixels; 0 while incoming.
    pub x: i64,
    /// Position on the canvas, in canvas pixels; 0 while incoming.
    pub y: i64,
    /// What it points at; `None` for an idea or a question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<CardRef>,
    /// What the card shows as its title, filled by the core: the target's title for a reference,
    /// the first line of [`Card::text`] for an idea or a question.
    pub title: String,
    /// The card's own words (idea, question); empty for a reference.
    #[serde(default)]
    pub text: String,
    /// The target is gone (a damaged database); the title then says so.
    #[serde(default)]
    pub missing: bool,
    /// `user` or the id of the agent that added it.
    pub added_by: String,
    /// When it was added, in Unix milliseconds.
    pub created_ms: i64,
    /// The user has seen it. A card an agent puts into a block is unseen until the user selects
    /// it (ADR 0027).
    #[serde(default = "seen_default")]
    pub seen: bool,
}

fn seen_default() -> bool {
    true
}

/// A link between two cards of one effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardLink {
    /// The card the link starts at (the question, the parent).
    pub from: CardId,
    /// The card it leads to.
    pub to: CardId,
    /// What the link says, if anything.
    #[serde(default)]
    pub label: String,
}

/// Fields for a new card, in [`CanvasCommand::AddCard`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCard {
    /// The effort it belongs to.
    pub effort: EffortId,
    /// The block to put it in, where the core finds room; `None` puts it in incoming.
    #[serde(default)]
    pub block: Option<BlockId>,
    /// What it is. A reference's `kind` must match [`RefKind::card_kind`].
    pub kind: CardKind,
    /// What it points at; required for a finding, source, decision, map or task card, and absent
    /// for an idea or a question.
    #[serde(default)]
    pub reference: Option<CardRef>,
    /// The words of an idea or a question.
    #[serde(default)]
    pub text: String,
}

/// The first line of `text`, cut at 120 characters: the title of an idea or question card.
pub fn card_title(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.trim().chars().take(120).collect()
}

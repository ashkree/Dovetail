//! Canvases and cards (ADR 0025, `docs/design-v2.md` §6).
//!
//! A canvas is a board of cards for one effort. A card either holds the user's or an agent's own
//! words (an idea, a question) or points at something stored elsewhere (a finding, a source, a
//! decision, a map, a task) by [`CardRef`]. A card with no canvas is in the effort's **incoming**
//! strip until the user places it. Only the user moves or deletes a card: the agent tools add to
//! incoming and link, nothing more.

use serde::{Deserialize, Serialize};

use crate::effort::EffortId;

/// Identifies a canvas. Matches the `canvases.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct CanvasId(pub i64);

/// Identifies a card. Matches the `cards.id` row id in `state.db`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct CardId(pub i64);

/// How a canvas is laid out and what it is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    /// A root question and the questions, findings and ideas under it.
    QuestionTree,
    /// Options side by side against the same criteria.
    OptionsCompare,
    /// No structure.
    FreeBoard,
}

/// A canvas of an effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Canvas {
    /// Canvas id.
    pub id: CanvasId,
    /// The effort it belongs to.
    pub effort: EffortId,
    /// Its title.
    pub title: String,
    /// Its layout.
    pub framework: Framework,
}

/// What a card is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CardRef {
    /// Which table.
    pub kind: RefKind,
    /// The row's id.
    pub id: String,
}

/// One card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Card {
    /// Card id.
    pub id: CardId,
    /// The effort it belongs to.
    pub effort: EffortId,
    /// The canvas it is placed on; `None`: it is in the effort's incoming strip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<CanvasId>,
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
}

/// A link between two cards of one effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CardLink {
    /// The card the link starts at (the question, the parent).
    pub from: CardId,
    /// The card it leads to.
    pub to: CardId,
    /// What the link says, if anything.
    #[serde(default)]
    pub label: String,
}

/// Fields for a new card, in [`crate::Command::AddCard`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewCard {
    /// The effort it belongs to.
    pub effort: EffortId,
    /// The canvas to place it on, at `x`, `y`; `None` puts it in incoming.
    #[serde(default)]
    pub canvas: Option<CanvasId>,
    /// What it is. A reference's `kind` must match [`RefKind::card_kind`].
    pub kind: CardKind,
    /// Position on the canvas; ignored for incoming.
    #[serde(default)]
    pub x: i64,
    /// Position on the canvas; ignored for incoming.
    #[serde(default)]
    pub y: i64,
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

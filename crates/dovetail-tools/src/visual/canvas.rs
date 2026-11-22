//! Tools of the `canvas` suite: blocks, cards, and links on an effort's canvas.

use dovetail_api::canvas::{BlockId, CardId, CardKind, Layout};
use dovetail_api::effort::EffortId;
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::{ToolError, text};

/// Tools in the `canvas` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasTool {
    /// `canvas_add_block` (ADR 0027): a block where the effort's canvas has room.
    AddBlock(CanvasAddBlock),
    /// `canvas_add_card` (ADR 0025, ADR 0027): a card into a block, or the incoming strip.
    AddCard(CanvasAddCard),
    /// `canvas_link_cards` (ADR 0025): link two cards.
    LinkCards(CanvasLinkCards),
}

impl CanvasTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            CanvasTool::AddBlock(_) => "canvas_add_block",
            CanvasTool::AddCard(_) => "canvas_add_card",
            CanvasTool::LinkCards(_) => "canvas_link_cards",
        }
    }

    pub(crate) fn validate(&self, _role: Role) -> Result<(), ToolError> {
        match self {
            CanvasTool::AddBlock(b) => {
                if b.block_title().is_empty() {
                    return Err(ToolError::Empty("title"));
                }
                Ok(())
            }
            CanvasTool::AddCard(c) => {
                if c.under.is_some() && c.block.is_none() {
                    return Err(ToolError::BadCard(
                        "`under` places a card in a block: name the `block` too",
                    ));
                }
                if c.kind.has_own_text() {
                    if c.reference.is_some() {
                        return Err(ToolError::BadCard(
                            "an idea or question card holds its own `text`: leave `reference` out",
                        ));
                    }
                    text("text", &c.text)
                } else {
                    if !c.text.trim().is_empty() {
                        return Err(ToolError::BadCard(
                            "a card that points at something has no `text` of its own: leave it out",
                        ));
                    }
                    match &c.reference {
                        Some(r) if r.trim().is_empty() => Err(ToolError::Empty("reference")),
                        Some(_) => Ok(()),
                        None => Err(ToolError::BadCard(
                            "a finding, source, decision, map or task card needs its `reference`: the id of what it points at",
                        )),
                    }
                }
            }
            CanvasTool::LinkCards(l) => {
                if l.from.0 <= 0 {
                    return Err(ToolError::NotACardId("from"));
                }
                if l.to.0 <= 0 {
                    return Err(ToolError::NotACardId("to"));
                }
                Ok(())
            }
        }
    }
}

/// `canvas_add_block` (ADR 0027): add a block to an effort's canvas. The core places it in empty
/// space; nothing already on the canvas moves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasAddBlock {
    /// The effort; `None` means the one the user last tagged (Unsorted when none).
    #[serde(default)]
    pub effort: Option<EffortId>,
    /// `question_tree`, `options_compare` or `free_area`.
    pub layout: Layout,
    /// What the block is about: the root question, the choice being compared.
    pub title: String,
}

impl CanvasAddBlock {
    /// The title, trimmed.
    pub fn block_title(&self) -> &str {
        self.title.trim()
    }
}

/// `canvas_add_card` (ADR 0025, ADR 0027): put a card into a block, where the core finds room,
/// or into the effort's incoming strip. Nothing already on the canvas moves; there is no tool to
/// move or delete a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasAddCard {
    /// The effort; `None` means the one the user last tagged (Unsorted when none), or the
    /// block's effort when `block` is given.
    #[serde(default)]
    pub effort: Option<EffortId>,
    /// The block to put it in; `None` puts it in the incoming strip.
    #[serde(default)]
    pub block: Option<BlockId>,
    /// A card in that block to put it under and link it from: in a question tree the question
    /// it answers, in an options compare the option it is evidence for.
    #[serde(default)]
    pub under: Option<CardId>,
    /// What the card is: `idea` or `question` with `text`, or `finding`, `source`, `decision`,
    /// `map` or `task` with `reference`.
    pub kind: CardKind,
    /// The words of an idea or a question.
    #[serde(default)]
    pub text: String,
    /// The id of what a finding, source, decision, map or task card points at (a map's id is
    /// text; the others are numbers). It must exist.
    #[serde(default)]
    pub reference: Option<String>,
}

/// `canvas_link_cards` (ADR 0025): link two cards of one effort, such as a question to the
/// finding that answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasLinkCards {
    /// The card the link starts at.
    pub from: CardId,
    /// The card it leads to.
    pub to: CardId,
    /// What the link says, if anything.
    #[serde(default)]
    pub label: String,
}

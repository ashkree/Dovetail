//! Canvases and cards (ADR 0025): the handlers of the canvas commands and of the agents' two
//! canvas tools.
//!
//! The database is the only copy (the snapshot reads it, as it reads tasks), so every handler
//! writes through the DB thread first and then announces the stored row. Only the user's
//! commands move, place or delete a card; the agents' tools add to incoming and link.

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::tools::{CanvasAddCard, CanvasLinkCards};
use dovetail_api::{
    ActivityKind, AgentId, CanvasId, Card, CardId, CardKind, CardRef, EffortId, EntryId, Event,
    Framework, NewCard, RefKind, TaskId, UNSORTED_EFFORT,
};

use crate::db::DbError;
use crate::service::Core;
use crate::tools::{error, ok};

/// `added_by` of a card the user made.
const USER: &str = "user";

impl Core {
    pub(crate) async fn create_canvas(&mut self, effort: EffortId, title: String, fw: Framework) {
        match self.db.create_canvas(effort, title, fw).await {
            Ok(canvas) => self.emit(Event::CanvasUpserted(canvas)),
            Err(e) => self.fail(e),
        }
    }

    pub(crate) async fn rename_canvas(&mut self, id: CanvasId, title: String) {
        match self.db.rename_canvas(id, title).await {
            Ok(canvas) => self.emit(Event::CanvasUpserted(canvas)),
            Err(e) => self.fail(e),
        }
    }

    /// Adds a card for `added_by`, then announces it and its link.
    async fn add_card_as(
        &mut self,
        card: NewCard,
        link_from: Option<CardId>,
        added_by: &str,
    ) -> Result<Card, DbError> {
        let (card, link) = self
            .db
            .add_card(card, link_from, added_by.to_owned())
            .await?;
        self.emit(Event::CardUpserted(card.clone()));
        if let Some(link) = link {
            self.emit(Event::CardLinked(link));
        }
        Ok(card)
    }

    pub(crate) async fn add_card(&mut self, card: NewCard, link_from: Option<CardId>) {
        if let Err(e) = self.add_card_as(card, link_from, USER).await {
            self.fail(e);
        }
    }

    pub(crate) async fn place_card(&mut self, id: CardId, canvas: CanvasId, x: i64, y: i64) {
        match self.db.place_card(id, canvas, x, y).await {
            Ok(card) => self.emit(Event::CardUpserted(card)),
            Err(e) => self.fail(e),
        }
    }

    pub(crate) async fn move_card(&mut self, id: CardId, x: i64, y: i64) {
        match self.db.move_card(id, x, y).await {
            Ok(card) => self.emit(Event::CardUpserted(card)),
            Err(e) => self.fail(e),
        }
    }

    pub(crate) async fn delete_card(&mut self, id: CardId, incoming_only: bool) {
        match self.db.delete_card(id, incoming_only).await {
            Ok(()) => self.emit(Event::CardRemoved(id)),
            Err(e) => self.fail(e),
        }
    }

    pub(crate) async fn link_cards(&mut self, from: CardId, to: CardId, label: String) {
        match self.db.link_cards(from, to, label).await {
            Ok(link) => self.emit(Event::CardLinked(link)),
            Err(e) => self.fail(e),
        }
    }

    pub(crate) async fn unlink_cards(&mut self, from: CardId, to: CardId) {
        match self.db.unlink_cards(from, to).await {
            Ok(()) => self.emit(Event::CardUnlinked { from, to }),
            Err(e) => self.fail(e),
        }
    }

    pub(crate) async fn load_card(&mut self, id: CardId) {
        match self.db.load_card_body(id).await {
            Ok(body) => self.emit(Event::CardLoaded { id, body }),
            Err(e) => self.fail(e),
        }
    }

    /// A chat reply becomes an idea card in the effort's incoming strip, by copying its text
    /// (ADR 0025, point 6): the card keeps the words whatever happens to the conversation.
    pub(crate) async fn card_from_reply(
        &mut self,
        agent: AgentId,
        entry: EntryId,
        effort: EffortId,
    ) {
        let text = self
            .tails
            .get(&agent)
            .and_then(|t| t.entries.iter().find(|e| e.id == entry))
            .map(|e| e.text.clone());
        let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
            self.fail("that reply has no text to keep (or it is no longer loaded)");
            return;
        };
        let card = NewCard {
            effort,
            canvas: None,
            kind: CardKind::Idea,
            x: 0,
            y: 0,
            reference: None,
            text,
        };
        self.add_card(card, None).await;
    }

    /// A finding was filed under `task`: one finding card goes into its effort's incoming strip.
    /// Failing to make it never fails the filing; it is logged.
    pub(crate) async fn card_for_finding(
        &mut self,
        finding: dovetail_api::research::FindingId,
        task: TaskId,
        agent: &AgentId,
    ) {
        match self
            .db
            .add_finding_card(finding, task, agent.0.clone())
            .await
        {
            Ok(card) => self.emit(Event::CardUpserted(card)),
            Err(e) => tracing::warn!("making a card for finding {}: {e}", finding.0),
        }
    }

    /// `canvas_add_card`: validation already checked the shape of the call.
    pub(crate) async fn canvas_add_card_tool(
        &mut self,
        agent: &AgentId,
        call: CanvasAddCard,
    ) -> SocketResponseBody {
        let effort = call
            .effort
            .or_else(|| self.effort_tags.get(agent).copied())
            .unwrap_or(UNSORTED_EFFORT);
        let reference = match (call.kind.has_own_text(), call.reference) {
            (false, Some(id)) => {
                let kind = match call.kind {
                    CardKind::Finding => RefKind::Finding,
                    CardKind::Source => RefKind::Source,
                    CardKind::Decision => RefKind::Decision,
                    CardKind::Map => RefKind::Map,
                    _ => RefKind::Task,
                };
                Some(CardRef { kind, id })
            }
            _ => None,
        };
        let new = NewCard {
            effort,
            canvas: None,
            kind: call.kind,
            x: 0,
            y: 0,
            reference,
            text: call.text,
        };
        match self.add_card_as(new, None, &agent.0).await {
            Ok(card) => {
                self.activity(
                    ActivityKind::Task,
                    Some(agent),
                    format!(
                        "canvas: added card {} \"{}\" to incoming",
                        card.id.0, card.title
                    ),
                );
                ok(format!(
                    "Added card {} to the incoming strip of effort {}; the user decides whether it goes on the canvas.",
                    card.id.0, card.effort.0
                ))
            }
            Err(e) => error(format!("could not add the card: {e}")),
        }
    }

    /// `canvas_link_cards`.
    pub(crate) async fn canvas_link_cards_tool(
        &mut self,
        call: CanvasLinkCards,
    ) -> SocketResponseBody {
        match self.db.link_cards(call.from, call.to, call.label).await {
            Ok(link) => {
                let text = format!("Linked card {} to card {}.", link.from.0, link.to.0);
                self.emit(Event::CardLinked(link));
                ok(text)
            }
            Err(e) => error(format!("could not link the cards: {e}")),
        }
    }
}

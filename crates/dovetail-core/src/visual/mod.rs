//! Visual domain: canvas blocks/cards, design variants, slice previews, and map drawing.

pub(crate) mod canvas;
pub(crate) mod db;
pub(crate) mod design;
pub(crate) mod map;
pub(crate) mod preview;

use dovetail_api::Event;
use dovetail_api::agents::agent::AgentId;
use dovetail_api::visual::{
    canvas::CanvasCommand,
    design::DesignCommand,
    map::{MapCommand, MapEvent},
};
use dovetail_tools::{SocketResponseBody, canvas::CanvasTool, design::DesignTool, map::MapTool};

use crate::engine::Core;

/// State owned by the visual domain.
#[derive(Default)]
pub(crate) struct VisualState {
    /// Each slice's preview process and state (ADR 0026, `preview`).
    pub(crate) previews: preview::Previews,
}

impl Core {
    pub(crate) async fn handle_canvas_command(&mut self, command: CanvasCommand) {
        match command {
            CanvasCommand::CreateBlock {
                effort,
                title,
                layout,
                near,
            } => self.create_block(effort, title, layout, near).await,
            CanvasCommand::RenameBlock { id, title } => self.rename_block(id, title).await,
            CanvasCommand::AddCard { card, link_from } => self.add_card(card, link_from).await,
            CanvasCommand::PlaceCard { id, block } => self.place_card(id, block).await,
            CanvasCommand::SeeCard { id } => self.see_card(id).await,
            CanvasCommand::MoveCard { id, x, y } => self.move_card(id, x, y).await,
            CanvasCommand::DeleteCard { id } => self.delete_card(id, false).await,
            CanvasCommand::DropIncoming { id } => self.delete_card(id, true).await,
            CanvasCommand::LinkCards { from, to, label } => self.link_cards(from, to, label).await,
            CanvasCommand::UnlinkCards { from, to } => self.unlink_cards(from, to).await,
            CanvasCommand::CardFromReply {
                agent,
                entry,
                effort,
            } => self.card_from_reply(agent, entry, effort).await,
            CanvasCommand::LoadCard { id } => self.load_card(id).await,
        }
    }

    pub(crate) async fn handle_design_command(&mut self, command: DesignCommand) {
        match command {
            DesignCommand::StartPreview { agent } => self.start_preview(agent),
            DesignCommand::RestartPreview { agent } => self.restart_preview(agent),
            DesignCommand::SelectVariant {
                agent,
                slot,
                option,
            } => self.select_variant(agent, slot, option).await,
            DesignCommand::KeepVariants { agent } => self.keep_variants(agent).await,
        }
    }

    pub(crate) async fn handle_map_command(&mut self, command: MapCommand) {
        match command {
            MapCommand::Load { id, version } => match self.db.load_map(id.clone(), version).await {
                Ok(Some(map)) => self.emit(Event::Map(MapEvent::Loaded(map))),
                Ok(None) => self.fail(match version {
                    Some(v) => format!("map {} has no version {v}", id.0),
                    None => format!("there is no map {}", id.0),
                }),
                Err(e) => self.fail(e.to_string()),
            },
        }
    }

    pub(crate) async fn handle_canvas_tool(
        &mut self,
        agent: &AgentId,
        tool: CanvasTool,
    ) -> SocketResponseBody {
        match tool {
            CanvasTool::AddBlock(add) => self.canvas_add_block_tool(agent, add).await,
            CanvasTool::AddCard(add) => self.canvas_add_card_tool(agent, add).await,
            CanvasTool::LinkCards(link) => self.canvas_link_cards_tool(link).await,
        }
    }

    pub(crate) async fn handle_design_tool(
        &mut self,
        agent: &AgentId,
        tool: DesignTool,
    ) -> SocketResponseBody {
        match tool {
            DesignTool::DeclareVariants(declared) => {
                self.declare_variants_tool(agent, declared).await
            }
        }
    }

    pub(crate) async fn handle_map_tool(
        &mut self,
        agent: &AgentId,
        tool: MapTool,
    ) -> SocketResponseBody {
        match tool {
            MapTool::Draw(draw) => self.draw_map(agent, draw.0).await,
        }
    }
}

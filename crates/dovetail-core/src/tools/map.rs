//! `maps_draw` (M15, design 23i): validate, check lane links across maps, store a version.

use dovetail_api::{Event, agent::AgentId};
use dovetail_api::{map::MapEvent, socket::SocketResponseBody};
use dovetail_maps::{DrawMapInput, MapError, check_lane_links, report, validate};
use serde_json::{Value, json};

use super::{error, ok};
use crate::db::{Drawn, NewMapVersion};
use crate::service::Core;

impl Core {
    /// Answers `maps_draw`: the agent-readable report when the map is refused, else
    /// `{"id", "version"}` and a line saying how to redraw it.
    pub(super) async fn draw_map(
        &mut self,
        agent: &AgentId,
        arguments: Value,
    ) -> SocketResponseBody {
        let input: DrawMapInput = match serde_json::from_value(arguments) {
            Ok(input) => input,
            Err(e) => {
                return error(format!(
                    "The map was not saved: the input does not fit maps_draw's schema ({e}). Fix \
                     it and call maps_draw again with the whole map."
                ));
            }
        };
        let valid = match validate(&input) {
            Ok(valid) => valid,
            Err(errors) => return error(report(&errors)),
        };
        if valid.content.lanes.iter().any(|l| l.node.is_some()) {
            let known = match self.db.structure_nodes().await {
                Ok(known) => known,
                Err(e) => return error(format!("could not read the project's maps: {e}")),
            };
            let errors = check_lane_links(&valid.content, &known);
            if !errors.is_empty() {
                return error(report(&errors));
            }
        }
        let new = NewMapVersion {
            id: valid.requested_id,
            content: valid.content,
            task: self.agents.get(agent).and_then(|r| r.agent.task),
            drawn_by: agent.clone(),
        };
        let kind = new.content.kind;
        let saved = match self.db.save_map(new).await {
            Ok(Drawn::Saved(saved)) => saved,
            Ok(Drawn::KindChanged { id, was }) => {
                return error(report(&[MapError::KindChanged {
                    id: id.0,
                    was: was.as_str(),
                    now: kind.as_str(),
                }]));
            }
            Err(e) => return error(format!("could not save the map: {e}")),
        };
        let head = json!({ "id": saved.id.0, "version": saved.version });
        let what = if saved.version == 1 {
            "new map".to_owned()
        } else {
            format!("version {} of map", saved.version)
        };
        let text = format!(
            "{head}\nSaved {what} \"{}\" ({}). To change it, call maps_draw again with \"id\": \
             \"{}\" and the whole map.",
            saved.title,
            saved.kind.as_str(),
            saved.id.0
        );
        self.emit(Event::Map(MapEvent::Upserted(saved)));
        ok(text)
    }
}

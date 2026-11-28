//! The map painter the canvas uses (M15 f): `dovetail_maps::MapRenderer` behind the TUI's
//! [`MapPainter`] trait, so merman stays out of `dovetail-tui`.

use std::sync::Arc;

use dovetail_api::visual::map::Map;
use dovetail_maps::{MapRenderer, RenderOptions};
use dovetail_tui::canvas::{MapPainter, PaintError, Picture, Placed, PxRect};

/// Paints with merman (structure maps) or Dovetail's grid (swimlanes).
#[derive(Default)]
pub struct Painter {
    renderer: MapRenderer,
}

impl MapPainter for Painter {
    fn paint(&self, map: &Map, fit: (u32, u32)) -> Result<Picture, PaintError> {
        let options = RenderOptions {
            // The canvas rings the selection itself, over the cached picture.
            selected: None,
            fit: Some(fit),
        };
        let rendered = self
            .renderer
            .render(&map.content, &options)
            .map_err(|e| PaintError(e.to_string()))?;
        Ok(Picture {
            png: rendered.png,
            width: rendered.width,
            height: rendered.height,
            nodes: rendered
                .nodes
                .into_iter()
                .map(|n| Placed {
                    id: n.id,
                    rect: PxRect {
                        x: n.rect.x,
                        y: n.rect.y,
                        w: n.rect.width,
                        h: n.rect.height,
                    },
                    is_group: n.is_group,
                    step: n.step,
                })
                .collect(),
        })
    }
}

/// A shared painter for the event loop.
pub fn painter() -> Arc<dyn MapPainter> {
    Arc::new(Painter::default())
}

#[cfg(test)]
mod tests {
    use dovetail_api::visual::map::{MapCommand, MapEvent};
    use dovetail_api::{Command, Event, ServerMsg};
    use dovetail_mock::{MockConfig, MockCore};

    use super::*;

    /// The mock's two seeded maps, as `LoadMap` returns them.
    fn seeded() -> Vec<Map> {
        let mut core = MockCore::new(MockConfig::default());
        let ids: Vec<_> = core.state().maps.iter().map(|m| m.id.clone()).collect();
        let mut maps = Vec::new();
        for id in ids {
            core.handle(Command::Map(MapCommand::Load { id, version: None }));
            while let Some(scheduled) = core.next_message() {
                if let ServerMsg::Event {
                    event: Event::Map(MapEvent::Loaded(map)),
                    ..
                } = scheduled.msg
                {
                    maps.push(map);
                    break;
                }
            }
        }
        maps
    }

    /// Both map types paint inside the fit box, with every node's box inside the picture.
    #[test]
    fn both_seeded_maps_paint_with_their_nodes_inside_the_picture() {
        let maps = seeded();
        assert_eq!(maps.len(), 2);
        let painter = Painter::default();
        for map in &maps {
            let picture = painter.paint(map, (1600, 1000)).expect("paints");
            assert!(picture.png.starts_with(b"\x89PNG"), "{}", map.id.0);
            assert!(picture.width <= 1600 && picture.height <= 1000);
            assert!(!picture.nodes.is_empty(), "{}", map.id.0);
            for n in &picture.nodes {
                let r = n.rect;
                assert!(
                    r.x >= -1.0
                        && r.y >= -1.0
                        && r.x + r.w <= f64::from(picture.width) + 1.0
                        && r.y + r.h <= f64::from(picture.height) + 1.0,
                    "{} {:?} outside {}x{}",
                    map.id.0,
                    n,
                    picture.width,
                    picture.height
                );
            }
        }
    }
}

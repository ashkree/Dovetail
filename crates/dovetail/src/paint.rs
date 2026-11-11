//! The map painter the canvas uses (M15 f): `dovetail_maps::MapRenderer` behind the TUI's
//! [`MapPainter`] trait, so merman stays out of `dovetail-tui`.

use std::sync::Arc;

use dovetail_api::map::Map;
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

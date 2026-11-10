//! `MapRenderer`: a structure map in, a PNG and the node rectangles in PNG pixels out.
//!
//! merman is pre-1.0 and heavy, so everything that touches it lives in this file and
//! `mermaid.rs`. Positions come from merman's layout JSON (S5): `layout.FlowchartV2.nodes[]`
//! with centre `x`, `y`, `width`, `height` in SVG user units, and `bounds` for the drawing's
//! extent. The PNG's viewBox is those bounds widened by [`DIAGRAM_PADDING`] on every side, and the
//! PNG's pixels are the viewBox scaled by the raster plan's `effective_scale`.

use dovetail_api::map::{MapContent, MapKind, NodeId};
use merman::svg::export::{RasterFitBox, RasterOptions};
use merman::{OperationControl, PngRequest, RenderOutput, RenderRequest, Renderer, SvgRequest};
use serde_json::Value;

use crate::mermaid::to_mermaid;

/// merman's `diagramPadding` default: the viewBox extends the layout bounds by this on each side.
const DIAGRAM_PADDING: f64 = 8.0;

/// Why a map could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// Only structure maps go through merman; swimlane maps have their own layout.
    #[error("only structure maps are rendered with merman, this map is a {0} map")]
    NotStructure(&'static str),
    /// merman rejected the source or failed to rasterise it.
    #[error("merman failed to render the map: {0}")]
    Merman(String),
    /// merman returned no diagram (an empty map).
    #[error("the map has nothing to draw")]
    Empty,
    /// The layout JSON did not have the shape S5 recorded; merman changed under us.
    #[error("merman's layout JSON is not in the expected shape: {0}")]
    Layout(String),
}

/// What to render.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// The highlighted node, if any.
    pub selected: Option<NodeId>,
    /// Fit the picture inside this many pixels (width, height), keeping the aspect ratio. `None`
    /// renders at merman's natural size. Render at the pane's size: bigger images cost memory.
    pub fit: Option<(u32, u32)>,
}

/// A rectangle in PNG pixels, from the top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl PixelRect {
    /// Whether the point (px, py) is inside.
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

/// Where one node (or group) was drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct NodePlacement {
    /// The map node.
    pub id: NodeId,
    /// Its box in PNG pixels.
    pub rect: PixelRect,
    /// True for a group parent: its box encloses its children, so a hit test should prefer a
    /// non-group placement when both contain the point.
    pub is_group: bool,
}

/// A rendered map.
#[derive(Debug, Clone)]
pub struct RenderedMap {
    /// The PNG, transparent background.
    pub png: Vec<u8>,
    /// PNG width in pixels.
    pub width: u32,
    /// PNG height in pixels.
    pub height: u32,
    /// One entry per node that was drawn, in the map's node order.
    pub nodes: Vec<NodePlacement>,
}

impl RenderedMap {
    /// The node under a pixel: the innermost box (a node before the group around it).
    pub fn hit_test(&self, px: f64, py: f64) -> Option<&NodeId> {
        let hits = self.nodes.iter().filter(|n| n.rect.contains(px, py));
        let best = hits.min_by(|a, b| {
            (a.is_group, a.rect.width * a.rect.height)
                .partial_cmp(&(b.is_group, b.rect.width * b.rect.height))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        best.map(|n| &n.id)
    }
}

/// Renders structure maps with merman.
pub struct MapRenderer {
    renderer: Renderer,
}

impl Default for MapRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl MapRenderer {
    /// A renderer. System fonts load on the first PNG render, not here.
    pub fn new() -> Self {
        Self {
            renderer: Renderer::new(),
        }
    }

    /// The Mermaid source for a structure map, as [`MapRenderer::render`] would draw it.
    pub fn mermaid(content: &MapContent, selected: Option<&NodeId>) -> String {
        to_mermaid(content, selected).0
    }

    /// Renders a structure map to PNG and finds where each node landed.
    pub fn render(
        &self,
        content: &MapContent,
        options: &RenderOptions,
    ) -> Result<RenderedMap, RenderError> {
        if content.kind != MapKind::Structure {
            return Err(RenderError::NotStructure(content.kind.as_str()));
        }
        if content.nodes.is_empty() {
            return Err(RenderError::Empty);
        }
        let (source, mermaid_ids) = to_mermaid(content, options.selected.as_ref());

        let raster = match options.fit {
            Some((w, h)) => RasterOptions::default().with_fit_to(RasterFitBox::contain(w, h)),
            None => RasterOptions::default().with_scale(1.0),
        };
        let png = self
            .renderer
            .render(RenderRequest::png(
                &source,
                OperationControl::new(),
                PngRequest {
                    svg: SvgRequest::default(),
                    options: raster,
                },
            ))
            .map_err(|e| RenderError::Merman(e.to_string()))?;
        let RenderOutput::Png(Some(png)) = png else {
            return Err(RenderError::Empty);
        };

        let layout = self
            .renderer
            .render(RenderRequest::layout_json(
                &source,
                OperationControl::new(),
                SvgRequest::default(),
            ))
            .map_err(|e| RenderError::Merman(e.to_string()))?;
        let RenderOutput::LayoutJson(Some(layout)) = layout else {
            return Err(RenderError::Empty);
        };
        let mut json = Vec::new();
        layout
            .layout()
            .write_json(&mut json)
            .map_err(|e| RenderError::Layout(e.to_string()))?;

        let plan = &png.plan;
        let nodes = extract_placements(&json, content, &mermaid_ids, plan.effective_scale)?;
        Ok(RenderedMap {
            png: png.bytes,
            width: plan.width_px,
            height: plan.height_px,
            nodes,
        })
    }
}

/// Reads node boxes from layout JSON and maps them to PNG pixels.
///
/// `scale` is PNG pixels per SVG unit. `mermaid_ids` is parallel to `content.nodes`.
pub(crate) fn extract_placements(
    layout_json: &[u8],
    content: &MapContent,
    mermaid_ids: &[String],
    scale: f64,
) -> Result<Vec<NodePlacement>, RenderError> {
    let root: Value =
        serde_json::from_slice(layout_json).map_err(|e| RenderError::Layout(e.to_string()))?;
    // `semantic` has its own `nodes`; the geometry is under `layout`.
    let flow = root
        .pointer("/layout/FlowchartV2")
        .or_else(|| root.get("FlowchartV2"))
        .ok_or_else(|| RenderError::Layout("no layout.FlowchartV2".into()))?;
    let min_x = flow
        .pointer("/bounds/min_x")
        .and_then(Value::as_f64)
        .ok_or_else(|| RenderError::Layout("no bounds.min_x".into()))?;
    let min_y = flow
        .pointer("/bounds/min_y")
        .and_then(Value::as_f64)
        .ok_or_else(|| RenderError::Layout("no bounds.min_y".into()))?;
    let boxes = flow
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| RenderError::Layout("no nodes array".into()))?;

    let (vb_x, vb_y) = (min_x - DIAGRAM_PADDING, min_y - DIAGRAM_PADDING);
    let mut placements = Vec::new();
    for (node, mid) in content.nodes.iter().zip(mermaid_ids) {
        let Some(b) = boxes
            .iter()
            .find(|b| b.get("id").and_then(Value::as_str) == Some(mid))
        else {
            continue;
        };
        let num = |k: &str| {
            b.get(k)
                .and_then(Value::as_f64)
                .ok_or_else(|| RenderError::Layout(format!("node {mid} has no {k}")))
        };
        let (cx, cy, w, h) = (num("x")?, num("y")?, num("width")?, num("height")?);
        placements.push(NodePlacement {
            id: node.id.clone(),
            rect: PixelRect {
                x: (cx - w / 2.0 - vb_x) * scale,
                y: (cy - h / 2.0 - vb_y) * scale,
                width: w * scale,
                height: h * scale,
            },
            is_group: b
                .get("is_cluster")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(placements)
}

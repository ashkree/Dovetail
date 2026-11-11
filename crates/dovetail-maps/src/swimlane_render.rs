//! A swimlane map to PNG: [`crate::swimlane::layout`]'s grid drawn as SVG by Dovetail itself,
//! then rasterised with resvg (decision 23e: "laid out by Dovetail on a grid, in the same visual
//! style"). Positions are exact: every step box is a grid rectangle scaled by a known factor, so
//! [`RenderedMap::nodes`] needs no layout JSON.

use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

use dovetail_api::map::{MapContent, NodeKind};
use resvg::{tiny_skia, usvg};

use crate::mermaid::kind_colors;
use crate::render::{NodePlacement, PixelRect, RenderError, RenderOptions, RenderedMap};
use crate::swimlane::{GridParams, SwimlaneLayout, layout};

/// SVG units per grid unit across and down. Grid units are shaped like terminal cells (about
/// twice as tall as wide), so the y factor is double.
const UNIT_X: f64 = 9.0;
const UNIT_Y: f64 = 18.0;
/// Label size in SVG units.
const FONT_SIZE: f64 = 14.0;
/// Rough width of one label character, for truncating labels to their box.
const CHAR_W: f64 = FONT_SIZE * 0.56;
/// merman's default raster size limit, kept the same for both map types.
const MAX_SIDE: u32 = 4096;
/// Lane band tints, alternating, and the text colour on them (dark theme, as structure maps).
const LANE_TINTS: [&str; 2] = ["#2a2d36", "#22252d"];
const LANE_TEXT: &str = "#d8d4cc";
const ARROW: &str = "#b9b4aa";

/// System fonts, loaded once per process (the first render pays for it, as with merman).
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Renders a swimlane map.
pub(crate) fn render(
    content: &MapContent,
    options: &RenderOptions,
) -> Result<RenderedMap, RenderError> {
    let grid =
        layout(content, GridParams::default()).map_err(|e| RenderError::Layout(e.to_string()))?;
    let svg = to_svg(content, &grid);
    let opt = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| RenderError::Raster(e.to_string()))?;
    let (w, h) = (
        f64::from(tree.size().width()),
        f64::from(tree.size().height()),
    );
    let scale = scale_for(w, h, options.fit);
    let (pw, ph) = (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    );
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or(RenderError::Empty)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale as f32, scale as f32),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|e| RenderError::Raster(e.to_string()))?;
    let nodes = grid
        .steps
        .iter()
        .map(|s| NodePlacement {
            id: s.node.clone(),
            rect: PixelRect {
                x: f64::from(s.rect.x) * UNIT_X * scale,
                y: f64::from(s.rect.y) * UNIT_Y * scale,
                width: f64::from(s.rect.w) * UNIT_X * scale,
                height: f64::from(s.rect.h) * UNIT_Y * scale,
            },
            is_group: false,
            step: Some(s.step),
        })
        .collect();
    Ok(RenderedMap {
        png,
        width: pw,
        height: ph,
        nodes,
    })
}

/// PNG pixels per SVG unit: fit inside `fit` keeping the aspect ratio, else 1; never past
/// [`MAX_SIDE`] on a side.
fn scale_for(w: f64, h: f64, fit: Option<(u32, u32)>) -> f64 {
    let wanted = match fit {
        Some((fw, fh)) => (f64::from(fw) / w).min(f64::from(fh) / h),
        None => 1.0,
    };
    wanted.min(f64::from(MAX_SIDE) / w.max(h))
}

/// Escapes text for an SVG text node.
fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Cuts a label to `width` SVG units, with an ellipsis.
fn fit_label(text: &str, width: f64) -> String {
    let max = ((width / CHAR_W).floor() as usize).max(1);
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn to_svg(content: &MapContent, grid: &SwimlaneLayout) -> String {
    let (w, h) = (
        f64::from(grid.width) * UNIT_X,
        f64::from(grid.height) * UNIT_Y,
    );
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="sans-serif" font-size="{FONT_SIZE}">"#
    );
    let _ = write!(
        s,
        r#"<defs><marker id="head" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{ARROW}"/></marker></defs>"#
    );
    for lane in &grid.lanes {
        let r = scaled(lane.rect);
        let tint = LANE_TINTS[lane.lane % LANE_TINTS.len()];
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{tint}"/>"#,
            r.0, r.1, r.2, r.3
        );
        let hd = scaled(lane.header);
        let _ = write!(
            s,
            r#"<text x="{}" y="{}" fill="{LANE_TEXT}" font-weight="bold" dominant-baseline="middle">{}</text>"#,
            hd.0 + 8.0,
            hd.1 + hd.3 / 2.0,
            esc(&fit_label(&lane.label, hd.2 - 12.0))
        );
    }
    for arrow in &grid.arrows {
        let points: Vec<String> = arrow
            .route
            .iter()
            .map(|p| format!("{},{}", f64::from(p.x) * UNIT_X, f64::from(p.y) * UNIT_Y))
            .collect();
        let _ = write!(
            s,
            r#"<polyline points="{}" fill="none" stroke="{ARROW}" stroke-width="2" marker-end="url(#head)"/>"#,
            points.join(" ")
        );
        if let Some(label) = &arrow.label {
            let _ = write!(
                s,
                r#"<text x="{}" y="{}" fill="{ARROW}" font-size="{}" text-anchor="middle">{}</text>"#,
                f64::from(arrow.label_at.x) * UNIT_X,
                f64::from(arrow.label_at.y) * UNIT_Y - 4.0,
                FONT_SIZE * 0.85,
                esc(&fit_label(
                    label,
                    f64::from(GridParams::default().gap) * UNIT_X * 2.0
                ))
            );
        }
    }
    for step in &grid.steps {
        let kind = content
            .nodes
            .iter()
            .find(|n| n.id == step.node)
            .map_or(NodeKind::Concept, |n| n.kind);
        let c = kind_colors(kind);
        let (x, y, bw, bh) = scaled(step.rect);
        let dash = if c.dashed {
            r#" stroke-dasharray="6 4""#
        } else {
            ""
        };
        let (fill, stroke) = (c.fill.unwrap_or("none"), c.stroke.unwrap_or("none"));
        let _ = write!(
            s,
            r#"<rect x="{x}" y="{y}" width="{bw}" height="{bh}" rx="6" fill="{fill}" stroke="{stroke}" stroke-width="{}"{dash}/>"#,
            c.stroke_width
        );
        let style = if kind == NodeKind::Concept {
            r#" font-style="italic""#
        } else if kind == NodeKind::Logic {
            r#" font-family="monospace""#
        } else {
            ""
        };
        let label = format!("{} {}", step.step + 1, step.label);
        let _ = write!(
            s,
            r#"<text x="{}" y="{}" fill="{}" text-anchor="middle" dominant-baseline="middle"{style}>{}</text>"#,
            x + bw / 2.0,
            y + bh / 2.0,
            c.text,
            esc(&fit_label(&label, bw - 12.0))
        );
    }
    s.push_str("</svg>");
    s
}

/// A grid rectangle in SVG units: (x, y, width, height).
fn scaled(r: crate::swimlane::Rect) -> (f64, f64, f64, f64) {
    (
        f64::from(r.x) * UNIT_X,
        f64::from(r.y) * UNIT_Y,
        f64::from(r.w) * UNIT_X,
        f64::from(r.h) * UNIT_Y,
    )
}

//! The map canvas (decision 23, M15 f): the open map as an image through kitty's graphics
//! protocol, beside the outline.
//!
//! The picture comes from a [`MapPainter`] the binary hands the event loop (merman lives in
//! `dovetail-maps`, which this crate must not depend on). It is painted once per map version and
//! terminal size, off the UI thread, and cached: pan and zoom only change which part of it kitty
//! shows ([`view`]), through the placement's source rectangle. The selected node gets a ring
//! drawn as a second, small image over the cached one ([`ring`]), so moving the selection never
//! repaints the map: a ring costs well under a millisecond against 56–184 ms for a merman
//! re-render with a highlight class (S5), and the layout does not move when classes change.
//!
//! [`kitty`] writes the escape sequences; [`detect`] decides whether the terminal can show
//! images at all. Without them the outline is the whole view.

pub mod detect;
pub mod kitty;
pub mod ring;
pub mod view;

use std::fmt;
use std::sync::Arc;

use dovetail_api::visual::map::{Map, MapId, NodeId};

/// A rectangle in picture pixels, from the top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PxRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl PxRect {
    /// Whether the point is inside; the left and top edges count, the right and bottom do not.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// The middle.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Where one node was drawn in the picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The node.
    pub id: NodeId,
    /// Its box.
    pub rect: PxRect,
    /// A group's box encloses its children: a hit prefers anything inside it.
    pub is_group: bool,
    /// On a swimlane map, the flow step this box draws.
    pub step: Option<usize>,
}

/// A painted map: the PNG and where each node landed in it.
#[derive(Clone, PartialEq)]
pub struct Picture {
    /// PNG bytes.
    pub png: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Node boxes, in the map's node order (swimlanes: flow order).
    pub nodes: Vec<Placed>,
}

impl fmt::Debug for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Picture")
            .field("png", &format_args!("{} bytes", self.png.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("nodes", &self.nodes.len())
            .finish()
    }
}

impl Picture {
    /// The node under a picture pixel: the innermost box, a node before the group around it.
    pub fn hit_test(&self, x: f64, y: f64) -> Option<&Placed> {
        self.nodes
            .iter()
            .filter(|n| n.rect.contains(x, y))
            .min_by(|a, b| {
                (a.is_group, a.rect.w * a.rect.h)
                    .partial_cmp(&(b.is_group, b.rect.w * b.rect.h))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// The box to ring for a node: the given flow step's box when there is one, else the node's
    /// first.
    pub fn box_of(&self, id: &NodeId, step: Option<usize>) -> Option<&Placed> {
        step.and_then(|s| self.nodes.iter().find(|n| n.step == Some(s) && &n.id == id))
            .or_else(|| self.nodes.iter().find(|n| &n.id == id))
    }
}

/// A direction for `hjkl` on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `h`.
    Left,
    /// `j`.
    Down,
    /// `k`.
    Up,
    /// `l`.
    Right,
}

/// The node nearest `from` in `dir`, other than `skip`. Nodes within 45° of the direction come
/// first; among them (or among all ahead, when none is) the smallest distance along the
/// direction plus twice the distance across it wins, so a node straight ahead beats a closer
/// one off to the side.
pub fn nearest<'a>(
    nodes: &'a [Placed],
    from: (f64, f64),
    dir: Direction,
    skip: Option<&NodeId>,
) -> Option<&'a Placed> {
    let ahead: Vec<(f64, f64, &Placed)> = nodes
        .iter()
        .filter(|n| Some(&n.id) != skip)
        .filter_map(|n| {
            let (x, y) = n.rect.center();
            let (along, across) = match dir {
                Direction::Left => (from.0 - x, (y - from.1).abs()),
                Direction::Right => (x - from.0, (y - from.1).abs()),
                Direction::Up => (from.1 - y, (x - from.0).abs()),
                Direction::Down => (y - from.1, (x - from.0).abs()),
            };
            (along > 0.5).then_some((along, across, n))
        })
        .collect();
    let best = |cone: bool| {
        ahead
            .iter()
            .filter(|(along, across, _)| !cone || across <= along)
            .min_by(|a, b| {
                (a.0 + 2.0 * a.1)
                    .partial_cmp(&(b.0 + 2.0 * b.1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, _, n)| *n)
    };
    best(true).or_else(|| best(false))
}

/// Why a map could not be painted, as text for the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintError(pub String);

impl fmt::Display for PaintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PaintError {}

/// Paints maps. The binary's implementation wraps `dovetail_maps::MapRenderer`; it runs on the
/// event loop's painter thread, never the UI thread.
pub trait MapPainter: Send + Sync {
    /// Paints `map` to fit inside `fit` pixels (width, height), keeping its aspect ratio.
    fn paint(&self, map: &Map, fit: (u32, u32)) -> Result<Picture, PaintError>;
}

/// Which painting a picture belongs to: the map version and the box it was fitted to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintKey {
    /// The map.
    pub map: MapId,
    /// Its version.
    pub version: u32,
    /// The fit box in pixels.
    pub fit: (u32, u32),
}

/// A painting the app wants done.
#[derive(Debug, Clone)]
pub struct PaintJob {
    /// What it is for.
    pub key: PaintKey,
    /// The map to paint.
    pub map: Arc<Map>,
}

/// A finished painting, back from the painter thread.
#[derive(Debug, Clone)]
pub struct Painted {
    /// What it was for.
    pub key: PaintKey,
    /// The picture, or why there is none.
    pub result: Result<Arc<Picture>, PaintError>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(id: &str, x: f64, y: f64, w: f64, h: f64, is_group: bool) -> Placed {
        Placed {
            id: NodeId(id.into()),
            rect: PxRect { x, y, w, h },
            is_group,
            step: None,
        }
    }

    #[test]
    fn hjkl_picks_the_node_straight_ahead_over_a_closer_one_aside() {
        let nodes = vec![
            placed("here", 100.0, 100.0, 20.0, 20.0, false),
            placed("ahead", 300.0, 100.0, 20.0, 20.0, false),
            placed("aside", 160.0, 200.0, 20.0, 20.0, false),
            placed("near", 150.0, 160.0, 20.0, 20.0, false),
            placed("above", 100.0, 0.0, 20.0, 20.0, false),
        ];
        let from = nodes[0].rect.center();
        let here = Some(&nodes[0].id);
        let go = |d| nearest(&nodes, from, d, here).map(|p| p.id.0.as_str());
        assert_eq!(go(Direction::Right), Some("ahead"));
        // "near" is closer to the right but mostly below: it belongs to `j`.
        assert_eq!(go(Direction::Down), Some("near"));
        assert_eq!(go(Direction::Up), Some("above"));
        assert_eq!(go(Direction::Left), None);
    }

    #[test]
    fn a_hit_prefers_the_node_inside_a_group() {
        let picture = Picture {
            png: Vec::new(),
            width: 200,
            height: 200,
            nodes: vec![
                placed("core", 0.0, 0.0, 200.0, 200.0, true),
                placed("db", 20.0, 20.0, 50.0, 30.0, false),
            ],
        };
        let hit = |x, y| picture.hit_test(x, y).map(|p| p.id.0.as_str());
        assert_eq!(hit(30.0, 30.0), Some("db"));
        assert_eq!(hit(150.0, 150.0), Some("core"));
        assert_eq!(hit(250.0, 10.0), None);
        // Right and bottom edges are outside.
        assert_eq!(hit(70.0, 30.0), Some("core"));
    }
}

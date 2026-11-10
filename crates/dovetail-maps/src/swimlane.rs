//! Swimlane grid layout (decisions 23e, 23g): puts a swimlane map's one flow on a lane × step
//! grid and returns plain geometry, so the canvas can draw and hit-test it without this crate's
//! renderer. Pure and deterministic: the same [`MapContent`] and [`GridParams`] give the same
//! [`SwimlaneLayout`].
//!
//! Steps take one column each, in flow order, so reading order is time order. A flow longer than
//! [`GridParams::max_per_band`] steps wraps into another *band*: a repeat of every lane below
//! (or beside, when lanes are columns) the previous one. Lanes with no steps still get their
//! row. Coordinates are in abstract grid units that the canvas scales to pixels.
//!
//! Lane `node` links to structure-map nodes are only carried along ([`LaneBox::link`]); this
//! module never resolves them.

use dovetail_api::map::{LaneId, MapContent, MapKind, NodeId};

/// Which way lanes run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Lanes are rows stacked top to bottom; the flow runs left to right.
    #[default]
    LanesAsRows,
    /// Lanes are columns side by side; the flow runs top to bottom.
    LanesAsColumns,
}

/// Sizes and wrapping for one layout. [`GridParams::default`] suits a terminal-cell canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridParams {
    /// Lane direction.
    pub orientation: Orientation,
    /// Steps per band before the flow wraps; at least 1.
    pub max_per_band: usize,
    /// A step box's width.
    pub step_w: u32,
    /// A step box's height.
    pub step_h: u32,
    /// Thickness of the lane label strip: its width when lanes are rows, its height when columns.
    pub header: u32,
    /// Space between step boxes, and between the header and the first box; at least 2 so arrows
    /// have room to turn.
    pub gap: u32,
    /// Space between a step box and its lane's edge, across the lane.
    pub pad: u32,
    /// Space between bands; at least 2.
    pub band_gap: u32,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            orientation: Orientation::default(),
            max_per_band: 6,
            step_w: 24,
            step_h: 5,
            header: 14,
            gap: 6,
            pad: 2,
            band_gap: 4,
        }
    }
}

impl GridParams {
    /// Applies the minimums that keep routes well formed.
    fn normalized(mut self) -> Self {
        self.max_per_band = self.max_per_band.max(1);
        self.step_w = self.step_w.max(1);
        self.step_h = self.step_h.max(1);
        self.gap = self.gap.max(2);
        self.band_gap = self.band_gap.max(2);
        self
    }
}

/// A point on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    /// Horizontal, from the left.
    pub x: u32,
    /// Vertical, from the top.
    pub y: u32,
}

/// An axis-aligned rectangle on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// Whether `p` is inside; the left and top edges count, the right and bottom do not.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    /// The middle, rounded down.
    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }
}

/// One lane in one band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneBox {
    /// Index into [`MapContent::lanes`].
    pub lane: usize,
    /// The lane's id.
    pub id: LaneId,
    /// The lane's label, drawn in the header.
    pub label: String,
    /// The structure-map node this lane stands for, kept as given (decision 23e).
    pub link: Option<NodeId>,
    /// Which band; 0 is the first.
    pub band: usize,
    /// The whole lane, header included.
    pub rect: Rect,
    /// The label strip at the start of the lane.
    pub header: Rect,
}

/// One step's box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepBox {
    /// Index into the flow's steps.
    pub step: usize,
    /// The step's node.
    pub node: NodeId,
    /// The node's label.
    pub label: String,
    /// Index into [`MapContent::lanes`].
    pub lane: usize,
    /// Which band; 0 is the first.
    pub band: usize,
    /// Column within the band.
    pub column: usize,
    /// Where it sits.
    pub rect: Rect,
}

/// The arrow from one step to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrow {
    /// Step it leaves.
    pub from: usize,
    /// Step it enters (`from + 1`).
    pub to: usize,
    /// The arrow's label, from the target step.
    pub label: Option<String>,
    /// Corner points from the source box to the target box; every segment is horizontal or
    /// vertical. At least two points.
    pub route: Vec<Point>,
    /// Where to draw the label: the middle of the longest segment.
    pub label_at: Point,
}

/// What a point on the canvas touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A step, by index.
    Step(usize),
    /// An arrow, by index into [`SwimlaneLayout::arrows`] (arrow `i` leaves step `i`).
    Arrow(usize),
    /// A lane, by index into [`MapContent::lanes`], in any band.
    Lane(usize),
}

/// The finished layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwimlaneLayout {
    /// Total width.
    pub width: u32,
    /// Total height.
    pub height: u32,
    /// Number of bands, at least 1.
    pub bands: usize,
    /// Every lane in every band: band by band, lanes in map order.
    pub lanes: Vec<LaneBox>,
    /// One per flow step, in flow order.
    pub steps: Vec<StepBox>,
    /// One per consecutive pair of steps.
    pub arrows: Vec<Arrow>,
}

/// Why a map cannot be laid out. `validate` rules these out; they are errors, not panics, because
/// the content may come from storage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Not a swimlane map.
    #[error("map is a {0} map, not a swimlane map")]
    NotSwimlane(&'static str),
    /// A swimlane map has exactly one flow.
    #[error("a swimlane map has exactly one flow, this one has {0}")]
    FlowCount(usize),
    /// A step has no lane.
    #[error("step {0} has no lane")]
    StepWithoutLane(usize),
    /// A step names a lane the map lacks.
    #[error("step {step} is in lane \"{lane}\", which the map does not have")]
    UnknownLane {
        /// Step index.
        step: usize,
        /// The missing lane's id.
        lane: String,
    },
    /// A step names a node the map lacks.
    #[error("step {step} is on node \"{node}\", which the map does not have")]
    UnknownNode {
        /// Step index.
        step: usize,
        /// The missing node's id.
        node: String,
    },
}

/// Lays out a swimlane map.
pub fn layout(content: &MapContent, params: GridParams) -> Result<SwimlaneLayout, LayoutError> {
    if content.kind != MapKind::Swimlane {
        return Err(LayoutError::NotSwimlane(content.kind.as_str()));
    }
    let [flow] = content.flows.as_slice() else {
        return Err(LayoutError::FlowCount(content.flows.len()));
    };
    let p = params.normalized();

    // Resolve every step first, so errors come before any geometry.
    let mut placed = Vec::with_capacity(flow.steps.len());
    for (i, s) in flow.steps.iter().enumerate() {
        let lane_id = s.lane.as_ref().ok_or(LayoutError::StepWithoutLane(i))?;
        let lane = content
            .lanes
            .iter()
            .position(|l| &l.id == lane_id)
            .ok_or_else(|| LayoutError::UnknownLane {
                step: i,
                lane: lane_id.0.clone(),
            })?;
        let node = content
            .nodes
            .iter()
            .find(|n| n.id == s.node)
            .ok_or_else(|| LayoutError::UnknownNode {
                step: i,
                node: s.node.0.clone(),
            })?;
        placed.push((lane, node));
    }

    // Work on two axes: `a` runs along the flow, `c` across the lanes. `at` turns (a, c) into a
    // point, `rect` an (a, c) box into a rectangle.
    let (step_len, step_cross) = match p.orientation {
        Orientation::LanesAsRows => (p.step_w, p.step_h),
        Orientation::LanesAsColumns => (p.step_h, p.step_w),
    };
    let at = |a: u32, c: u32| match p.orientation {
        Orientation::LanesAsRows => Point { x: a, y: c },
        Orientation::LanesAsColumns => Point { x: c, y: a },
    };
    let rect = |a: u32, c: u32, len: u32, cross: u32| match p.orientation {
        Orientation::LanesAsRows => Rect {
            x: a,
            y: c,
            w: len,
            h: cross,
        },
        Orientation::LanesAsColumns => Rect {
            x: c,
            y: a,
            w: cross,
            h: len,
        },
    };

    let lane_count = content.lanes.len() as u32;
    let lane_thick = step_cross + 2 * p.pad;
    let band_thick = lane_count * lane_thick;
    let band_pitch = band_thick + p.band_gap;
    let pitch = step_len + p.gap;
    let bands = flow.steps.len().div_ceil(p.max_per_band).max(1);
    let columns = flow.steps.len().min(p.max_per_band) as u32;
    let total_a = p.header + p.gap + columns * pitch;
    let total_c = bands as u32 * band_thick + (bands as u32 - 1) * p.band_gap;
    let Point {
        x: width,
        y: height,
    } = at(total_a, total_c);

    let mut lanes = Vec::with_capacity(bands * content.lanes.len());
    for band in 0..bands {
        for (li, l) in content.lanes.iter().enumerate() {
            let c = band as u32 * band_pitch + li as u32 * lane_thick;
            lanes.push(LaneBox {
                lane: li,
                id: l.id.clone(),
                label: l.label.clone(),
                link: l.node.clone(),
                band,
                rect: rect(0, c, total_a, lane_thick),
                header: rect(0, c, p.header, lane_thick),
            });
        }
    }

    // Per step: its box and the (a, c) edges the routes need.
    struct Edges {
        a_start: u32,
        a_end: u32,
        c_start: u32,
        c_end: u32,
        band: usize,
    }
    let mut steps = Vec::with_capacity(placed.len());
    let mut edges = Vec::with_capacity(placed.len());
    for (i, &(lane, node)) in placed.iter().enumerate() {
        let (band, column) = (i / p.max_per_band, i % p.max_per_band);
        let a_start = p.header + p.gap + column as u32 * pitch;
        let c_start = band as u32 * band_pitch + lane as u32 * lane_thick + p.pad;
        steps.push(StepBox {
            step: i,
            node: node.id.clone(),
            label: node.label.clone(),
            lane,
            band,
            column,
            rect: rect(a_start, c_start, step_len, step_cross),
        });
        edges.push(Edges {
            a_start,
            a_end: a_start + step_len,
            c_start,
            c_end: c_start + step_cross,
            band,
        });
    }

    let mut arrows = Vec::new();
    for (i, pair) in edges.windows(2).enumerate() {
        let (f, t) = (&pair[0], &pair[1]);
        let (f_mid_c, t_mid_c) = (f.c_start + step_cross / 2, t.c_start + step_cross / 2);
        let route = if f.band != t.band {
            // Wrap: drop out of the source into the gap below its band, run back to the start of
            // the next band, and enter the target from its start side.
            let gap_c = f.band as u32 * band_pitch + band_thick + p.band_gap / 2;
            let (f_mid_a, t_in_a) = (f.a_start + step_len / 2, t.a_start - p.gap / 2);
            vec![
                at(f_mid_a, f.c_end),
                at(f_mid_a, gap_c),
                at(t_in_a, gap_c),
                at(t_in_a, t_mid_c),
                at(t.a_start, t_mid_c),
            ]
        } else if f_mid_c == t_mid_c {
            vec![at(f.a_end, f_mid_c), at(t.a_start, t_mid_c)]
        } else {
            let turn = f.a_end + p.gap / 2;
            vec![
                at(f.a_end, f_mid_c),
                at(turn, f_mid_c),
                at(turn, t_mid_c),
                at(t.a_start, t_mid_c),
            ]
        };
        let label_at = longest_midpoint(&route);
        arrows.push(Arrow {
            from: i,
            to: i + 1,
            label: flow.steps[i + 1].arrow.clone(),
            route,
            label_at,
        });
    }

    Ok(SwimlaneLayout {
        width,
        height,
        bands,
        lanes,
        steps,
        arrows,
    })
}

/// The middle of the longest segment; the first one on a tie.
fn longest_midpoint(route: &[Point]) -> Point {
    let len = |a: Point, b: Point| a.x.abs_diff(b.x) + a.y.abs_diff(b.y);
    let mut best = (0, route[0], route[0]);
    for w in route.windows(2) {
        let l = len(w[0], w[1]);
        if l > best.0 {
            best = (l, w[0], w[1]);
        }
    }
    Point {
        x: (best.1.x + best.2.x) / 2,
        y: (best.1.y + best.2.y) / 2,
    }
}

impl SwimlaneLayout {
    /// What is at `p`: a step first, then an arrow within `tolerance` units of its line, then a
    /// lane. `None` outside every lane.
    pub fn hit(&self, p: Point, tolerance: u32) -> Option<Hit> {
        if let Some(s) = self.steps.iter().find(|s| s.rect.contains(p)) {
            return Some(Hit::Step(s.step));
        }
        let near = |a: Point, b: Point| {
            p.x + tolerance >= a.x.min(b.x)
                && p.x <= a.x.max(b.x) + tolerance
                && p.y + tolerance >= a.y.min(b.y)
                && p.y <= a.y.max(b.y) + tolerance
        };
        if let Some(i) = self
            .arrows
            .iter()
            .position(|a| a.route.windows(2).any(|w| near(w[0], w[1])))
        {
            return Some(Hit::Arrow(i));
        }
        self.lanes
            .iter()
            .find(|l| l.rect.contains(p))
            .map(|l| Hit::Lane(l.lane))
    }

    /// The step box for flow step `i`.
    pub fn step_rect(&self, i: usize) -> Option<Rect> {
        self.steps.get(i).map(|s| s.rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::map::{Flow, FlowStep, Lane, Node, NodeKind};

    fn node(id: &str) -> Node {
        Node {
            id: NodeId(id.into()),
            kind: NodeKind::Part,
            label: id.to_uppercase(),
            description: String::new(),
            file: None,
            pseudocode: None,
        }
    }

    fn lane(id: &str) -> Lane {
        Lane {
            id: LaneId(id.into()),
            label: id.into(),
            node: None,
        }
    }

    /// Steps are `(lane, arrow)`; step `i` is on node `n{i}`.
    fn map(lanes: &[&str], steps: &[(&str, Option<&str>)]) -> MapContent {
        MapContent {
            title: "t".into(),
            kind: MapKind::Swimlane,
            nodes: (0..steps.len()).map(|i| node(&format!("n{i}"))).collect(),
            groups: vec![],
            connections: vec![],
            flows: vec![Flow {
                name: "f".into(),
                steps: steps
                    .iter()
                    .enumerate()
                    .map(|(i, (l, a))| FlowStep {
                        node: NodeId(format!("n{i}")),
                        connection: None,
                        arrow: a.map(String::from),
                        note: String::new(),
                        lane: Some(LaneId((*l).into())),
                    })
                    .collect(),
            }],
            lanes: lanes.iter().map(|l| lane(l)).collect(),
        }
    }

    #[test]
    fn steps_take_columns_in_flow_order() {
        let m = map(&["a", "b"], &[("a", None), ("b", Some("go")), ("a", None)]);
        let l = layout(&m, GridParams::default()).unwrap();
        assert_eq!(l.bands, 1);
        let cols: Vec<_> = l.steps.iter().map(|s| s.column).collect();
        assert_eq!(cols, [0, 1, 2]);
        assert!(l.steps[0].rect.x < l.steps[1].rect.x && l.steps[1].rect.x < l.steps[2].rect.x);
        // Lane b is below lane a.
        assert!(l.steps[1].rect.y > l.steps[0].rect.y);
        assert_eq!(l.steps[2].rect.y, l.steps[0].rect.y);
        assert_eq!(l.arrows[0].label.as_deref(), Some("go"));
        assert_eq!(l.arrows[1].label, None);
    }

    #[test]
    fn same_lane_arrow_is_straight_and_cross_lane_arrow_turns() {
        let m = map(&["a", "b"], &[("a", None), ("a", None), ("b", None)]);
        let l = layout(&m, GridParams::default()).unwrap();
        assert_eq!(l.arrows[0].route.len(), 2);
        assert_eq!(l.arrows[1].route.len(), 4);
        for a in &l.arrows {
            assert!(
                a.route
                    .windows(2)
                    .all(|w| w[0].x == w[1].x || w[0].y == w[1].y)
            );
        }
    }

    #[test]
    fn long_flow_wraps_into_bands() {
        let steps: Vec<_> = (0..7)
            .map(|i| (if i % 2 == 0 { "a" } else { "b" }, None))
            .collect();
        let m = map(&["a", "b"], &steps);
        let p = GridParams {
            max_per_band: 3,
            ..GridParams::default()
        };
        let l = layout(&m, p).unwrap();
        assert_eq!(l.bands, 3);
        assert_eq!(l.lanes.len(), 6);
        let bc: Vec<_> = l.steps.iter().map(|s| (s.band, s.column)).collect();
        assert_eq!(bc, [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (2, 0)]);
        // The wrap arrow ends at the start of the next band, to the left of where it began.
        let wrap = &l.arrows[2];
        assert!(wrap.route.last().unwrap().x < wrap.route[0].x);
        assert!(l.steps[3].rect.y > l.steps[2].rect.y);
        // Everything fits in the reported size.
        for s in &l.steps {
            assert!(s.rect.x + s.rect.w <= l.width && s.rect.y + s.rect.h <= l.height);
        }
        for a in &l.arrows {
            assert!(a.route.iter().all(|pt| pt.x <= l.width && pt.y <= l.height));
        }
    }

    #[test]
    fn max_flow_in_one_lane() {
        let steps: Vec<_> = (0..crate::limits::MAX_STEPS)
            .map(|_| ("only", None))
            .collect();
        let m = map(&["only"], &steps);
        let l = layout(&m, GridParams::default()).unwrap();
        assert_eq!(l.steps.len(), 60);
        assert_eq!(l.bands, 10);
        assert_eq!(l.arrows.len(), 59);
    }

    #[test]
    fn empty_lane_keeps_its_row() {
        let m = map(&["a", "empty", "b"], &[("a", None), ("b", None)]);
        let l = layout(&m, GridParams::default()).unwrap();
        let empty = l.lanes.iter().find(|x| x.id.0 == "empty").unwrap();
        assert!(empty.rect.h > 0);
        assert!(l.steps[1].rect.y >= l.lanes[2].rect.y);
        assert_eq!(l.hit(empty.rect.center(), 0), Some(Hit::Lane(1)));
    }

    #[test]
    fn lanes_as_columns_transposes() {
        let m = map(&["a", "b"], &[("a", None), ("b", None), ("a", None)]);
        let p = GridParams {
            orientation: Orientation::LanesAsColumns,
            ..GridParams::default()
        };
        let l = layout(&m, p).unwrap();
        assert!(l.steps[0].rect.y < l.steps[1].rect.y && l.steps[1].rect.y < l.steps[2].rect.y);
        assert!(l.steps[1].rect.x > l.steps[0].rect.x);
        assert_eq!(l.steps[0].rect.w, 24);
        assert_eq!(l.steps[0].rect.h, 5);
        assert!(
            l.arrows[0]
                .route
                .windows(2)
                .all(|w| w[0].x == w[1].x || w[0].y == w[1].y)
        );
    }

    #[test]
    fn lane_links_are_kept() {
        let mut m = map(&["a"], &[("a", None)]);
        m.lanes[0].node = Some(NodeId("api".into()));
        let l = layout(&m, GridParams::default()).unwrap();
        assert_eq!(l.lanes[0].link, Some(NodeId("api".into())));
    }

    #[test]
    fn layout_is_deterministic() {
        let m = map(
            &["a", "b"],
            &[("a", None), ("b", Some("x")), ("a", None), ("b", None)],
        );
        let p = GridParams {
            max_per_band: 3,
            ..GridParams::default()
        };
        assert_eq!(layout(&m, p).unwrap(), layout(&m, p).unwrap());
    }

    #[test]
    fn hit_prefers_step_then_arrow_then_lane() {
        let m = map(&["a", "b"], &[("a", None), ("b", None)]);
        let l = layout(&m, GridParams::default()).unwrap();
        assert_eq!(l.hit(l.steps[1].rect.center(), 0), Some(Hit::Step(1)));
        let mid = l.arrows[0].route[1];
        assert_eq!(l.hit(mid, 0), Some(Hit::Arrow(0)));
        // Just beside the arrow's vertical run: only a tolerance catches it.
        let beside = Point {
            x: mid.x + 1,
            y: mid.y,
        };
        assert_eq!(l.hit(beside, 0), Some(Hit::Lane(0)));
        assert_eq!(l.hit(beside, 1), Some(Hit::Arrow(0)));
        // Header and out of bounds.
        assert_eq!(
            l.hit(
                Point {
                    x: 1,
                    y: l.lanes[1].rect.y + 1
                },
                0
            ),
            Some(Hit::Lane(1))
        );
        assert_eq!(
            l.hit(
                Point {
                    x: l.width + 5,
                    y: 0
                },
                0
            ),
            None
        );
        assert_eq!(l.step_rect(9), None);
    }

    #[test]
    fn bad_content_is_an_error() {
        let mut m = map(&["a"], &[("a", None)]);
        m.kind = MapKind::Structure;
        assert_eq!(
            layout(&m, GridParams::default()),
            Err(LayoutError::NotSwimlane("structure"))
        );
        let mut m = map(&["a"], &[("zz", None)]);
        assert!(matches!(
            layout(&m, GridParams::default()),
            Err(LayoutError::UnknownLane { step: 0, .. })
        ));
        m.flows[0].steps[0].lane = None;
        assert_eq!(
            layout(&m, GridParams::default()),
            Err(LayoutError::StepWithoutLane(0))
        );
        m.flows.clear();
        assert_eq!(
            layout(&m, GridParams::default()),
            Err(LayoutError::FlowCount(0))
        );
    }
}

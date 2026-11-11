//! The canvas half of the map view: the cached picture, the view over it, and where it was
//! drawn last. The outline buffer owns one ([`super::MapOutlineBuffer`]); the selection is the
//! outline's, so the two always agree.

use std::sync::Arc;

use dovetail_api::map::{Map, NodeId};
use ratatui::layout::Rect;

use crate::canvas::kitty::Scene;
use crate::canvas::ring::Ring;
use crate::canvas::view::{self, CellPx, PAN_STEP, View, ZOOM_STEP};
use crate::canvas::{Direction, PaintJob, PaintKey, Painted, Picture, Placed, nearest};

/// Canvas state.
#[derive(Default)]
pub struct Canvas {
    /// The cell size when the terminal shows kitty graphics; `None` means outline only.
    cell: Option<CellPx>,
    /// The terminal size in cells, for the paint box.
    screen: (u16, u16),
    picture: Option<Arc<Picture>>,
    /// What [`Canvas::picture`] was painted for.
    key: Option<PaintKey>,
    /// Asked for and not back yet.
    pending: Option<PaintKey>,
    /// A painting for the event loop to start.
    job: Option<PaintJob>,
    /// Why the last painting failed.
    error: Option<String>,
    view: View,
    /// Views to go back to with `Backspace`, one per drill-in.
    drill: Vec<View>,
    /// Where the canvas was drawn in the last frame, in cells.
    area: Option<Rect>,
    /// Where a mouse drag last was, in cells.
    drag: Option<(u16, u16)>,
}

impl Canvas {
    /// Graphics on (with the cell size) or off.
    pub fn set_graphics(&mut self, cell: Option<CellPx>) {
        self.cell = cell;
    }

    /// The terminal's size in cells.
    pub fn set_screen(&mut self, cols: u16, rows: u16) {
        self.screen = (cols, rows);
    }

    /// Whether the terminal shows images.
    pub fn active(&self) -> bool {
        self.cell.is_some()
    }

    /// The picture, once painted.
    pub fn picture(&self) -> Option<&Arc<Picture>> {
        self.picture.as_ref()
    }

    /// Why there is no picture, if painting failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Forgets the picture and view, for another map.
    pub fn reset(&mut self) {
        let (cell, screen) = (self.cell, self.screen);
        *self = Self {
            cell,
            screen,
            ..Self::default()
        };
    }

    /// The canvas is not drawn this frame.
    pub fn hide(&mut self) {
        self.area = None;
    }

    /// Records where the canvas is drawn this frame (`None`: not drawn) and asks for a painting
    /// of `map` if the picture is missing or stale.
    pub fn frame_at(&mut self, map: &Arc<Map>, area: Option<Rect>) {
        self.area = area;
        let Some(cell) = self.cell else {
            return;
        };
        if area.is_none() {
            return;
        }
        let key = PaintKey {
            map: map.id.clone(),
            version: map.version,
            fit: view::fit_box(self.screen, cell),
        };
        // A failed painting keeps its key too, so it is not retried until the version or the
        // terminal size changes.
        if self.key.as_ref() == Some(&key) || self.pending.as_ref() == Some(&key) {
            return;
        }
        self.pending = Some(key.clone());
        self.job = Some(PaintJob {
            key,
            map: map.clone(),
        });
    }

    /// The painting the event loop should start, if any.
    pub fn take_job(&mut self) -> Option<PaintJob> {
        self.job.take()
    }

    /// A painting came back. Returns whether it was the one waited for.
    pub fn receive(&mut self, painted: Painted) -> bool {
        if self.pending.as_ref() != Some(&painted.key) {
            return false;
        }
        self.pending = None;
        let same_map = self.key.as_ref().is_some_and(|k| k.map == painted.key.map);
        if !same_map {
            self.view = View::default();
            self.drill.clear();
        }
        self.key = Some(painted.key);
        match painted.result {
            Ok(picture) => {
                self.picture = Some(picture);
                self.error = None;
            }
            Err(e) => {
                self.picture = None;
                self.error = Some(e.0);
            }
        }
        true
    }

    fn frame(&self) -> Option<view::Frame> {
        let picture = self.picture.as_ref()?;
        Some(view::Frame {
            picture: (picture.width, picture.height),
            area: self.area?,
            cell: self.cell?,
        })
    }

    /// What kitty should show, with a ring around `selected` (the node and the flow step it was
    /// reached by).
    pub fn scene(
        &self,
        selected: Option<(&NodeId, Option<usize>)>,
        ring: [u8; 3],
    ) -> Option<Scene> {
        let frame = self.frame()?;
        let placement = self.view.place(frame)?;
        let picture = self.picture.clone()?;
        let ring = selected
            .and_then(|(id, step)| picture.box_of(id, step))
            .and_then(|b| Ring::around(b.rect, &placement, frame.cell, ring));
        Some(Scene {
            picture,
            placement,
            ring,
        })
    }

    /// The node drawn under a screen cell.
    pub fn node_at(&self, col: u16, row: u16) -> Option<&Placed> {
        let frame = self.frame()?;
        let (x, y) = self
            .view
            .place(frame)?
            .cell_to_picture(frame.cell, col, row)?;
        self.picture.as_ref()?.hit_test(x, y)
    }

    /// Whether a screen cell is on the canvas.
    pub fn contains(&self, col: u16, row: u16) -> bool {
        self.area
            .is_some_and(|a| a.contains(ratatui::layout::Position { x: col, y: row }))
    }

    /// The node `hjkl` moves to from `from` (its box), or the one nearest the middle of the view
    /// when nothing is selected.
    pub fn step(&self, from: Option<&Placed>, dir: Direction) -> Option<&Placed> {
        let picture = self.picture.as_ref()?;
        match from {
            Some(at) => nearest(&picture.nodes, at.rect.center(), dir, Some(&at.id)),
            None => {
                let middle = self.frame().and_then(|f| self.view.visible(f))?.center();
                picture.nodes.iter().min_by(|a, b| {
                    let d = |p: &Placed| {
                        let (x, y) = p.rect.center();
                        (x - middle.0).powi(2) + (y - middle.1).powi(2)
                    };
                    d(a).partial_cmp(&d(b)).unwrap_or(std::cmp::Ordering::Equal)
                })
            }
        }
    }

    /// Keeps a node's box on screen.
    pub fn reveal(&mut self, node: &Placed) {
        if let Some(f) = self.frame() {
            self.view.reveal(node.rect, f);
        }
    }

    /// `+`, `-` and the scroll wheel; `around` is a screen cell to zoom about.
    pub fn zoom(&mut self, zoom_in: bool, around: Option<(u16, u16)>) {
        let Some(f) = self.frame() else {
            return;
        };
        let at = around.and_then(|(c, r)| {
            self.view
                .place(f)
                .and_then(|p| p.cell_to_picture(f.cell, c, r))
        });
        let factor = if zoom_in { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
        self.view.zoom_by(factor, at, f);
    }

    /// `0`: the whole map, and out of every drill-in.
    pub fn fit(&mut self) {
        self.view.fit();
        self.drill.clear();
    }

    /// `Ctrl-hjkl`: pans a step.
    pub fn pan(&mut self, dir: Direction) {
        let Some(f) = self.frame() else {
            return;
        };
        let (dx, dy) = match dir {
            Direction::Left => (-PAN_STEP, 0.0),
            Direction::Right => (PAN_STEP, 0.0),
            Direction::Up => (0.0, -PAN_STEP),
            Direction::Down => (0.0, PAN_STEP),
        };
        self.view.pan(dx, dy, f);
    }

    /// A mouse drag reached `(col, row)`; the first call only starts it.
    pub fn drag_to(&mut self, col: u16, row: u16) {
        let (Some(f), Some((c0, r0))) = (self.frame(), self.drag) else {
            self.drag = Some((col, row));
            return;
        };
        let dx = (f64::from(col) - f64::from(c0)) * f64::from(f.cell.w);
        let dy = (f64::from(row) - f64::from(r0)) * f64::from(f.cell.h);
        self.view.drag(dx, dy, f);
        self.drag = Some((col, row));
    }

    /// The mouse button came up.
    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// `Enter` on a group: zoom to it, remembering the view to come back to.
    pub fn drill_in(&mut self, group: &Placed) {
        if let Some(f) = self.frame() {
            self.drill.push(self.view);
            self.view.show(group.rect, f);
        }
    }

    /// `Backspace`: back to the view before the last drill-in. Returns false at the top.
    pub fn drill_out(&mut self) -> bool {
        match self.drill.pop() {
            Some(view) => {
                self.view = view;
                true
            }
            None => false,
        }
    }

    /// How far drilled in.
    pub fn depth(&self) -> usize {
        self.drill.len()
    }

    /// The zoom, for the title.
    pub fn zoom_level(&self) -> f64 {
        self.view.zoom()
    }
}

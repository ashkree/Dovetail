//! The selection ring: a small RGBA image of a frame around the selected node, placed over the
//! cached map picture (see the module docs of [`super`] for why not a re-render).

use super::PxRect;
use super::view::{CellPx, Placement};

/// Space between the node's box and the ring, in screen pixels.
const GAP: f64 = 3.0;
/// The ring's thickness in screen pixels.
const THICKNESS: i64 = 3;

/// A ring ready to send: its pixels' size and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ring {
    /// Screen column of the cell its top left corner is in.
    pub col: u16,
    /// Screen row of that cell.
    pub row: u16,
    /// Pixel offset inside that cell, across.
    pub x_off: u32,
    /// Pixel offset inside that cell, down.
    pub y_off: u32,
    /// Image width in pixels: the part of the ring inside the canvas.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// The whole ring's outer box relative to the image's top left; it can stick out of the
    /// image where the canvas edge cuts it.
    outer: (i64, i64, i64, i64),
    /// RGBA colour.
    color: [u8; 4],
}

impl Ring {
    /// The ring around `node` (a picture rectangle) under `placement`, cut to the placement's
    /// area. `None` when none of it is on screen.
    pub fn around(
        node: PxRect,
        placement: &Placement,
        cell: CellPx,
        color: [u8; 3],
    ) -> Option<Self> {
        let s = placement.to_screen(cell, node);
        let (sw, sh) = placement.screen_size(cell);
        let outer = (
            (s.x - GAP - THICKNESS as f64).floor() as i64,
            (s.y - GAP - THICKNESS as f64).floor() as i64,
            (s.x + s.w + GAP + THICKNESS as f64).ceil() as i64,
            (s.y + s.h + GAP + THICKNESS as f64).ceil() as i64,
        );
        let x0 = outer.0.max(0);
        let y0 = outer.1.max(0);
        let x1 = outer.2.min(sw as i64);
        let y1 = outer.3.min(sh as i64);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (cw, ch) = (i64::from(cell.w), i64::from(cell.h));
        Some(Self {
            col: placement.col + (x0 / cw) as u16,
            row: placement.row + (y0 / ch) as u16,
            x_off: (x0 % cw) as u32,
            y_off: (y0 % ch) as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
            outer: (outer.0 - x0, outer.1 - y0, outer.2 - x0, outer.3 - y0),
            color: [color[0], color[1], color[2], 255],
        })
    }

    /// Whether the image pixel (x, y) is on the ring.
    fn on_ring(&self, x: i64, y: i64) -> bool {
        let (l, t, r, b) = self.outer;
        let inside_outer = x >= l && x < r && y >= t && y < b;
        let inside_inner =
            x >= l + THICKNESS && x < r - THICKNESS && y >= t + THICKNESS && y < b - THICKNESS;
        inside_outer && !inside_inner
    }

    /// The image: ring pixels in the colour, the rest transparent.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for y in 0..i64::from(self.height) {
            for x in 0..i64::from(self.width) {
                out.extend_from_slice(if self.on_ring(x, y) {
                    &self.color
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: CellPx = CellPx { w: 10, h: 20 };

    /// The picture at one screen pixel per picture pixel, 100×100 at cell (5, 2).
    fn placement() -> Placement {
        Placement {
            src_x: 0,
            src_y: 0,
            src_w: 100,
            src_h: 100,
            col: 5,
            row: 2,
            cols: 10,
            rows: 5,
        }
    }

    #[test]
    fn the_ring_frames_the_node_with_a_gap() {
        let node = PxRect {
            x: 30.0,
            y: 40.0,
            w: 20.0,
            h: 10.0,
        };
        let ring = Ring::around(node, &placement(), CELL, [255, 0, 0]).expect("on screen");
        // Outer box from (24, 34) to (56, 56): cell (2, 1) of the placement, 4 px in.
        assert_eq!((ring.col, ring.row, ring.x_off, ring.y_off), (7, 3, 4, 14));
        assert_eq!((ring.width, ring.height), (32, 22));
        let rgba = ring.rgba();
        assert_eq!(rgba.len(), 32 * 22 * 4);
        let alpha = |x: usize, y: usize| rgba[(y * 32 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 255, "corner is ring");
        assert_eq!(alpha(16, 11), 0, "the node shows through");
        assert_eq!(alpha(4, 11), 0, "the gap is clear");
    }

    #[test]
    fn a_ring_cut_by_the_canvas_edge_keeps_only_the_visible_part() {
        let node = PxRect {
            x: 90.0,
            y: 10.0,
            w: 40.0,
            h: 10.0,
        };
        let ring = Ring::around(node, &placement(), CELL, [255, 0, 0]).expect("partly on screen");
        assert_eq!(ring.width, 100 - 84);
        let off = PxRect {
            x: 200.0,
            y: 10.0,
            w: 10.0,
            h: 10.0,
        };
        assert_eq!(Ring::around(off, &placement(), CELL, [255, 0, 0]), None);
    }
}

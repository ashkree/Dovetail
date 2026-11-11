//! Pan and zoom as a crop of the cached picture.
//!
//! At zoom 1 the whole picture fits the canvas area. Zooming in shows a smaller part of it
//! (kitty's source rectangle) in the same cells, so nothing is repainted. The column and row
//! counts are worked out first and the source rectangle back from them, so kitty never
//! stretches the picture out of shape when a size rounds to whole cells.

use ratatui::layout::Rect;

use super::PxRect;

/// Most zoom past "fit". The picture is painted at about twice the screen's size (see
/// [`fit_box`]), so this is as far in as it stays reasonably sharp.
pub const MAX_ZOOM: f64 = 8.0;
/// One zoom step (`+`, `-`, one scroll-wheel notch).
pub const ZOOM_STEP: f64 = 1.25;
/// One keyboard pan step, as a share of what is visible.
pub const PAN_STEP: f64 = 0.25;
/// How much room a drilled-into group gets around it, as a share of its size on each side.
const DRILL_MARGIN: f64 = 0.1;
/// merman's raster size limit (S5); the swimlane renderer keeps the same.
pub const MAX_SIDE: u32 = 4096;
/// The picture is painted at this many times the screen's pixel size, for zoom headroom.
pub const OVERSAMPLE: u32 = 2;

/// One terminal cell in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPx {
    /// Width.
    pub w: u16,
    /// Height.
    pub h: u16,
}

/// The box a map is painted to fit: the whole screen in pixels, times [`OVERSAMPLE`], capped at
/// [`MAX_SIDE`]. It depends on the screen only, so entering MAP mode never repaints.
pub fn fit_box(screen: (u16, u16), cell: CellPx) -> (u32, u32) {
    let side =
        |cells: u16, px: u16| (u32::from(cells) * u32::from(px) * OVERSAMPLE).clamp(1, MAX_SIDE);
    (side(screen.0, cell.w), side(screen.1, cell.h))
}

/// Where kitty draws the picture: which part of it, into which cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// Source rectangle in picture pixels.
    pub src_x: u32,
    /// Top of the source rectangle.
    pub src_y: u32,
    /// Source width.
    pub src_w: u32,
    /// Source height.
    pub src_h: u32,
    /// Screen column of the top left cell.
    pub col: u16,
    /// Screen row of the top left cell.
    pub row: u16,
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
}

impl Placement {
    /// Screen pixels per picture pixel, across.
    fn scale_x(&self, cell: CellPx) -> f64 {
        f64::from(self.cols) * f64::from(cell.w) / f64::from(self.src_w)
    }

    /// Screen pixels per picture pixel, down.
    fn scale_y(&self, cell: CellPx) -> f64 {
        f64::from(self.rows) * f64::from(cell.h) / f64::from(self.src_h)
    }

    /// The picture point at the middle of a screen cell, if the cell shows the picture.
    pub fn cell_to_picture(&self, cell: CellPx, col: u16, row: u16) -> Option<(f64, f64)> {
        if col < self.col
            || row < self.row
            || col >= self.col + self.cols
            || row >= self.row + self.rows
        {
            return None;
        }
        let px = (f64::from(col - self.col) + 0.5) * f64::from(cell.w);
        let py = (f64::from(row - self.row) + 0.5) * f64::from(cell.h);
        Some((
            f64::from(self.src_x) + px / self.scale_x(cell),
            f64::from(self.src_y) + py / self.scale_y(cell),
        ))
    }

    /// A picture rectangle in screen pixels from the placement's top left corner.
    pub fn to_screen(&self, cell: CellPx, r: PxRect) -> PxRect {
        let (sx, sy) = (self.scale_x(cell), self.scale_y(cell));
        PxRect {
            x: (r.x - f64::from(self.src_x)) * sx,
            y: (r.y - f64::from(self.src_y)) * sy,
            w: r.w * sx,
            h: r.h * sy,
        }
    }

    /// The placement's own size in screen pixels.
    pub fn screen_size(&self, cell: CellPx) -> (f64, f64) {
        (
            f64::from(self.cols) * f64::from(cell.w),
            f64::from(self.rows) * f64::from(cell.h),
        )
    }
}

/// Zoom and the picture point at the middle of the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    zoom: f64,
    /// `None` is the picture's middle.
    center: Option<(f64, f64)>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            center: None,
        }
    }
}

/// What a view is laid over: the picture's size, the canvas area and the cell size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Picture width and height in pixels.
    pub picture: (u32, u32),
    /// The canvas area in cells.
    pub area: Rect,
    /// One cell in pixels.
    pub cell: CellPx,
}

impl Frame {
    fn area_px(&self) -> (f64, f64) {
        (
            f64::from(self.area.width) * f64::from(self.cell.w),
            f64::from(self.area.height) * f64::from(self.cell.h),
        )
    }

    fn picture_f(&self) -> (f64, f64) {
        (f64::from(self.picture.0), f64::from(self.picture.1))
    }

    /// Screen pixels per picture pixel at zoom 1: the whole picture fits.
    fn fit_scale(&self) -> f64 {
        let (aw, ah) = self.area_px();
        let (pw, ph) = self.picture_f();
        (aw / pw).min(ah / ph)
    }

    fn usable(&self) -> bool {
        self.picture.0 > 0
            && self.picture.1 > 0
            && self.area.width > 0
            && self.area.height > 0
            && self.cell.w > 0
            && self.cell.h > 0
    }
}

impl View {
    /// The current zoom; 1 is "fit".
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// Back to the whole picture.
    pub fn fit(&mut self) {
        *self = Self::default();
    }

    /// Where kitty draws the picture for this view, or `None` when nothing can be drawn.
    pub fn place(&self, f: Frame) -> Option<Placement> {
        if !f.usable() {
            return None;
        }
        let (aw, ah) = f.area_px();
        let (pw, ph) = f.picture_f();
        let s = f.fit_scale() * self.zoom;
        let (cw, ch) = (f64::from(f.cell.w), f64::from(f.cell.h));
        // Cells first, then the source back from them, so the aspect ratio holds.
        let cols = ((pw.min(aw / s) * s / cw).round() as u16).clamp(1, f.area.width);
        let rows = ((ph.min(ah / s) * s / ch).round() as u16).clamp(1, f.area.height);
        let vw = (f64::from(cols) * cw / s).min(pw);
        let vh = (f64::from(rows) * ch / s).min(ph);
        let (cx, cy) = self.center.unwrap_or((pw / 2.0, ph / 2.0));
        let sx = (cx - vw / 2.0).clamp(0.0, pw - vw);
        let sy = (cy - vh / 2.0).clamp(0.0, ph - vh);
        let src_w = (vw.round() as u32).clamp(1, f.picture.0);
        let src_h = (vh.round() as u32).clamp(1, f.picture.1);
        Some(Placement {
            src_x: (sx.round() as u32).min(f.picture.0 - src_w),
            src_y: (sy.round() as u32).min(f.picture.1 - src_h),
            src_w,
            src_h,
            col: f.area.x + (f.area.width - cols) / 2,
            row: f.area.y + (f.area.height - rows) / 2,
            cols,
            rows,
        })
    }

    /// The part of the picture on screen, in picture pixels.
    pub fn visible(&self, f: Frame) -> Option<PxRect> {
        self.place(f).map(|p| PxRect {
            x: f64::from(p.src_x),
            y: f64::from(p.src_y),
            w: f64::from(p.src_w),
            h: f64::from(p.src_h),
        })
    }

    /// Keeps the middle where the placement actually is, so panning past an edge does not build
    /// up a distance that has to be panned back first.
    fn settle(&mut self, f: Frame) {
        if let Some(v) = self.visible(f) {
            self.center = Some(v.center());
        }
    }

    /// Zooms by `factor`, keeping the picture point `around` (default: the middle) where it is
    /// on screen.
    pub fn zoom_by(&mut self, factor: f64, around: Option<(f64, f64)>, f: Frame) {
        if !f.usable() {
            return;
        }
        let (pw, ph) = f.picture_f();
        let old = self.zoom;
        self.zoom = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        let (cx, cy) = self.center.unwrap_or((pw / 2.0, ph / 2.0));
        let (ax, ay) = around.unwrap_or((cx, cy));
        let k = old / self.zoom;
        self.center = Some((ax + (cx - ax) * k, ay + (cy - ay) * k));
        self.settle(f);
    }

    /// Pans by a share of what is visible: `dx` across, `dy` down; negative goes left or up.
    pub fn pan(&mut self, dx: f64, dy: f64, f: Frame) {
        let Some(v) = self.visible(f) else {
            return;
        };
        let (cx, cy) = v.center();
        self.center = Some((cx + dx * v.w, cy + dy * v.h));
        self.settle(f);
    }

    /// Pans by screen pixels (a mouse drag): the picture follows the pointer.
    pub fn drag(&mut self, dx_px: f64, dy_px: f64, f: Frame) {
        let Some(p) = self.place(f) else {
            return;
        };
        let (sw, sh) = p.screen_size(f.cell);
        self.pan(-dx_px / sw, -dy_px / sh, f);
    }

    /// Zooms so `r` fills the canvas with a little room around it (drilling into a group).
    pub fn show(&mut self, r: PxRect, f: Frame) {
        if !f.usable() || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (aw, ah) = f.area_px();
        let pad = 1.0 + 2.0 * DRILL_MARGIN;
        let wanted = (aw / (r.w * pad)).min(ah / (r.h * pad));
        self.zoom = (wanted / f.fit_scale()).clamp(1.0, MAX_ZOOM);
        self.center = Some(r.center());
        self.settle(f);
    }

    /// Recentres on `r` when its middle is off screen; otherwise leaves the view alone.
    pub fn reveal(&mut self, r: PxRect, f: Frame) {
        let Some(v) = self.visible(f) else {
            return;
        };
        let (x, y) = r.center();
        if !v.contains(x, y) {
            self.center = Some((x, y));
            self.settle(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: CellPx = CellPx { w: 10, h: 20 };

    /// A 1000×500 picture on an 80×25 canvas (800×500 px) at the screen's top left.
    fn frame() -> Frame {
        Frame {
            picture: (1000, 500),
            area: Rect::new(0, 0, 80, 25),
            cell: CELL,
        }
    }

    #[test]
    fn fit_shows_the_whole_picture_centred_without_stretching() {
        let p = View::default().place(frame()).expect("drawable");
        assert_eq!((p.src_x, p.src_y, p.src_w, p.src_h), (0, 0, 1000, 500));
        // 0.8 px per picture px: 800×400 px, so 80 columns and 20 rows, 2 rows down.
        assert_eq!((p.col, p.row, p.cols, p.rows), (0, 2, 80, 20));
        assert!((p.scale_x(CELL) - p.scale_y(CELL)).abs() < 1e-9);
    }

    #[test]
    fn zooming_in_crops_around_the_middle() {
        let mut v = View::default();
        v.zoom_by(2.0, None, frame());
        let p = v.place(frame()).expect("drawable");
        assert_eq!((p.cols, p.rows), (80, 25));
        // 1.6 px per picture px: 500×312.5 visible, centred on (500, 250).
        assert_eq!((p.src_w, p.src_h), (500, 313));
        assert_eq!((p.src_x, p.src_y), (250, 94));
    }

    #[test]
    fn zoom_around_a_point_keeps_that_point_under_the_pointer() {
        let f = frame();
        let mut v = View::default();
        // Zoomed in already, so neither edge clamps the move.
        v.zoom_by(2.0, None, f);
        let before = v.place(f).expect("drawable");
        let (col, row) = (30, 10);
        let at = before
            .cell_to_picture(CELL, col, row)
            .expect("on the picture");
        v.zoom_by(ZOOM_STEP, Some(at), f);
        let after = v.place(f).expect("drawable");
        let moved = after
            .cell_to_picture(CELL, col, row)
            .expect("on the picture");
        // Within a cell's worth of picture pixels.
        assert!((moved.0 - at.0).abs() < 10.0, "{at:?} -> {moved:?}");
        assert!((moved.1 - at.1).abs() < 20.0, "{at:?} -> {moved:?}");
    }

    #[test]
    fn zoom_stays_between_fit_and_the_maximum() {
        let mut v = View::default();
        v.zoom_by(1.0 / ZOOM_STEP, None, frame());
        assert_eq!(v.zoom(), 1.0);
        for _ in 0..40 {
            v.zoom_by(ZOOM_STEP, None, frame());
        }
        assert_eq!(v.zoom(), MAX_ZOOM);
    }

    #[test]
    fn panning_stops_at_the_edge_and_comes_straight_back() {
        let f = frame();
        let mut v = View::default();
        v.zoom_by(2.0, None, f);
        for _ in 0..10 {
            v.pan(-PAN_STEP, 0.0, f);
        }
        assert_eq!(v.place(f).map(|p| p.src_x), Some(0));
        // One step right moves at once: nothing built up past the edge.
        v.pan(PAN_STEP, 0.0, f);
        assert_eq!(v.place(f).map(|p| p.src_x), Some(125));
    }

    #[test]
    fn panning_at_fit_does_nothing() {
        let f = frame();
        let mut v = View::default();
        v.pan(PAN_STEP, PAN_STEP, f);
        assert_eq!(v.place(f), View::default().place(f));
    }

    #[test]
    fn a_cell_maps_to_the_picture_point_under_its_middle() {
        let p = View::default().place(frame()).expect("drawable");
        // Rows 0 and 1 are the letterbox above the picture.
        assert_eq!(p.cell_to_picture(CELL, 0, 0), None);
        assert_eq!(p.cell_to_picture(CELL, 80, 10), None);
        // Cell (0, 2): its middle is (5, 10) px into the placement, 0.8 px per picture px.
        assert_eq!(p.cell_to_picture(CELL, 0, 2), Some((6.25, 12.5)));
        assert_eq!(p.cell_to_picture(CELL, 79, 21), Some((993.75, 487.5)));
    }

    #[test]
    fn drilling_in_fills_the_canvas_with_the_group() {
        let f = frame();
        let mut v = View::default();
        let group = PxRect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        v.show(group, f);
        let seen = v.visible(f).expect("drawable");
        assert!(
            seen.contains(100.0, 100.0) && seen.contains(299.0, 199.0),
            "{seen:?}"
        );
        assert!(seen.w < 300.0, "zoomed in: {seen:?}");
    }

    #[test]
    fn revealing_an_off_screen_node_recentres_on_it() {
        let f = frame();
        let mut v = View::default();
        v.zoom_by(4.0, None, f);
        let far = PxRect {
            x: 900.0,
            y: 450.0,
            w: 40.0,
            h: 20.0,
        };
        v.reveal(far, f);
        let seen = v.visible(f).expect("drawable");
        assert!(seen.contains(920.0, 460.0), "{seen:?}");
        // An on-screen node leaves the view where it is.
        let before = v;
        v.reveal(
            PxRect {
                x: seen.x + 1.0,
                y: seen.y + 1.0,
                w: 2.0,
                h: 2.0,
            },
            f,
        );
        assert_eq!(v, before);
    }

    #[test]
    fn the_paint_box_is_twice_the_screen_and_capped() {
        assert_eq!(fit_box((200, 50), CELL), (4000, 2000));
        assert_eq!(fit_box((400, 100), CELL), (MAX_SIDE, 4000));
    }
}

//! The reading column: at most 120 columns wide, centred in a wider pane.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Default reading width in columns (configurable later through `config.toml`).
pub const DEFAULT_WIDTH: u16 = 120;

/// The column inside `area`: `max_width` wide at most, centred when the pane is wider.
pub fn column(area: Rect, max_width: u16) -> Rect {
    let width = area.width.min(max_width.max(1));
    Rect {
        x: area.x + (area.width - width) / 2,
        width,
        ..area
    }
}

/// Paints `background` behind the column (the solid background of reading views). Text already
/// drawn keeps its colour; only cells' backgrounds change.
pub fn paint_background(buf: &mut Buffer, column: Rect, background: Style) {
    buf.set_style(column, background);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_pane_uses_all_of_it() {
        let area = Rect::new(2, 1, 80, 20);
        assert_eq!(column(area, 120), area);
    }

    #[test]
    fn a_wide_pane_centres_a_120_column_column() {
        let area = Rect::new(0, 0, 200, 20);
        let c = column(area, 120);
        assert_eq!((c.x, c.width), (40, 120));
        let area = Rect::new(10, 0, 201, 20);
        assert_eq!(column(area, 120).x, 10 + 40);
    }

    #[test]
    fn the_column_keeps_the_panes_rows() {
        let c = column(Rect::new(0, 3, 200, 17), 120);
        assert_eq!((c.y, c.height), (3, 17));
    }
}

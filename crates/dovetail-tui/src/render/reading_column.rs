//! The reading column: at most 120 columns wide, centred in a wider pane. Reading views (reading
//! mode, a research document in the task detail) use [`reading_text`]: 96 columns of text,
//! padded, centred, with no background (ADR 0018, decided 2026-11-11 later).

use ratatui::layout::Rect;

use crate::theme::READING_PADDING;

/// Default reading width in columns (configurable later through `config.toml`).
pub const DEFAULT_WIDTH: u16 = 120;

/// Widest reading mode's text gets, in columns.
pub const READING_TEXT_WIDTH: u16 = 96;

/// Where reading mode's text goes inside `area`: a column [`READING_TEXT_WIDTH`] wide plus
/// [`READING_PADDING`] on each side, centred, less that padding. A narrow pane keeps the
/// padding and gives up text width.
pub fn reading_text(area: Rect) -> Rect {
    let column = column(area, READING_TEXT_WIDTH + 2 * READING_PADDING.x);
    READING_PADDING.shrink(column)
}

/// The column inside `area`: `max_width` wide at most, centred when the pane is wider.
pub fn column(area: Rect, max_width: u16) -> Rect {
    let width = area.width.min(max_width.max(1));
    Rect {
        x: area.x + (area.width - width) / 2,
        width,
        ..area
    }
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
    fn reading_text_is_96_columns_padded_and_centred() {
        // 102 = 96 + 3 a side, centred in 200: starts 49 in; the text starts 3 further.
        let t = reading_text(Rect::new(0, 0, 200, 20));
        assert_eq!((t.x, t.width), (52, 96));
        assert_eq!((t.y, t.height), (1, 18));
        let t = reading_text(Rect::new(10, 2, 201, 20));
        assert_eq!((t.x, t.width), (10 + 49 + 3, 96));
    }

    #[test]
    fn a_narrow_pane_keeps_the_reading_padding() {
        let t = reading_text(Rect::new(4, 0, 60, 10));
        assert_eq!((t.x, t.width, t.y, t.height), (7, 54, 1, 8));
    }

    #[test]
    fn the_column_keeps_the_panes_rows() {
        let c = column(Rect::new(0, 3, 200, 17), 120);
        assert_eq!((c.y, c.height), (3, 17));
    }
}

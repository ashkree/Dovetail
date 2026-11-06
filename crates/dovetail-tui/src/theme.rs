//! The one struct holding every colour (Decision 16). Dusk is the default (plan A19).

use ratatui::style::{Color, Modifier, Style};

/// All colours the UI uses. Backgrounds stay unset on transparent terminals; only selections and
/// badges paint one.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Body text: off-white, never pure white.
    pub body: Color,
    /// Bold text, speaker labels and headings: the warm accent.
    pub bold: Color,
    /// Italic text.
    pub italic: Color,
    /// Inline code and file paths.
    pub code: Color,
    /// Tool summaries, hints, older turns.
    pub dim: Color,
    /// Working agents.
    pub working: Color,
    /// Needs you, warnings, permission-mode badges.
    pub needs_you: Color,
    /// Done and success.
    pub ok: Color,
    /// Errors.
    pub error: Color,
    /// Border of the focused region.
    pub focus: Color,
    /// Border of other regions.
    pub line: Color,
    /// Selected row background.
    pub selection: Color,
    /// Text on a coloured badge.
    pub ink: Color,
    /// Solid background of the reading column, when switched on.
    pub reading_bg: Color,
    /// Status-bar mode badge in Normal mode.
    pub mode_normal: Color,
    /// Status-bar mode badge in Insert mode.
    pub mode_insert: Color,
    /// Status-bar mode badge in Command mode.
    pub mode_command: Color,
}

impl Theme {
    /// Dusk: cool slate with a sand accent.
    pub fn dusk() -> Self {
        Self {
            body: Color::Rgb(0xd4, 0xd9, 0xdf),
            bold: Color::Rgb(0xe5, 0xb5, 0x67),
            italic: Color::Rgb(0xc0, 0xa5, 0xe0),
            code: Color::Rgb(0x79, 0xcd, 0xbd),
            dim: Color::Rgb(0x80, 0x8a, 0x96),
            working: Color::Rgb(0x6c, 0xb6, 0xff),
            needs_you: Color::Rgb(0xf0, 0x88, 0x3e),
            ok: Color::Rgb(0x7e, 0xe7, 0x87),
            error: Color::Rgb(0xff, 0x7b, 0x72),
            focus: Color::Rgb(0x6c, 0xb6, 0xff),
            line: Color::Rgb(0x4a, 0x52, 0x5e),
            selection: Color::Rgb(0x2a, 0x32, 0x40),
            ink: Color::Rgb(0x15, 0x17, 0x1c),
            reading_bg: Color::Rgb(0x1b, 0x20, 0x28),
            mode_normal: Color::Rgb(0xe5, 0xb5, 0x67),
            mode_insert: Color::Rgb(0x7e, 0xe7, 0x87),
            mode_command: Color::Rgb(0xc0, 0xa5, 0xe0),
        }
    }

    /// Plain body text.
    pub fn body(&self) -> Style {
        Style::new().fg(self.body)
    }

    /// Dimmed text.
    pub fn dim(&self) -> Style {
        Style::new().fg(self.dim)
    }

    /// Bold in the accent colour.
    pub fn accent(&self) -> Style {
        Style::new().fg(self.bold).add_modifier(Modifier::BOLD)
    }

    /// Foreground only.
    pub fn fg(&self, color: Color) -> Style {
        Style::new().fg(color)
    }

    /// Text on a coloured badge.
    pub fn badge(&self, background: Color) -> Style {
        Style::new().fg(self.ink).bg(background)
    }

    /// Context fill: plain up to 60 %, amber to 80 %, red above (Decision 22).
    pub fn context(&self, pct: u8) -> Style {
        match pct {
            0..=60 => self.dim(),
            61..=80 => Style::new().fg(self.needs_you),
            _ => Style::new().fg(self.error),
        }
    }

    /// The selected row.
    pub fn selected(&self) -> Style {
        Style::new().bg(self.selection)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dusk()
    }
}

/// Extra space kept free inside the conversation pane, on top of the pane's own border and
/// one-column inset. The one place to tune it.
pub struct Padding {
    /// Columns free on each of the left and right sides.
    pub x: u16,
    /// Rows free at the top and at the bottom.
    pub y: u16,
}

impl Padding {
    /// `area` less this padding on every side (never below zero size).
    pub fn shrink(&self, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let dx = self.x.min(area.width / 2);
        let dy = self.y.min(area.height / 2);
        ratatui::layout::Rect {
            x: area.x + dx,
            y: area.y + dy,
            width: area.width - 2 * dx,
            height: area.height - 2 * dy,
        }
    }
}

/// Padding inside the coordinator chat pane.
pub const CHAT_PADDING: Padding = Padding { x: 2, y: 1 };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_fill_is_plain_amber_then_red() {
        let t = Theme::dusk();
        assert_eq!(t.context(0).fg, Some(t.dim));
        assert_eq!(t.context(60).fg, Some(t.dim));
        assert_eq!(t.context(61).fg, Some(t.needs_you));
        assert_eq!(t.context(80).fg, Some(t.needs_you));
        assert_eq!(t.context(81).fg, Some(t.error));
        assert_eq!(t.context(100).fg, Some(t.error));
    }
}

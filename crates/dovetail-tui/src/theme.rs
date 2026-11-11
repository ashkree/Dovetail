//! The one struct holding every colour (Decision 16). Purple is the default (task #13); Dusk
//! stays as the cool-slate alternative.

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
    /// Status-bar mode badge in Normal mode.
    pub mode_normal: Color,
    /// Status-bar mode badge in Insert mode.
    pub mode_insert: Color,
    /// Status-bar mode badge in Command mode.
    pub mode_command: Color,
    /// Status-bar mode badge in MAP mode.
    pub mode_map: Color,
    /// The ring around the selected node on the map canvas: the yellow of the Mermaid
    /// `<kind>Sel` classes, so both renderers' selections look alike.
    pub map_ring: Color,
    /// Citation markers (`[3]`, `[F3]`, `[S2]`) in research text.
    pub cite: Color,
    /// Image attachment chips (`[image 1 · 412 KB]`), foreground only.
    pub chip: Color,
    /// Your name in a message header rule.
    pub speaker_you: Color,
    /// An agent's name in a message header rule.
    pub speaker_agent: Color,
    /// The rule and time of your message headers: your name colour, dimmed.
    pub speaker_you_rule: Color,
    /// The rule and time of an agent's message headers: its name colour, dimmed.
    pub speaker_agent_rule: Color,
}

impl Theme {
    /// Purple: the AGS desktop palette (`~/.config/theme/theme.conf`) on a dark translucent
    /// terminal. Text colours are the AGS ones lifted for contrast: every foreground is at least
    /// 6:1 against the kitty glass `#171b23`; `dim` (chrome only) is 5.4:1.
    pub fn purple() -> Self {
        Self {
            // text-bright #f0eaff softened ~13 % toward the glass: 11.7:1, never pure white.
            body: Color::Rgb(0xd6, 0xd2, 0xe8),
            // warning #f5c069, softened.
            bold: Color::Rgb(0xe0, 0xb8, 0x78),
            // accent-muted #a48fd4, lifted.
            italic: Color::Rgb(0xb9, 0xa4, 0xe8),
            // success #6edbb0, softened.
            code: Color::Rgb(0x7f, 0xd6, 0xc0),
            // text-secondary #9293aa, lifted a little.
            dim: Color::Rgb(0x8d, 0x8e, 0xa6),
            // info #70b8f0.
            working: Color::Rgb(0x7c, 0xc0, 0xf2),
            needs_you: Color::Rgb(0xf0, 0xb8, 0x62),
            ok: Color::Rgb(0x7f, 0xdc, 0xb8),
            // danger #c87060, lifted from 4.6:1.
            error: Color::Rgb(0xe0, 0x80, 0x6e),
            // accent-muted #a48fd4.
            focus: Color::Rgb(0xa4, 0x8f, 0xd4),
            // crust-dark #4f5066.
            line: Color::Rgb(0x4f, 0x50, 0x66),
            // mantle-light #2c2e3e.
            selection: Color::Rgb(0x2c, 0x2e, 0x3e),
            // Between core-void and core-deep; at least 8:1 on every badge colour.
            ink: Color::Rgb(0x13, 0x14, 0x1a),
            mode_normal: Color::Rgb(0xe0, 0xb8, 0x78),
            mode_insert: Color::Rgb(0x7f, 0xdc, 0xb8),
            mode_command: Color::Rgb(0xb9, 0xa4, 0xe8),
            mode_map: Color::Rgb(0x7f, 0xd6, 0xc0),
            map_ring: Color::Rgb(0xff, 0xd8, 0x4a),
            cite: Color::Rgb(0xe4, 0x9c, 0xc4),
            chip: Color::Rgb(0x9f, 0xb8, 0xe0),
            speaker_you: Color::Rgb(0xb9, 0xa4, 0xe8),
            speaker_agent: Color::Rgb(0xe0, 0xb8, 0x78),
            // The name colours at ~55 %, over the glass.
            speaker_you_rule: Color::Rgb(0x7c, 0x6a, 0xb8),
            speaker_agent_rule: Color::Rgb(0x8a, 0x6d, 0x40),
        }
    }

    /// Dusk: cool slate with a sand accent.
    pub fn dusk() -> Self {
        Self {
            body: Color::Rgb(0xb8, 0xbd, 0xc2),
            bold: Color::Rgb(0xc7, 0x9d, 0x5a),
            italic: Color::Rgb(0xa7, 0x90, 0xc3),
            code: Color::Rgb(0x69, 0xb2, 0xa4),
            dim: Color::Rgb(0x80, 0x8a, 0x96),
            working: Color::Rgb(0x64, 0xa9, 0xed),
            needs_you: Color::Rgb(0xdf, 0x7e, 0x3a),
            ok: Color::Rgb(0x75, 0xd7, 0x7e),
            error: Color::Rgb(0xed, 0x72, 0x6a),
            focus: Color::Rgb(0x5e, 0x9e, 0xde),
            line: Color::Rgb(0x4a, 0x52, 0x5e),
            selection: Color::Rgb(0x2a, 0x32, 0x40),
            ink: Color::Rgb(0x15, 0x17, 0x1c),
            // Mode badges are backgrounds under `ink` text: softened like the text colours, but
            // never below 7:1 contrast with `ink` (WCAG AAA). Normal and Insert take the full
            // ~13 % (7.2:1 and 8.8:1); Command only ~7 %, as 13 % would leave 6.3:1.
            mode_normal: Color::Rgb(0xc7, 0x9d, 0x5a),
            mode_insert: Color::Rgb(0x6e, 0xc9, 0x76),
            mode_command: Color::Rgb(0xb3, 0x99, 0xd0),
            // Under `ink` text like the others, well above 7:1.
            mode_map: Color::Rgb(0x69, 0xb2, 0xa4),
            map_ring: Color::Rgb(0xff, 0xd8, 0x4a),
            cite: Color::Rgb(0xd9, 0x8c, 0xb3),
            chip: Color::Rgb(0x88, 0xab, 0xca),
            speaker_you: Color::Rgb(0x5e, 0x9e, 0xde),
            speaker_agent: Color::Rgb(0xc7, 0x9d, 0x5a),
            speaker_you_rule: Color::Rgb(0x41, 0x6c, 0x98),
            speaker_agent_rule: Color::Rgb(0x8a, 0x6d, 0x40),
        }
    }

    /// An attachment chip.
    pub fn chip(&self) -> Style {
        Style::new().fg(self.chip)
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
        self.level(u32::from(pct), 61, 81)
    }

    /// A usage limit (task #16): dim below 70 %, orange from 70 %, red from 90 %.
    pub fn usage(&self, pct: u32) -> Style {
        self.level(pct, 70, 90)
    }

    /// Dim below `warn`, amber from `warn`, red from `alarm`.
    fn level(&self, pct: u32, warn: u32, alarm: u32) -> Style {
        if pct >= alarm {
            Style::new().fg(self.error)
        } else if pct >= warn {
            Style::new().fg(self.needs_you)
        } else {
            self.dim()
        }
    }

    /// The selected row.
    pub fn selected(&self) -> Style {
        Style::new().bg(self.selection)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::purple()
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

/// Padding inside reading mode's reading column, around its text (ADR 0018, decided
/// 2026-11-11 later).
pub const READING_PADDING: Padding = Padding { x: 3, y: 1 };

#[cfg(test)]
mod tests {
    use super::*;

    /// Contrast ratio (WCAG) of two RGB colours.
    fn contrast(a: Color, b: [u8; 3]) -> f64 {
        fn lum(c: [u8; 3]) -> f64 {
            let f = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
        }
        let Color::Rgb(r, g, bl) = a else {
            panic!("not rgb")
        };
        let (x, y) = (lum([r, g, bl]), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn purple_text_is_readable_on_the_glass() {
        let t = Theme::purple();
        let glass = [0x17, 0x1b, 0x23];
        assert!(contrast(t.body, glass) >= 10.0);
        for c in [
            t.bold,
            t.italic,
            t.code,
            t.working,
            t.needs_you,
            t.ok,
            t.error,
            t.cite,
            t.chip,
        ] {
            assert!(contrast(c, glass) >= 6.0, "{c:?}");
        }
        assert!(contrast(t.dim, glass) >= 5.0);
        let ink = [0x13, 0x14, 0x1a];
        for c in [t.mode_normal, t.mode_insert, t.mode_command, t.mode_map] {
            assert!(contrast(c, ink) >= 7.0, "{c:?}");
        }
    }

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

    #[test]
    fn usage_turns_orange_at_70_and_red_at_90() {
        let t = Theme::dusk();
        assert_eq!(t.usage(0).fg, Some(t.dim));
        assert_eq!(t.usage(69).fg, Some(t.dim));
        assert_eq!(t.usage(70).fg, Some(t.needs_you));
        assert_eq!(t.usage(89).fg, Some(t.needs_you));
        assert_eq!(t.usage(90).fg, Some(t.error));
        assert_eq!(t.usage(120).fg, Some(t.error));
    }
}

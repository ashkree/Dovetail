//! The Mist colours the variants use, copied from `crates/dovetail-tui/src/theme.rs`
//! (`Theme::mist`). No new hues: tints derive from `selection` and `ink`.

use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub body: Color,
    pub bold: Color,
    pub italic: Color,
    pub code: Color,
    pub link: Color,
    pub dim: Color,
    pub needs_you: Color,
    pub focus: Color,
    pub line: Color,
    pub selection: Color,
    pub ink: Color,
    pub speaker_you: Color,
    pub speaker_agent: Color,
}

impl Theme {
    pub fn mist() -> Self {
        Self {
            body: Color::Rgb(0xd0, 0xd6, 0xf2),
            bold: Color::Rgb(0xb4, 0xbe, 0xfe),
            italic: Color::Rgb(0xf5, 0xc2, 0xe7),
            code: Color::Rgb(0xa6, 0xe3, 0xa1),
            link: Color::Rgb(0x89, 0xb4, 0xfa),
            dim: Color::Rgb(0x9a, 0x9b, 0xb2),
            needs_you: Color::Rgb(0xf9, 0xe2, 0xaf),
            focus: Color::Rgb(0xb4, 0xbe, 0xfe),
            line: Color::Rgb(0x4f, 0x50, 0x66),
            selection: Color::Rgb(0x2c, 0x2e, 0x3e),
            ink: Color::Rgb(0x13, 0x14, 0x1a),
            speaker_you: Color::Rgb(0x89, 0xb4, 0xfa),
            speaker_agent: Color::Rgb(0xb4, 0xbe, 0xfe),
        }
    }

    pub fn body(&self) -> Style {
        Style::new().fg(self.body)
    }
    pub fn dim(&self) -> Style {
        Style::new().fg(self.dim)
    }
    pub fn fg(&self, c: Color) -> Style {
        Style::new().fg(c)
    }
    pub fn accent(&self) -> Style {
        Style::new().fg(self.bold).add_modifier(Modifier::BOLD)
    }
}

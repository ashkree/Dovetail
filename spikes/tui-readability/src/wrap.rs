//! Copied unchanged from `crates/dovetail-tui/src/render/wrap.rs` (`wrap_spans`, `tokens`,
//! `line_width`); `wrap_ranges` is dropped, the spike has no editor.

use std::ops::Range;

use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A space that never breaks a line.
pub const NBSP: char = '\u{a0}';

/// A newline inside a span forces a line break (markdown hard break).
pub const BREAK: &str = "\n";

/// Wraps `spans` to `width` cells at word boundaries, keeping every span's style.
pub fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut line: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    let mut glued = false;

    for span in spans {
        if span.content == BREAK {
            lines.push(finish(std::mem::take(&mut line)));
            used = 0;
            glued = false;
            continue;
        }
        for range in tokens(&span.content) {
            let token = &span.content[range];
            if token.trim().is_empty() {
                if used > 0 && used < width {
                    line.push(Span::styled(" ", span.style));
                    used += 1;
                }
                glued = false;
                continue;
            }
            let w = token.width();
            if used > 0 && used + w > width {
                let start = line
                    .iter()
                    .rposition(|s| s.content.trim().is_empty())
                    .map_or(0, |i| i + 1);
                let carry = if glued && start > 0 {
                    line.split_off(start)
                } else {
                    Vec::new()
                };
                let carried: usize = carry.iter().map(|s| s.content.width()).sum();
                lines.push(finish(std::mem::take(&mut line)));
                used = 0;
                if carried + w <= width {
                    line = carry;
                    used = carried;
                } else if !carry.is_empty() {
                    lines.push(carry);
                }
            }
            glued = true;
            if w <= width {
                line.push(Span::styled(token.to_string(), span.style));
                used += w;
                continue;
            }
            let mut piece = String::new();
            for c in token.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > width {
                    line.push(Span::styled(std::mem::take(&mut piece), span.style));
                    lines.push(finish(std::mem::take(&mut line)));
                    used = 0;
                }
                piece.push(c);
                used += cw;
            }
            line.push(Span::styled(piece, span.style));
        }
    }
    lines.push(finish(line));
    lines
}

fn finish(mut line: Vec<Span<'static>>) -> Vec<Span<'static>> {
    while line.last().is_some_and(|s| s.content.trim().is_empty()) {
        line.pop();
    }
    line
}

/// Splits text into alternating words and whitespace runs, as byte ranges.
fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace() && c != NBSP;
        match in_space {
            Some(prev) if prev != space => {
                out.push(start..i);
                start = i;
            }
            _ => {}
        }
        in_space = Some(space);
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

/// Total cell width of a line of spans.
pub fn line_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

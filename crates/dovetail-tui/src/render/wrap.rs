//! Word wrapping for styled text, and for editable text as byte ranges.

use std::ops::Range;

use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A newline inside a span forces a line break (markdown hard break).
pub const BREAK: &str = "\n";

/// Wraps `spans` to `width` cells at word boundaries, keeping every span's style. Runs of
/// whitespace collapse to one space, a word longer than a line is split, and a span whose text is
/// [`BREAK`] ends the line. Always returns at least one line.
pub fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut line: Vec<Span<'static>> = Vec::new();
    let mut used = 0;

    for span in spans {
        if span.content == BREAK {
            lines.push(finish(std::mem::take(&mut line)));
            used = 0;
            continue;
        }
        for range in tokens(&span.content) {
            let token = &span.content[range];
            if token.trim().is_empty() {
                // Never start a line with a space, and never pile two up.
                if used > 0 && used < width {
                    line.push(Span::styled(" ", span.style));
                    used += 1;
                }
                continue;
            }
            let w = token.width();
            if used > 0 && used + w > width {
                lines.push(finish(std::mem::take(&mut line)));
                used = 0;
            }
            if w <= width {
                line.push(Span::styled(token.to_string(), span.style));
                used += w;
                continue;
            }
            // Longer than a whole line: split by characters.
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

/// Drops a trailing space so lines have no ragged edge.
fn finish(mut line: Vec<Span<'static>>) -> Vec<Span<'static>> {
    while line.last().is_some_and(|s| s.content.trim().is_empty()) {
        line.pop();
    }
    line
}

/// Wraps editable text to `width` cells at word boundaries, as byte ranges into `text`, one per
/// display row. Unlike [`wrap_spans`] nothing is dropped: whitespace is kept as typed and a run
/// of it at a wrap point hangs off the end of its row (so a row may be wider than `width` only
/// by trailing whitespace). A word longer than a whole line is split by characters. Each `\n`
/// ends a row and belongs to no row; every logical line gives at least one row.
pub fn wrap_ranges(text: &str, width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        let mut start = line_start;
        let mut used = 0;
        for range in tokens(line) {
            let token = &line[range.clone()];
            let at = line_start + range.start;
            let w = token.width();
            if token.trim().is_empty() {
                used += w;
                continue;
            }
            if used > 0 && used + w > width {
                rows.push(start..at);
                start = at;
                used = 0;
            }
            if w <= width {
                used += w;
                continue;
            }
            for (i, c) in token.char_indices() {
                let cw = c.width().unwrap_or(0);
                if used > 0 && used + cw > width {
                    rows.push(start..at + i);
                    start = at + i;
                    used = 0;
                }
                used += cw;
            }
        }
        rows.push(start..line_start + line.len());
        line_start += line.len() + 1;
    }
    rows
}

/// Splits text into alternating words and whitespace runs, as byte ranges.
fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    fn text(line: &[Span]) -> String {
        line.iter().map(|s| s.content.as_ref()).collect()
    }

    fn plain(s: &str) -> Vec<Span<'static>> {
        vec![Span::raw(s.to_string())]
    }

    #[test]
    fn wraps_at_word_boundaries() {
        let lines = wrap_spans(&plain("the quick brown fox"), 10);
        let texts: Vec<_> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(texts, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn keeps_styles_across_a_wrap() {
        let red = Style::new().fg(Color::Red);
        let spans = vec![Span::raw("aaa "), Span::styled("bbb ccc", red)];
        let lines = wrap_spans(&spans, 7);
        assert_eq!(text(&lines[0]), "aaa bbb");
        let bbb = lines[0].iter().find(|s| s.content == "bbb").expect("bbb");
        assert_eq!(bbb.style, red);
        assert_eq!(text(&lines[1]), "ccc");
        assert_eq!(lines[1][0].style, red);
    }

    #[test]
    fn splits_a_word_longer_than_the_line() {
        let lines = wrap_spans(&plain("abcdefghij"), 4);
        let texts: Vec<_> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(texts, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn a_break_span_ends_the_line_and_spaces_collapse() {
        let spans = vec![Span::raw("one   two"), Span::raw(BREAK), Span::raw("three")];
        let lines = wrap_spans(&spans, 40);
        let texts: Vec<_> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(texts, vec!["one two", "three"]);
    }

    #[test]
    fn empty_text_is_one_empty_line() {
        assert_eq!(wrap_spans(&[], 10).len(), 1);
    }

    fn ranges(text: &str, width: usize) -> Vec<&str> {
        wrap_ranges(text, width)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn ranges_wrap_at_word_boundaries() {
        assert_eq!(
            ranges("the quick brown fox", 10),
            vec!["the quick ", "brown fox"]
        );
    }

    #[test]
    fn ranges_keep_a_word_that_ends_exactly_at_the_boundary() {
        assert_eq!(ranges("abcde fg", 5), vec!["abcde ", "fg"]);
        assert_eq!(ranges("ab cde", 5), vec!["ab ", "cde"]);
    }

    #[test]
    fn ranges_split_only_a_word_longer_than_the_line() {
        assert_eq!(ranges("ab cdefghij", 4), vec!["ab ", "cdef", "ghij"]);
        assert_eq!(ranges("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn ranges_keep_multiple_spaces_and_hang_them_at_a_wrap() {
        assert_eq!(ranges("ab   cd", 10), vec!["ab   cd"]);
        assert_eq!(ranges("abcd    ef", 5), vec!["abcd    ", "ef"]);
    }

    #[test]
    fn ranges_keep_blank_lines_and_cover_every_byte_but_newlines() {
        let text = "one two three\n\nfour";
        let rows = wrap_ranges(text, 6);
        assert_eq!(ranges(text, 6), vec!["one ", "two ", "three", "", "four"]);
        let covered: usize = rows.iter().map(|r| r.len()).sum();
        assert_eq!(covered + 2, text.len());
        assert_eq!(wrap_ranges("", 5), vec![0..0]);
    }

    #[test]
    fn ranges_respect_wide_characters() {
        assert_eq!(ranges("日本語 です", 6), vec!["日本語 ", "です"]);
    }

    #[test]
    fn no_line_exceeds_the_width() {
        let lines = wrap_spans(&plain("a bb ccc dddd eeeee ffffff ggggggg"), 9);
        assert!(lines.iter().all(|l| line_width(l) <= 9));
    }
}

//! Citation markers in research text (`docs/research.md` §7, §9): `[3]`, `[F3]`, `[S2]`, and
//! lists of them (`[F1, S2]`). Reports cite findings and sources this way; reading colours the
//! markers so they stand out from the prose.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// The char ranges (half-open, in chars) of the citation markers in `text`.
pub fn markers(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some(close) = chars[i + 1..].iter().position(|&c| c == ']')
        {
            let inner: String = chars[i + 1..i + 1 + close].iter().collect();
            if is_citation(&inner) {
                found.push((i, i + close + 2));
                i += close + 2;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// `text` with the spaces inside citation markers made no-break spaces, so a list such as
/// `[F1, S2]` is never wrapped across two lines.
pub fn keep_together(text: &str) -> String {
    let found = markers(text);
    if found.is_empty() {
        return text.to_string();
    }
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if c == ' ' && found.iter().any(|&(a, b)| (a..b).contains(&i)) {
                super::wrap::NBSP
            } else {
                c
            }
        })
        .collect()
}

/// `3`, `F3`, `S12`, or a comma-separated list of those.
fn is_citation(inner: &str) -> bool {
    !inner.is_empty()
        && inner.split(',').all(|part| {
            let part = part.trim();
            let digits = part.strip_prefix(['F', 'S']).unwrap_or(part);
            !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
        })
}

/// `lines` with every citation marker in the citation colour. A marker can straddle spans
/// (the renderer may split text); styles elsewhere are kept.
pub fn colour(lines: Vec<Line<'static>>, theme: &Theme) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| colour_line(line, theme))
        .collect()
}

fn colour_line(line: Line<'static>, theme: &Theme) -> Line<'static> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let found = markers(&text);
    if found.is_empty() {
        return line;
    }
    let cite = Style::new().fg(theme.cite);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut pos = 0;
    for span in &line.spans {
        let mut run = String::new();
        let mut run_cited = false;
        for ch in span.content.chars() {
            let cited = found.iter().any(|&(a, b)| (a..b).contains(&pos));
            if cited != run_cited && !run.is_empty() {
                spans.push(styled(span, std::mem::take(&mut run), run_cited, cite));
            }
            run_cited = cited;
            run.push(ch);
            pos += 1;
        }
        if !run.is_empty() {
            spans.push(styled(span, run, run_cited, cite));
        }
    }
    Line { spans, ..line }
}

fn styled(from: &Span<'static>, text: String, cited: bool, cite: Style) -> Span<'static> {
    let style = if cited {
        from.style.patch(cite)
    } else {
        from.style
    };
    Span::styled(text, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_numbered_and_prefixed_markers() {
        let text = "per seat [1][F3] and [S2, F4], not [x] or [] or [3";
        let found: Vec<&str> = markers(text)
            .into_iter()
            .map(|(a, b)| &text[a..b])
            .collect();
        assert_eq!(found, ["[1]", "[F3]", "[S2, F4]"]);
    }

    #[test]
    fn a_citation_list_is_kept_on_one_line() {
        let kept = keep_together("uses [F1, S2] and more [3]");
        assert_eq!(kept, "uses [F1,\u{a0}S2] and more [3]");
        let wrapped = crate::render::wrap::wrap_spans(&[Span::raw(kept)], 10);
        assert!(
            wrapped
                .iter()
                .any(|l| l.iter().any(|s| s.content.contains("[F1,")))
        );
        assert!(
            wrapped
                .iter()
                .all(|l| !l.iter().any(|s| s.content == "[F1,")),
            "{wrapped:?}"
        );
    }

    #[test]
    fn colours_a_marker_split_over_spans() {
        let theme = Theme::default();
        let line = Line::from(vec![Span::raw("see ["), Span::raw("F3"), Span::raw("] ok")]);
        let out = colour(vec![line], &theme).remove(0);
        let cited: String = out
            .spans
            .iter()
            .filter(|s| s.style.fg == Some(theme.cite))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(cited, "[F3]");
        let all: String = out.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(all, "see [F3] ok");
    }
}

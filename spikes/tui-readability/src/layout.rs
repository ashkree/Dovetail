//! Places blocks: prose in the centred column, wide blocks across the pane, headings per variant.
//! Replaces the shipped `reading_column.rs` (`column`, `reading_text`) with per-variant geometry.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::highlight;
use crate::markdown::{Block, Kind};
use crate::theme::Theme;
use crate::wrap::{line_width, wrap_spans};

#[derive(Clone, Copy, PartialEq)]
pub enum Head {
    Plain,
    /// Bold, with a rule below.
    Rule,
}

#[derive(Clone, Copy)]
pub struct Variant {
    pub name: &'static str,
    pub prose_w: usize,
    pub margin_x: usize,
    pub margin_top: usize,
    pub gap: usize,
    pub gap_head: usize,
    pub head: Head,
    /// Scaled headings through OSC 66 (when the terminal has it).
    pub scaled: bool,
    pub tint_code: bool,
    pub highlight: bool,
    pub zebra: bool,
    pub solid: bool,
}

pub const A: Variant = Variant {
    name: "A quiet",
    prose_w: 88,
    margin_x: 4,
    margin_top: 1,
    gap: 1,
    gap_head: 2,
    head: Head::Rule,
    scaled: false,
    tint_code: false,
    highlight: false,
    zebra: false,
    solid: false,
};
pub const B: Variant = Variant {
    name: "B kitty",
    scaled: true,
    tint_code: true,
    highlight: true,
    ..A
};
pub const C: Variant = Variant {
    name: "C dense",
    prose_w: 110,
    margin_x: 2,
    margin_top: 0,
    gap_head: 1,
    head: Head::Plain,
    scaled: false,
    tint_code: true,
    highlight: true,
    zebra: true,
    solid: true,
    ..A
};

/// What the draw step needs to render one heading at scale, after ratatui's draw.
pub struct Heading {
    pub row: usize,
    pub x: usize,
    pub level: u8,
    pub text: String,
    /// Rows the scaled heading covers; 0 when drawn as normal text.
    pub rows: usize,
}

pub struct WideInfo {
    pub first: usize,
    pub rows: usize,
    pub x: usize,
    pub vis_w: usize,
    pub total_w: usize,
}

pub struct Row {
    pub x: usize,
    pub line: Line<'static>,
    /// Background over `bg_w` cells from `x`.
    pub bg: Option<(Color, usize)>,
    pub wide: Option<usize>,
    /// Index into `headings` when this is a scaled heading's first row.
    pub heading: Option<usize>,
}

pub struct Doc {
    pub rows: Vec<Row>,
    pub headings: Vec<Heading>,
    pub wides: Vec<WideInfo>,
    pub code_blocks: usize,
}

pub struct Opts {
    pub pane_w: usize,
    pub osc66: bool,
    pub highlight: bool,
}

pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

pub fn layout(blocks: &[Block], v: &Variant, o: &Opts, th: &Theme) -> Doc {
    let avail = o.pane_w.saturating_sub(2 * v.margin_x).max(20);
    let pw = v.prose_w.min(avail);
    let col_x = (o.pane_w - pw) / 2;
    let mut doc = Doc {
        rows: Vec::new(),
        headings: Vec::new(),
        wides: Vec::new(),
        code_blocks: 0,
    };
    let blank = || Row {
        x: 0,
        line: Line::default(),
        bg: None,
        wide: None,
        heading: None,
    };
    for _ in 0..v.margin_top {
        doc.rows.push(blank());
    }
    let start = doc.rows.len();
    for block in blocks {
        let before = if doc.rows.len() == start {
            0
        } else if matches!(block, Block::Heading { level, .. } if *level <= 2) {
            v.gap_head
        } else {
            v.gap
        };
        for _ in 0..before {
            doc.rows.push(blank());
        }
        match block {
            Block::Prose(lines) => {
                for line in lines {
                    doc.rows.push(Row {
                        x: col_x,
                        line: line.clone(),
                        ..blank()
                    });
                }
            }
            Block::Rule => doc.rows.push(Row {
                x: col_x,
                line: Line::from(Span::styled("─".repeat(pw.min(40)), th.dim())),
                ..blank()
            }),
            Block::Heading { level, spans } => heading(&mut doc, *level, spans, v, o, pw, col_x, th),
            Block::Wide {
                kind,
                lines,
                zebra,
                raw,
            } => wide(&mut doc, *kind, lines, zebra, raw, v, o, (pw, col_x, avail), th),
        }
    }
    doc
}

#[allow(clippy::too_many_arguments)]
fn heading(
    doc: &mut Doc,
    level: u8,
    spans: &[Span<'static>],
    v: &Variant,
    o: &Opts,
    pw: usize,
    col_x: usize,
    th: &Theme,
) {
    let text = plain(spans);
    let n = text.chars().count();
    // H1 at 2x: two cells a char. H2 at 1.5x inside 2x2 boxes, two chars per 4 cells.
    let scaled_w = if level == 1 { 2 * n } else { n.div_ceil(2) * 4 };
    let can_scale = v.scaled && o.osc66 && level <= 2 && text.is_ascii() && scaled_w <= pw;
    let rule_ch = if level == 1 { "━" } else { "─" };
    let rule = |doc: &mut Doc| {
        if v.head == Head::Rule && level <= 2 {
            doc.rows.push(Row {
                x: col_x,
                line: Line::from(Span::styled(rule_ch.repeat(pw), th.fg(th.line))),
                bg: None,
                wide: None,
                heading: None,
            });
        }
    };
    let blank = |x: usize, line: Line<'static>, heading: Option<usize>| Row {
        x,
        line,
        bg: None,
        wide: None,
        heading,
    };
    if can_scale {
        let idx = doc.headings.len();
        doc.headings.push(Heading {
            row: doc.rows.len(),
            x: col_x,
            level,
            text,
            rows: 2,
        });
        // The buffer holds the heading as normal text on the first row (drawn when the scaled
        // heading is cut by an edge), and a blank second row that the multicell covers.
        doc.rows
            .push(blank(col_x, Line::from(spans.to_vec()), Some(idx)));
        doc.rows.push(blank(col_x, Line::default(), None));
        rule(doc);
        return;
    }
    let style = if level >= 3 {
        Style::new()
            .fg(th.italic)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let restyled: Vec<Span<'static>> = spans
        .iter()
        .map(|s| Span::styled(s.content.clone(), s.style.patch(style)))
        .collect();
    for line in wrap_spans(&restyled, pw) {
        doc.rows.push(blank(col_x, Line::from(line), None));
    }
    rule(doc);
}

type Geometry = (usize, usize, usize);

#[allow(clippy::too_many_arguments)]
fn wide(
    doc: &mut Doc,
    kind: Kind,
    lines: &[Line<'static>],
    zebra: &[bool],
    raw: &Option<(String, String)>,
    v: &Variant,
    o: &Opts,
    (pw, col_x, avail): Geometry,
    th: &Theme,
) {
    let tinted = kind != Kind::Table && v.tint_code;
    let mut body: Vec<Line<'static>> = Vec::new();
    let mut bands: Vec<bool> = Vec::new();
    let mut label = None;
    if let Some((lang, code)) = raw {
        doc.code_blocks += 1;
        let lang_known = !lang.is_empty();
        if lang_known {
            label = Some(lang.clone());
        }
        // Diagrams skip highlighting unless forced; the proposal leaves that open.
        let hl = if o.highlight && kind == Kind::Code {
            highlight::highlight(lang, code, th)
        } else {
            None
        };
        body = hl.unwrap_or_else(|| {
            code.split('\n')
                .map(|l| Line::from(Span::styled(l.to_string(), th.fg(th.code))))
                .collect()
        });
        // 2-col indent; tinted blocks also pad 2 cols on the right.
        for l in &mut body {
            l.spans.insert(0, Span::raw("  "));
            if tinted {
                l.spans.push(Span::raw("  "));
            }
        }
        bands.resize(body.len(), false);
    } else {
        body.extend(lines.iter().cloned());
        bands.extend_from_slice(zebra);
    }
    if let Some(lang) = label {
        let mut l = Line::from(vec![Span::raw("  "), Span::styled(lang, th.dim())]);
        if tinted {
            l.spans.push(Span::raw("  "));
        }
        body.insert(0, l);
        bands.insert(0, false);
    }
    let total = body.iter().map(|l| line_width(&l.spans)).max().unwrap_or(0);
    let vis = total.min(avail).max(1);
    let x = if vis <= pw {
        col_x
    } else {
        (o.pane_w.saturating_sub(vis) / 2).max(v.margin_x)
    };
    let idx = doc.wides.len();
    doc.wides.push(WideInfo {
        first: doc.rows.len(),
        rows: body.len(),
        x,
        vis_w: vis,
        total_w: total,
    });
    for (line, band) in body.into_iter().zip(bands) {
        let bg = if tinted {
            Some((th.selection, vis))
        } else if band && v.zebra {
            Some((th.selection, vis))
        } else {
            None
        };
        doc.rows.push(Row {
            x,
            line,
            bg,
            wide: Some(idx),
            heading: None,
        });
    }
}

/// The cells `[off, off + w)` of `line`; a cut wide glyph becomes a space.
pub fn slice_line(line: &Line<'static>, off: usize, w: usize) -> Line<'static> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut col = 0;
    for span in &line.spans {
        let mut text = String::new();
        for c in span.content.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if col >= off && col + cw <= off + w {
                text.push(c);
            } else if col + cw > off && col < off + w {
                text.push(' ');
            }
            col += cw;
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span.style));
        }
    }
    Line::from(out)
}

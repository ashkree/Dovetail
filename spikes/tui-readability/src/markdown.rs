//! Markdown to blocks. Adapted from `crates/dovetail-tui/src/render/markdown.rs`: the renderer now
//! returns [`Block`]s instead of lines, so layout can place prose in the column and wide blocks
//! (tables, code, diagrams) across the pane, and the draw step knows where headings sit. Spacing
//! between blocks is layout's job (it differs per variant), so there is no `blank()` here.
//! Tables are box-drawn and fitted to the pane width with a 12-col floor; code is kept raw so
//! layout can highlight it.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;
use crate::wrap::{BREAK, line_width, wrap_spans};

const MIN_TEXT_WIDTH: usize = 12;
/// Narrowest a table column is squeezed to (the proposal's floor; the shipped renderer uses 6).
const MIN_COLUMN_WIDTH: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Table,
    Code,
    Diagram,
}

#[derive(Clone)]
pub enum Block {
    Prose(Vec<Line<'static>>),
    Heading {
        level: u8,
        spans: Vec<Span<'static>>,
    },
    /// Tables carry finished `lines` (and `zebra`, true for odd data rows); code carries `raw`.
    Wide {
        kind: Kind,
        lines: Vec<Line<'static>>,
        zebra: Vec<bool>,
        raw: Option<(String, String)>,
    },
    Rule,
}

enum Container {
    Quote,
    Item { marker: String, used: bool },
}

struct Table {
    rows: Vec<Vec<Vec<Span<'static>>>>,
    header_rows: usize,
    row: Vec<Vec<Span<'static>>>,
}

/// Renders `text`: prose wrapped to `prose_w`, tables fitted to `wide_w`.
pub fn render(text: &str, prose_w: usize, wide_w: usize, theme: &Theme) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut r = Renderer {
        theme,
        prose_w,
        wide_w,
        out: Vec::new(),
        spans: Vec::new(),
        styles: Vec::new(),
        containers: Vec::new(),
        lists: Vec::new(),
        code: None,
        link: None,
        table: None,
        heading: None,
    };
    for event in Parser::new_ext(text, options) {
        r.event(event);
    }
    r.flush();
    r.out
}

struct Renderer<'t> {
    theme: &'t Theme,
    prose_w: usize,
    wide_w: usize,
    out: Vec<Block>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    containers: Vec<Container>,
    lists: Vec<Option<u64>>,
    code: Option<(String, String)>,
    link: Option<String>,
    table: Option<Table>,
    heading: Option<u8>,
}

impl Renderer<'_> {
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(self.theme.body(), |base, s| base.patch(*s))
    }

    fn prefixes(&mut self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let dim = self.theme.dim();
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for container in &mut self.containers {
            match container {
                Container::Quote => {
                    first.push(Span::styled("│ ", dim));
                    rest.push(Span::styled("│ ", dim));
                }
                Container::Item { marker, used } => {
                    let pad = " ".repeat(marker.chars().count());
                    first.push(if *used {
                        Span::raw(pad.clone())
                    } else {
                        Span::styled(marker.clone(), dim)
                    });
                    rest.push(Span::raw(pad));
                    *used = true;
                }
            }
        }
        (first, rest)
    }

    fn emit(&mut self, lines: Vec<Vec<Span<'static>>>) {
        let (first, rest) = self.prefixes();
        let lines = lines
            .into_iter()
            .enumerate()
            .map(|(i, body)| {
                let mut line = if i == 0 { first.clone() } else { rest.clone() };
                line.extend(body);
                Line::from(line)
            })
            .collect();
        self.out.push(Block::Prose(lines));
    }

    fn indent(&self) -> usize {
        self.containers
            .iter()
            .map(|c| match c {
                Container::Quote => 2,
                Container::Item { marker, .. } => marker.chars().count(),
            })
            .sum()
    }

    fn flush(&mut self) {
        if self.spans.is_empty() || self.heading.is_some() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        let room = self
            .prose_w
            .saturating_sub(self.indent())
            .max(MIN_TEXT_WIDTH);
        let lines = wrap_spans(&spans, room);
        self.emit(lines);
    }

    fn event(&mut self, event: Event) {
        let theme = self.theme;
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some((_, code)) = self.code.as_mut() {
                    code.push_str(&text);
                } else {
                    let style = self.style();
                    self.spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(text) => {
                let style = self.style().fg(theme.code);
                self.spans.push(Span::styled(text.to_string(), style));
            }
            Event::SoftBreak => {
                let style = self.style();
                self.spans.push(Span::styled(" ", style));
            }
            Event::HardBreak => self.spans.push(Span::raw(BREAK)),
            Event::Rule => {
                self.flush();
                self.out.push(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Container::Item { marker, .. }) = self
                    .containers
                    .iter_mut()
                    .rev()
                    .find(|c| matches!(c, Container::Item { .. }))
                {
                    *marker = if done { "☑ " } else { "☐ " }.to_string();
                }
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                self.spans.push(Span::styled(text.to_string(), theme.dim()));
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        let theme = self.theme;
        match tag {
            Tag::Paragraph => self.flush(),
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level as u8);
                self.styles.push(theme.accent());
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.containers.push(Container::Quote);
                self.styles.push(theme.dim());
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.containers.push(Container::Item {
                    marker,
                    used: false,
                });
            }
            Tag::Emphasis => self
                .styles
                .push(Style::new().fg(theme.italic).add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .styles
                .push(Style::new().fg(theme.bold).add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.styles.push(
                Style::new()
                    .fg(theme.dim)
                    .add_modifier(Modifier::CROSSED_OUT),
            ),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
                self.styles.push(
                    Style::new()
                        .fg(theme.link)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table {
                    rows: Vec::new(),
                    header_rows: 0,
                    row: Vec::new(),
                });
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                let level = self.heading.take().unwrap_or(1);
                let spans = std::mem::take(&mut self.spans);
                self.styles.pop();
                self.out.push(Block::Heading { level, spans });
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.styles.pop();
            }
            TagEnd::CodeBlock => {
                let (lang, code) = self.code.take().unwrap_or_default();
                let code = code.trim_end_matches('\n').to_string();
                let kind = if lang == "mermaid" || is_diagram(&code) {
                    Kind::Diagram
                } else {
                    Kind::Code
                };
                self.out.push(Block::Wide {
                    kind,
                    lines: Vec::new(),
                    zebra: Vec::new(),
                    raw: Some((lang, code)),
                });
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                self.styles.pop();
                if let Some(url) = self.link.take()
                    && !url.is_empty()
                {
                    self.spans
                        .push(Span::styled(format!(" ({url})"), self.theme.dim()));
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.spans);
                if let Some(table) = self.table.as_mut() {
                    table.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                    table.header_rows += 1;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.render_table(table);
                }
            }
            _ => {}
        }
    }

    /// A box-drawn table: natural widths when they fit `wide_w`, else the widest columns shrink
    /// to the 12-col floor and wrap inside cells. If the floors still do not fit, the table is
    /// wider than the pane and layout clips it (sideways scroll).
    fn render_table(&mut self, table: Table) {
        let theme = self.theme;
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let natural: Vec<usize> = (0..columns)
            .map(|c| {
                table
                    .rows
                    .iter()
                    .filter_map(|row| row.get(c))
                    .map(|cell| line_width(cell))
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        let widths = fit_columns(&natural, self.wide_w.saturating_sub(3 * columns + 1));
        let border = |l: &str, m: &str, r: &str| {
            let parts: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
            Line::from(Span::styled(
                format!("{l}{}{r}", parts.join(m)),
                theme.dim(),
            ))
        };
        let mut lines = vec![border("┌", "┬", "┐")];
        let mut zebra = vec![false];
        for (i, row) in table.rows.iter().enumerate() {
            let header = i < table.header_rows;
            let cells: Vec<Vec<Vec<Span<'static>>>> = widths
                .iter()
                .enumerate()
                .map(|(c, width)| {
                    let cell = row.get(c).cloned().unwrap_or_default();
                    let cell: Vec<Span<'static>> = if header {
                        cell.into_iter()
                            .map(|s| Span::styled(s.content, s.style.patch(theme.accent())))
                            .collect()
                    } else {
                        cell
                    };
                    if line_width(&cell) <= *width {
                        vec![cell]
                    } else {
                        wrap_spans(&cell, *width)
                    }
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for r in 0..height {
                let mut line = vec![Span::styled("│ ", theme.dim())];
                for (c, width) in widths.iter().enumerate() {
                    if c > 0 {
                        line.push(Span::styled(" │ ", theme.dim()));
                    }
                    let part = cells[c].get(r).cloned().unwrap_or_default();
                    let used = line_width(&part);
                    line.extend(part);
                    line.push(Span::raw(" ".repeat(width.saturating_sub(used))));
                }
                line.push(Span::styled(" │", theme.dim()));
                lines.push(Line::from(line));
                zebra.push(!header && (i - table.header_rows) % 2 == 1);
            }
            if header && i + 1 == table.header_rows {
                lines.push(border("├", "┼", "┤"));
                zebra.push(false);
            }
        }
        lines.push(border("└", "┴", "┘"));
        zebra.push(false);
        self.out.push(Block::Wide {
            kind: Kind::Table,
            lines,
            zebra,
            raw: None,
        });
    }
}

/// Box-drawing or arrow characters mark a code block as a diagram.
fn is_diagram(code: &str) -> bool {
    code.chars()
        .any(|c| ('\u{2500}'..='\u{257f}').contains(&c) || ('\u{2190}'..='\u{21ff}').contains(&c))
}

/// Column widths that fit `room`: columns narrower than an even share keep their width, the rest
/// split what is left evenly, never below the floor (or the column's own width).
fn fit_columns(natural: &[usize], room: usize) -> Vec<usize> {
    if natural.iter().sum::<usize>() <= room {
        return natural.to_vec();
    }
    let mut fixed = vec![false; natural.len()];
    let mut left = room;
    loop {
        let open = fixed.iter().filter(|f| !**f).count();
        if open == 0 {
            break;
        }
        let share = left / open;
        let mut changed = false;
        for (c, w) in natural.iter().enumerate() {
            if !fixed[c] && *w <= share {
                fixed[c] = true;
                left -= w;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let open = fixed.iter().filter(|f| !**f).count().max(1);
    let (share, mut extra) = (left / open, left % open);
    natural
        .iter()
        .zip(&fixed)
        .map(|(w, fixed)| {
            if *fixed {
                return *w;
            }
            let bonus = usize::from(extra > 0);
            extra = extra.saturating_sub(1);
            (share + bonus).max(MIN_COLUMN_WIDTH.min(*w))
        })
        .collect()
}

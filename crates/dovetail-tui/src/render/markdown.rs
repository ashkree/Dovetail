//! Markdown to styled lines, tuned for reading (Decision 16): a blank row between paragraphs,
//! list items, headings and code blocks; bold, italic and code each keep their style and get
//! their own colour; off-white body text. Code blocks and tables are not wrapped, so they may
//! use the full pane.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrap::{BREAK, line_width, wrap_spans};
use crate::theme::Theme;

/// Narrowest width a paragraph is wrapped to, however deep it is nested.
const MIN_TEXT_WIDTH: usize = 12;

/// Renders `text` to lines at most `width` cells wide (code blocks and tables excepted).
pub fn render(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut r = Renderer::new(width, theme);
    for event in Parser::new_ext(text, options) {
        r.event(event);
    }
    r.finish()
}

/// A block that indents what is inside it.
enum Container {
    Quote,
    Item { marker: String, used: bool },
}

struct Table {
    rows: Vec<Vec<Vec<Span<'static>>>>,
    header_rows: usize,
    row: Vec<Vec<Span<'static>>>,
}

struct Renderer<'t> {
    theme: &'t Theme,
    width: usize,
    out: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    containers: Vec<Container>,
    /// Next number of each open list; `None` for a bullet list.
    lists: Vec<Option<u64>>,
    code: Option<String>,
    link: Option<String>,
    table: Option<Table>,
    in_head: bool,
}

impl<'t> Renderer<'t> {
    fn new(width: usize, theme: &'t Theme) -> Self {
        Self {
            theme,
            width,
            out: Vec::new(),
            spans: Vec::new(),
            styles: Vec::new(),
            containers: Vec::new(),
            lists: Vec::new(),
            code: None,
            link: None,
            table: None,
            in_head: false,
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        while self.out.last().is_some_and(is_blank) {
            self.out.pop();
        }
        self.out
    }

    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(self.theme.body(), |base, s| base.patch(*s))
    }

    fn push_style(&mut self, style: Style) {
        self.styles.push(style);
    }

    fn pop_style(&mut self) {
        self.styles.pop();
    }

    fn blank(&mut self) {
        if !self.out.is_empty() && !self.out.last().is_some_and(is_blank) {
            self.out.push(Line::default());
        }
    }

    /// Prefixes for the first line of a block and for the rest, from the open containers. The
    /// first call inside an item takes its marker.
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
        for (i, body) in lines.into_iter().enumerate() {
            let mut line = if i == 0 { first.clone() } else { rest.clone() };
            line.extend(body);
            self.out.push(Line::from(line));
        }
    }

    /// Wraps and emits the paragraph being collected.
    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        let indent: usize = self
            .containers
            .iter()
            .map(|c| match c {
                Container::Quote => 2,
                Container::Item { marker, .. } => marker.chars().count(),
            })
            .sum();
        let room = self.width.saturating_sub(indent).max(MIN_TEXT_WIDTH);
        let lines = wrap_spans(&spans, room);
        self.emit(lines);
    }

    fn event(&mut self, event: Event) {
        let theme = self.theme;
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some(code) = self.code.as_mut() {
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
                self.blank();
                let rule = "─".repeat(self.width.min(40));
                self.emit(vec![vec![Span::styled(rule, theme.dim())]]);
                self.blank();
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
            Tag::Paragraph => {
                self.flush();
                self.blank();
            }
            Tag::Heading { .. } => {
                self.flush();
                self.blank();
                self.push_style(theme.accent());
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.blank();
                self.containers.push(Container::Quote);
                self.push_style(theme.dim());
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.blank();
                self.code = Some(String::new());
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty()
                {
                    self.emit(vec![vec![Span::styled(lang.to_string(), theme.dim())]]);
                }
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                self.blank();
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
            Tag::Emphasis => {
                self.push_style(Style::new().fg(theme.italic).add_modifier(Modifier::ITALIC))
            }
            Tag::Strong => {
                self.push_style(Style::new().fg(theme.bold).add_modifier(Modifier::BOLD))
            }
            Tag::Strikethrough => self.push_style(
                Style::new()
                    .fg(theme.dim)
                    .add_modifier(Modifier::CROSSED_OUT),
            ),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
                self.push_style(
                    Style::new()
                        .fg(theme.code)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Table(_) => {
                self.flush();
                self.blank();
                self.table = Some(Table {
                    rows: Vec::new(),
                    header_rows: 0,
                    row: Vec::new(),
                });
            }
            Tag::TableHead => self.in_head = true,
            Tag::TableRow | Tag::TableCell => {}
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.pop_style();
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                let style = self.theme.fg(self.theme.code);
                let lines: Vec<Vec<Span<'static>>> = code
                    .trim_end_matches('\n')
                    .split('\n')
                    .map(|l| vec![Span::styled(format!("  {l}"), style)])
                    .collect();
                self.emit(lines);
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link | TagEnd::Image => {
                self.pop_style();
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
                self.in_head = false;
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
                self.blank();
            }
            _ => {}
        }
    }

    fn render_table(&mut self, table: Table) {
        let theme = self.theme;
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|c| {
                table
                    .rows
                    .iter()
                    .filter_map(|row| row.get(c))
                    .map(|cell| line_width(cell))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let mut lines = Vec::new();
        for (i, row) in table.rows.iter().enumerate() {
            let header = i < table.header_rows;
            let mut line: Vec<Span<'static>> = Vec::new();
            for (c, width) in widths.iter().enumerate() {
                if c > 0 {
                    line.push(Span::styled(" │ ", theme.dim()));
                }
                let cell = row.get(c).cloned().unwrap_or_default();
                let used = line_width(&cell);
                for span in cell {
                    line.push(if header {
                        Span::styled(span.content, span.style.patch(theme.accent()))
                    } else {
                        span
                    });
                }
                if c + 1 < columns {
                    line.push(Span::raw(" ".repeat(width - used)));
                }
            }
            lines.push(line);
            if header && i + 1 == table.header_rows {
                let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                lines.push(vec![Span::styled(rule.join("─┼─"), theme.dim())]);
            }
        }
        self.emit(lines);
    }
}

fn is_blank(line: &Line) -> bool {
    line.spans.iter().all(|s| s.content.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn theme() -> Theme {
        Theme::dusk()
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn find<'a>(lines: &'a [Line<'static>], needle: &str) -> &'a Span<'static> {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(needle))
            .unwrap_or_else(|| panic!("no span containing {needle:?}"))
    }

    #[test]
    fn bold_italic_and_code_keep_their_style_and_get_their_own_colour() {
        let t = theme();
        let lines = render("a **bold** and *italic* and `code` word", 80, &t);
        let bold = find(&lines, "bold");
        assert_eq!(bold.style.fg, Some(t.bold));
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let italic = find(&lines, "italic");
        assert_eq!(italic.style.fg, Some(t.italic));
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
        let code = find(&lines, "code");
        assert_eq!(code.style.fg, Some(t.code));
        let body = find(&lines, "a");
        assert_eq!(body.style.fg, Some(t.body));
        assert_ne!(t.body, Color::Rgb(255, 255, 255), "never pure white");
    }

    #[test]
    fn the_three_emphasis_colours_and_the_body_are_all_different() {
        let t = theme();
        let all = [t.body, t.bold, t.italic, t.code];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn a_blank_row_separates_paragraphs() {
        let lines = render("one\n\ntwo", 80, &theme());
        assert_eq!(texts(&lines), vec!["one", "", "two"]);
    }

    #[test]
    fn a_blank_row_separates_list_items() {
        let lines = render("- a\n- b\n- c", 80, &theme());
        assert_eq!(texts(&lines), vec!["• a", "", "• b", "", "• c"]);
    }

    #[test]
    fn blank_rows_surround_headings_and_code_blocks() {
        let lines = render(
            "intro\n\n## Head\n\ntext\n\n```rust\nlet x = 1;\n```\n\nafter",
            80,
            &theme(),
        );
        let t = texts(&lines);
        assert_eq!(
            t,
            vec![
                "intro",
                "",
                "Head",
                "",
                "text",
                "",
                "rust",
                "  let x = 1;",
                "",
                "after"
            ]
        );
        assert_eq!(find(&lines, "Head").style.fg, Some(theme().bold));
    }

    #[test]
    fn ordered_lists_number_and_continuation_lines_hang_under_the_text() {
        let lines = render(
            "1. first item that is long enough to wrap onto a second line\n2. second",
            24,
            &theme(),
        );
        let t = texts(&lines);
        assert!(t[0].starts_with("1. first"), "{t:?}");
        assert!(t[1].starts_with("   "), "{t:?}");
        assert!(t.contains(&"2. second".to_string()));
    }

    #[test]
    fn paragraphs_wrap_to_the_width() {
        let lines = render("aaa bbb ccc ddd eee fff ggg hhh", 12, &theme());
        assert!(texts(&lines).iter().all(|l| l.chars().count() <= 12));
        assert!(lines.len() > 1);
    }

    #[test]
    fn code_blocks_are_not_wrapped() {
        let long = "x".repeat(150);
        let lines = render(&format!("```\n{long}\n```"), 40, &theme());
        assert!(texts(&lines).iter().any(|l| l.chars().count() > 100));
    }

    #[test]
    fn nested_lists_indent() {
        let lines = render("- outer\n  - inner", 80, &theme());
        let t = texts(&lines);
        assert_eq!(t[0], "• outer");
        assert_eq!(t[2], "  • inner");
    }

    #[test]
    fn block_quotes_get_a_bar_and_dim_text() {
        let t = theme();
        let lines = render("> quoted words", 80, &t);
        assert_eq!(texts(&lines), vec!["│ quoted words"]);
        assert_eq!(find(&lines, "quoted").style.fg, Some(t.dim));
    }

    #[test]
    fn tables_align_their_columns() {
        let lines = render("| a | long head |\n| - | - |\n| xx | y |", 80, &theme());
        let t = texts(&lines);
        assert_eq!(t[0], "a  │ long head");
        assert!(t[1].contains("─┼─"));
        assert_eq!(t[2], "xx │ y");
    }

    #[test]
    fn links_show_their_address_and_task_items_their_box() {
        let lines = render(
            "[docs](https://x.dev)\n\n- [x] done\n- [ ] todo",
            80,
            &theme(),
        );
        let t = texts(&lines);
        assert_eq!(t[0], "docs (https://x.dev)");
        assert!(t.contains(&"☑ done".to_string()));
        assert!(t.contains(&"☐ todo".to_string()));
    }

    #[test]
    fn empty_input_renders_nothing() {
        assert!(render("", 80, &theme()).is_empty());
    }

    #[test]
    fn a_hard_break_starts_a_new_line_without_a_blank_row() {
        let lines = render("one  \ntwo", 80, &theme());
        assert_eq!(texts(&lines), vec!["one", "two"]);
    }
}

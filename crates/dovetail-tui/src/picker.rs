//! The one picker: a floating search box and result list with a preview beside it, ranked by
//! `nucleo`. The agent switcher (`<leader>ss`) and the everything picker (`<leader><leader>`)
//! are this component with different items.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{AgentId, InboxId, ProjectId, TaskId};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::fmt;
use crate::input::{InputOutcome, TextInput};
use crate::theme::Theme;

/// What a picker row stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerKey {
    /// An agent.
    Agent(AgentId),
    /// A task.
    Task(TaskId),
    /// An inbox item.
    Inbox(InboxId),
    /// A slash command or skill, by name.
    Command(String),
    /// A registered project (the project switcher).
    Project(ProjectId),
    /// A Claude profile from config.toml, or `None` for the default (after `:project add`).
    Profile(Option<String>),
}

/// One row.
#[derive(Debug, Clone)]
pub struct PickerItem {
    /// What choosing it opens.
    pub key: PickerKey,
    /// The name shown.
    pub label: String,
    /// What is searched: the label plus anything else worth matching (branch, task title).
    pub haystack: String,
    /// Right-hand text: branch and age.
    pub detail: String,
    /// Status glyph.
    pub glyph: &'static str,
    /// Glyph colour.
    pub color: Color,
    /// Indent level (subagents sit under their parent).
    pub depth: usize,
    /// Section heading shown while the query is empty.
    pub section: &'static str,
}

/// What a key did to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Still open.
    Pending,
    /// A row was chosen.
    Chosen(PickerKey),
    /// Closed without choosing.
    Closed,
}

struct Shown {
    item: usize,
    /// Matched character positions in the label, for highlighting.
    indices: Vec<u32>,
}

/// A picker over a fixed list of items.
pub struct Picker {
    title: String,
    items: Vec<PickerItem>,
    query: TextInput,
    shown: Vec<Shown>,
    selected: usize,
    matcher: Matcher,
}

impl Picker {
    /// The title shown on the box.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// A picker over `items`, in the order they should show for an empty query.
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        let mut picker = Self {
            title: title.into(),
            items,
            query: TextInput::line(""),
            shown: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        picker.refilter();
        picker
    }

    /// The text typed so far.
    pub fn query(&self) -> &str {
        self.query.text()
    }

    /// The rows now showing, best first.
    pub fn shown(&self) -> Vec<&PickerItem> {
        self.shown.iter().map(|s| &self.items[s.item]).collect()
    }

    /// The highlighted row.
    pub fn selected(&self) -> Option<&PickerItem> {
        self.shown.get(self.selected).map(|s| &self.items[s.item])
    }

    /// Applies a key: type to filter, `Ctrl-j`/`Ctrl-k` or the arrows to move, `Enter` to choose,
    /// `Esc` to close.
    pub fn key(&mut self, key: KeyEvent) -> PickerOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') if ctrl || key.code == KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.shown.len().saturating_sub(1));
                PickerOutcome::Pending
            }
            KeyCode::Up | KeyCode::Char('k') if ctrl || key.code == KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                PickerOutcome::Pending
            }
            _ => match self.query.handle(key) {
                InputOutcome::Submit => self.selected().map_or(PickerOutcome::Pending, |i| {
                    PickerOutcome::Chosen(i.key.clone())
                }),
                InputOutcome::Cancel => PickerOutcome::Closed,
                InputOutcome::Edited => {
                    self.refilter();
                    PickerOutcome::Pending
                }
                InputOutcome::Ignored => PickerOutcome::Pending,
            },
        }
    }

    /// Re-ranks the items for the current query. An empty query keeps the given order; a typed
    /// one ranks by `nucleo`'s score, ties in the given order.
    fn refilter(&mut self) {
        self.selected = 0;
        let query = self.query.text().trim();
        if query.is_empty() {
            self.shown = (0..self.items.len())
                .map(|item| Shown {
                    item,
                    indices: Vec::new(),
                })
                .collect();
            return;
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let hay = Utf32Str::new(&item.haystack, &mut buf);
                pattern.score(hay, &mut self.matcher).map(|s| (s, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.shown = scored
            .into_iter()
            .map(|(_, item)| {
                let mut indices = Vec::new();
                let hay = Utf32Str::new(&self.items[item].label, &mut buf);
                pattern.indices(hay, &mut self.matcher, &mut indices);
                Shown { item, indices }
            })
            .collect();
    }

    /// Draws the picker over `screen`: search and results on the left, `preview` on the right
    /// (the end of the preview shows when it is too long, which is where an agent's latest turn
    /// is).
    pub fn render(
        &self,
        frame: &mut Frame,
        screen: Rect,
        theme: &Theme,
        preview_title: &str,
        preview: &[Line<'static>],
    ) {
        let width = screen.width.saturating_sub(4).min(130);
        let height = screen.height.saturating_sub(4).min(26);
        let area = Rect {
            x: screen.x + (screen.width - width) / 2,
            y: screen.y + (screen.height - height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, area);
        let halves = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);

        let left = Block::bordered()
            .border_style(theme.fg(theme.focus))
            .title(Line::from(Span::styled(
                format!(" {} ", self.title),
                theme.fg(theme.focus),
            )));
        let inner = left.inner(halves[0]);
        frame.render_widget(left, halves[0]);
        if inner.height < 3 || inner.width < 6 {
            return;
        }

        // Search row: prompt, text, count.
        let count = format!("{}/{}", self.shown.len(), self.items.len());
        let prompt = Line::from(vec![
            Span::styled("› ", theme.accent()),
            Span::styled(self.query.text().to_string(), theme.body()),
        ]);
        let row = Rect { height: 1, ..inner };
        frame.render_widget(Paragraph::new(prompt), row);
        let count_x = row.x + row.width.saturating_sub(count.len() as u16);
        frame.render_widget(
            Paragraph::new(Span::styled(count, theme.dim())),
            Rect {
                x: count_x,
                width: row.x + row.width - count_x,
                ..row
            },
        );
        frame.set_cursor_position((
            (row.x + 2 + unicode_width::UnicodeWidthStr::width(self.query.text()) as u16)
                .min(row.x + row.width.saturating_sub(1)),
            row.y,
        ));

        // Results.
        let list_area = Rect {
            y: inner.y + 2,
            height: inner.height - 2,
            ..inner
        };
        let headings = self.query.text().trim().is_empty();
        let mut lines: Vec<(Line<'static>, bool)> = Vec::new();
        let mut last_section = "";
        let mut selected_line = 0;
        for (n, shown) in self.shown.iter().enumerate() {
            let item = &self.items[shown.item];
            if headings && item.section != last_section && !item.section.is_empty() {
                lines.push((
                    Line::from(Span::styled(
                        item.section.to_string(),
                        theme.dim().add_modifier(Modifier::BOLD),
                    )),
                    false,
                ));
                last_section = item.section;
            }
            if n == self.selected {
                selected_line = lines.len();
            }
            lines.push((
                item_line(
                    item,
                    shown,
                    n == self.selected,
                    list_area.width as usize,
                    theme,
                ),
                n == self.selected,
            ));
        }
        let height = usize::from(list_area.height);
        let offset = (selected_line + 1).saturating_sub(height);
        for (n, (line, selected)) in lines.into_iter().skip(offset).take(height).enumerate() {
            let rect = Rect {
                y: list_area.y + n as u16,
                height: 1,
                ..list_area
            };
            frame.render_widget(Paragraph::new(line), rect);
            if selected {
                frame.buffer_mut().set_style(rect, theme.selected());
            }
        }

        // Preview.
        let right = Block::bordered()
            .border_style(theme.fg(theme.line))
            .title(Line::from(Span::styled(
                format!(" {preview_title} "),
                theme.dim(),
            )));
        let pinner = right.inner(halves[1]);
        frame.render_widget(right, halves[1]);
        let pheight = usize::from(pinner.height);
        let skip = preview.len().saturating_sub(pheight);
        for (n, line) in preview.iter().skip(skip).take(pheight).enumerate() {
            let rect = Rect {
                x: pinner.x + 1,
                y: pinner.y + n as u16,
                width: pinner.width.saturating_sub(2),
                height: 1,
            };
            frame.render_widget(Paragraph::new(line.clone()), rect);
        }
    }
}

fn item_line(
    item: &PickerItem,
    shown: &Shown,
    selected: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(if selected { "▌" } else { " " }, theme.fg(theme.focus)),
        Span::styled(item.glyph, theme.fg(item.color)),
        Span::raw(" "),
        Span::raw("  ".repeat(item.depth)),
    ];
    if item.depth > 0 {
        spans.push(Span::styled("└ ", theme.dim()));
    }
    for (i, c) in item.label.chars().enumerate() {
        let hit = shown.indices.contains(&(i as u32));
        spans.push(Span::styled(
            c.to_string(),
            if hit { theme.accent() } else { theme.body() },
        ));
    }
    let right = vec![Span::styled(fmt::truncate(&item.detail, 28), theme.dim())];
    super::buffers::spread_fit(spans, &[right], width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, section: &'static str) -> PickerItem {
        PickerItem {
            key: PickerKey::Agent(AgentId(name.into())),
            label: name.into(),
            haystack: name.into(),
            detail: String::new(),
            glyph: "○",
            color: Color::Reset,
            depth: 0,
            section,
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn picker() -> Picker {
        Picker::new(
            "Switch",
            vec![
                item("planner", "Agents"),
                item("eval run", "Agents"),
                item("triage group", "Agents"),
                item("rebase slice", "Agents"),
            ],
        )
    }

    fn names(p: &Picker) -> Vec<String> {
        p.shown().iter().map(|i| i.label.clone()).collect()
    }

    #[test]
    fn an_empty_query_keeps_the_given_order() {
        assert_eq!(
            names(&picker()),
            vec!["planner", "eval run", "triage group", "rebase slice"]
        );
    }

    #[test]
    fn typing_filters_and_ranks_by_fuzzy_score() {
        let mut p = picker();
        for c in "eval".chars() {
            p.key(key(c));
        }
        assert_eq!(names(&p).first().map(String::as_str), Some("eval run"));
        assert!(names(&p).len() < 4);
        // A fuzzy subsequence still matches.
        let mut p = picker();
        for c in "trg".chars() {
            p.key(key(c));
        }
        assert_eq!(names(&p), vec!["triage group"]);
    }

    #[test]
    fn ties_keep_the_given_order() {
        let mut p = Picker::new(
            "Switch",
            vec![
                item("slice-a", ""),
                item("slice-b", ""),
                item("slice-c", ""),
            ],
        );
        for c in "slice".chars() {
            p.key(key(c));
        }
        assert_eq!(names(&p), vec!["slice-a", "slice-b", "slice-c"]);
    }

    #[test]
    fn ctrl_j_and_ctrl_k_and_the_arrows_move_and_clamp() {
        let mut p = picker();
        p.key(ctrl('j'));
        p.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(p.selected().map(|i| i.label.as_str()), Some("triage group"));
        for _ in 0..10 {
            p.key(ctrl('j'));
        }
        assert_eq!(p.selected().map(|i| i.label.as_str()), Some("rebase slice"));
        for _ in 0..10 {
            p.key(ctrl('k'));
        }
        assert_eq!(p.selected().map(|i| i.label.as_str()), Some("planner"));
    }

    #[test]
    fn enter_chooses_the_highlighted_row_and_esc_closes() {
        let mut p = picker();
        p.key(ctrl('j'));
        assert_eq!(
            p.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            PickerOutcome::Chosen(PickerKey::Agent(AgentId("eval run".into())))
        );
        assert_eq!(
            p.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            PickerOutcome::Closed
        );
    }

    #[test]
    fn enter_with_no_matches_does_nothing() {
        let mut p = picker();
        for c in "zzzz".chars() {
            p.key(key(c));
        }
        assert!(p.shown().is_empty());
        assert_eq!(
            p.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            PickerOutcome::Pending
        );
    }

    #[test]
    fn j_and_k_are_text_not_movement() {
        let mut p = picker();
        p.key(key('j'));
        assert_eq!(p.query(), "j");
    }

    #[test]
    fn smart_case_matches_lowercase_queries_against_any_case() {
        let mut p = Picker::new("x", vec![item("Business home", "")]);
        for c in "business".chars() {
            p.key(key(c));
        }
        assert_eq!(names(&p), vec!["Business home"]);
    }
}

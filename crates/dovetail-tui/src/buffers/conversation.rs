//! One agent's conversation: the reading column, scrolling, folding turns and the chat input.
//! The text itself is built by `render::transcript`.

use std::collections::{HashMap, HashSet};

use crossterm::event::KeyEvent;
use dovetail_api::{AgentId, AgentStatus, Command, EntryId, EntryKind};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Buffer, Effect, ViewCx, pane, status_glyph, status_word, title};
use crate::action::Action;
use crate::fmt;
use crate::input::{InputOutcome, TextInput};
use crate::region::BufferKind;
use crate::render::reading_column::{self, DEFAULT_WIDTH};
use crate::render::transcript::{self, Turn};

/// Most rows the chat input grows to before it scrolls.
const MAX_INPUT_ROWS: usize = 6;

/// Per-agent view state, so switching agents keeps each one's place.
struct AgentView {
    /// First visible line.
    top: usize,
    /// Stuck to the bottom: new text scrolls into view.
    follow: bool,
    folded: HashSet<EntryId>,
    expanded: HashSet<EntryId>,
    cache: transcript::Cache,
    draft: String,
    /// The oldest entry when older history was last asked for, to ask once.
    loading: Option<EntryId>,
    /// Oldest entry and line count at the last draw, to keep the view in place when older
    /// history arrives above it.
    drawn: Option<(EntryId, usize)>,
}

impl Default for AgentView {
    /// A new view follows the bottom, like a chat.
    fn default() -> Self {
        Self {
            top: 0,
            follow: true,
            folded: HashSet::new(),
            expanded: HashSet::new(),
            cache: transcript::Cache::default(),
            draft: String::new(),
            loading: None,
            drawn: None,
        }
    }
}

/// What the last draw learned about the text, for the keys that follow it.
#[derive(Default)]
struct Layout {
    /// The lines of the last draw, for search.
    lines: Vec<Line<'static>>,
    turns: Vec<Turn>,
    max_top: usize,
    /// Older history exists; its oldest known entry.
    older: Option<EntryId>,
}

/// The conversation buffer.
pub struct ConversationBuffer {
    agent: Option<AgentId>,
    previous: Option<AgentId>,
    views: HashMap<AgentId, AgentView>,
    chat: Option<TextInput>,
    reading_width: u16,
    solid: bool,
    layout: Layout,
}

impl Default for ConversationBuffer {
    fn default() -> Self {
        Self {
            agent: None,
            previous: None,
            views: HashMap::new(),
            chat: None,
            reading_width: DEFAULT_WIDTH,
            solid: false,
            layout: Layout::default(),
        }
    }
}

impl ConversationBuffer {
    /// The agent shown.
    pub fn agent(&self) -> Option<&AgentId> {
        self.agent.as_ref()
    }

    /// The agent shown before this one, for the alternate-agent key.
    pub fn previous(&self) -> Option<&AgentId> {
        self.previous.as_ref()
    }

    /// Shows another agent, remembering the one it replaces.
    pub fn show(&mut self, agent: AgentId) {
        if self.agent.as_ref() != Some(&agent) {
            self.previous = self.agent.replace(agent);
            self.layout = Layout::default();
        }
    }

    /// Whether the reading column has a solid background.
    pub fn solid(&self) -> bool {
        self.solid
    }

    /// Toggles the solid reading background.
    pub fn toggle_solid(&mut self) {
        self.solid = !self.solid;
    }

    /// Starts typing to `agent`, picking up its unsent draft.
    pub fn begin_chat(&mut self, agent: AgentId) {
        self.show(agent.clone());
        let draft = self.views.entry(agent).or_default().draft.clone();
        self.chat = Some(TextInput::multiline(&draft));
    }

    /// Whether the chat input is open.
    pub fn is_chatting(&self) -> bool {
        self.chat.is_some()
    }

    /// Applies a key to the chat input. `Enter` returns the prompt to send; `Esc` keeps the
    /// text as the agent's draft.
    pub fn chat_key(&mut self, key: KeyEvent) -> Option<(AgentId, String)> {
        let chat = self.chat.as_mut()?;
        let agent = self.agent.clone()?;
        match chat.handle(key) {
            InputOutcome::Submit => {
                let text = chat.text().trim().to_string();
                self.chat = None;
                let view = self.views.entry(agent.clone()).or_default();
                if text.is_empty() {
                    return None;
                }
                view.draft.clear();
                view.follow = true;
                Some((agent, text))
            }
            InputOutcome::Cancel => {
                let text = chat.text().to_string();
                self.chat = None;
                self.views.entry(agent).or_default().draft = text;
                None
            }
            InputOutcome::Edited | InputOutcome::Ignored => None,
        }
    }

    fn view(&mut self) -> Option<&mut AgentView> {
        let agent = self.agent.clone()?;
        Some(self.views.entry(agent).or_default())
    }

    /// Asks for older history once, when the view is at the top and the tail says there is more.
    fn maybe_load_older(&mut self, effects: &mut Vec<Effect>) {
        let (Some(agent), Some(oldest)) = (self.agent.clone(), self.layout.older) else {
            return;
        };
        if let Some(view) = self.view()
            && view.top == 0
            && view.loading != Some(oldest)
        {
            view.loading = Some(oldest);
            effects.push(Effect::Command(Command::LoadConversation {
                agent,
                before: Some(oldest),
            }));
        }
    }

    fn scroll_to(&mut self, top: usize) {
        let max_top = self.layout.max_top;
        if let Some(view) = self.view() {
            view.top = top.min(max_top);
            view.follow = top >= max_top;
        }
    }

    /// Index of the turn at the top of the view (the last one while following the bottom).
    fn current_turn(&self, top: usize, follow: bool) -> Option<usize> {
        let turns = &self.layout.turns;
        if follow {
            return turns.len().checked_sub(1);
        }
        turns.iter().rposition(|t| t.start_line <= top)
    }

    fn render_input(&self, frame: &mut Frame, area: Rect, cx: &ViewCx) {
        let Some(chat) = &self.chat else { return };
        let theme = cx.theme;
        let text_width = usize::from(area.width.saturating_sub(2));
        let rows = chat.rows(text_width);
        let (cursor_row, cursor_col) = chat.cursor_position(text_width);
        let first = (cursor_row + 1).saturating_sub(usize::from(area.height));
        for (n, range) in rows
            .iter()
            .skip(first)
            .take(usize::from(area.height))
            .enumerate()
        {
            let prefix = if first + n == 0 { "› " } else { "  " };
            let line = Line::from(vec![
                Span::styled(prefix, theme.accent()),
                Span::styled(chat.text()[range.clone()].to_string(), theme.body()),
            ]);
            let rect = Rect {
                y: area.y + n as u16,
                height: 1,
                ..area
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        frame.set_cursor_position(Position::new(
            area.x + 2 + cursor_col as u16,
            area.y + (cursor_row - first) as u16,
        ));
    }
}

impl Buffer for ConversationBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Conversation
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let agent = self
            .agent
            .as_ref()
            .and_then(|id| cx.snap.agents.iter().find(|a| &a.id == id));
        let heading = match agent {
            Some(a) => {
                let (glyph, _) = status_glyph(a.status, theme);
                let mut parts = vec![a.name.clone(), format!("{glyph} {}", status_word(a.status))];
                if let Some(mb) = a.memory_mb {
                    parts.push(fmt::memory(mb));
                }
                parts.push(fmt::cost(a.cost_usd));
                parts.join(" · ")
            }
            None => "no agent".into(),
        };
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        let inner = crate::theme::CHAT_PADDING.shrink(inner);

        // Bottom: a rule, then the chat input or a hint.
        let input_rows = self.chat.as_mut().map_or(1, |c| {
            let width = usize::from(inner.width.saturating_sub(2));
            c.set_wrap_width(width);
            c.rows(width).len().clamp(1, MAX_INPUT_ROWS)
        });
        let bottom = (1 + input_rows as u16).min(inner.height.saturating_sub(1));
        let body = Rect {
            height: inner.height - bottom,
            ..inner
        };
        let rule = Rect {
            y: body.y + body.height,
            height: 1.min(bottom),
            ..inner
        };
        let input_area = Rect {
            y: rule.y + rule.height,
            height: bottom - rule.height,
            ..inner
        };
        if rule.height > 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "─".repeat(usize::from(rule.width)),
                    theme.dim(),
                )),
                rule,
            );
        }
        if self.chat.is_some() {
            self.render_input(frame, input_area, cx);
        } else if input_area.height > 0 {
            let hint = Rect {
                height: 1,
                ..input_area
            };
            let text = fmt::truncate(
                "› i to chat · ]] [[ turns · zc zo fold · gg G",
                usize::from(hint.width),
            );
            frame.render_widget(Paragraph::new(Span::styled(text, theme.dim())), hint);
        }

        let Some(agent) = agent else { return };
        let column = reading_column::column(body, self.reading_width);
        let tail = cx.snap.conversations.get(&agent.id);
        let entries = tail.map_or(&[][..], |t| t.entries.as_slice());
        let streaming = (agent.status == AgentStatus::Working)
            .then(|| entries.last())
            .flatten()
            .filter(|e| e.kind == EntryKind::Assistant)
            .map(|e| e.id);
        let agent_id = agent.id.clone();
        let name = agent.name.clone();
        let older = tail
            .filter(|t| t.has_more)
            .and_then(|t| t.entries.first().map(|e| e.id));
        let view = self.views.entry(agent_id).or_default();
        let params = transcript::Params {
            width: usize::from(column.width),
            theme,
            tz: cx.tz,
            agent: &name,
            streaming,
            folded: &view.folded,
            expanded: &view.expanded,
        };
        let mut built = transcript::build(entries, &params, &mut view.cache);
        if older.is_some() {
            let note = Line::from(Span::styled(
                "↑ older history · k or gg at the top loads it",
                theme.dim(),
            ));
            built.lines.splice(0..0, [note, Line::default()]);
            for turn in &mut built.turns {
                turn.start_line += 2;
            }
        }

        // Older history landed above the view: move down by what it added so the lines being
        // read stay put.
        let oldest = entries.first().map(|e| e.id);
        if let (Some((was, lines)), Some(now)) = (view.drawn, oldest)
            && was != now
            && entries.iter().any(|e| e.id == was)
            && !view.follow
        {
            let grown = built.lines.len() as isize - lines as isize;
            view.top = view.top.saturating_add_signed(grown);
        }
        view.drawn = oldest.map(|id| (id, built.lines.len()));

        let height = usize::from(body.height);
        let max_top = built.lines.len().saturating_sub(height);
        if view.follow || view.top > max_top {
            view.top = max_top;
        }
        let top = view.top;
        for (row, line) in built.lines.iter().skip(top).take(height).enumerate() {
            let rect = Rect {
                x: column.x,
                y: body.y + row as u16,
                width: body.x + body.width - column.x,
                height: 1,
            };
            frame.render_widget(Paragraph::new(line.clone()), rect);
        }
        if self.solid {
            reading_column::paint_background(
                frame.buffer_mut(),
                column,
                Style::new().bg(theme.reading_bg),
            );
        }
        self.layout = Layout {
            lines: built.lines,
            turns: built.turns,
            max_top,
            older,
        };
    }

    fn on_action(&mut self, action: Action, _cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let Some((top, follow)) = self.view().map(|v| (v.top, v.follow)) else {
            return false;
        };
        match action {
            Action::MoveDown => self.scroll_to(top + 1),
            Action::MoveUp => {
                self.scroll_to(top.saturating_sub(1));
                self.maybe_load_older(effects);
            }
            Action::GoTop => {
                self.scroll_to(0);
                self.maybe_load_older(effects);
            }
            Action::GoBottom => self.scroll_to(self.layout.max_top),
            Action::NextTurn => {
                let next = self
                    .layout
                    .turns
                    .iter()
                    .map(|t| t.start_line)
                    .find(|&start| start > top);
                self.scroll_to(next.unwrap_or(self.layout.max_top));
            }
            Action::PrevTurn => {
                let prev = self
                    .layout
                    .turns
                    .iter()
                    .map(|t| t.start_line)
                    .rfind(|&start| start < top);
                self.scroll_to(prev.unwrap_or(0));
                self.maybe_load_older(effects);
            }
            Action::FoldTurn | Action::UnfoldTurn => {
                let Some(turn) = self
                    .current_turn(top, follow)
                    .map(|i| self.layout.turns[i].clone())
                else {
                    return true;
                };
                if let Some(view) = self.view() {
                    if action == Action::FoldTurn {
                        view.folded.insert(turn.key);
                        // Stay on the folded turn rather than drifting below it.
                        view.follow = false;
                        view.top = turn.start_line;
                    } else {
                        view.folded.remove(&turn.key);
                        view.expanded.extend(turn.entries.iter().copied());
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn search_rows(&self, _cx: &ViewCx) -> Vec<String> {
        self.layout
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn current_row(&self, _cx: &ViewCx) -> usize {
        self.agent
            .as_ref()
            .and_then(|a| self.views.get(a))
            .map_or(0, |v| v.top)
    }

    fn goto_row(&mut self, row: usize, _cx: &ViewCx, _effects: &mut Vec<Effect>) {
        // Leave a line of context above the match.
        self.scroll_to(row.saturating_sub(1));
    }
}

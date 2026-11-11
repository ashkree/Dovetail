//! One agent's conversation: the reading column, scrolling, folding turns and the chat input.
//! The text itself is built by `render::transcript`.

use std::collections::{HashMap, HashSet};

use crossterm::event::KeyEvent;
use dovetail_api::{
    AgentId, AgentStatus, Attachment, Command, EntryId, EntryKind, MAX_ATTACHMENTS, Paste,
    SlashCommand,
};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use super::{Buffer, Effect, ViewCx, pane, status_glyph, status_word, title};
use crate::action::Action;
use crate::complete::{self, Completion, KeyResult};
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
    /// Where the cursor was in the draft when the chat bar was left.
    draft_cursor: usize,
    /// Images attached to the unsent draft.
    draft_images: Vec<Attachment>,
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
            draft_cursor: 0,
            draft_images: Vec::new(),
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
    /// Rows of text in view, for paging.
    page: usize,
    /// Older history exists; its oldest known entry.
    older: Option<EntryId>,
}

/// The conversation buffer.
pub struct ConversationBuffer {
    agent: Option<AgentId>,
    previous: Option<AgentId>,
    views: HashMap<AgentId, AgentView>,
    chat: Option<TextInput>,
    /// Images attached to the message being typed, shown as chips above the input.
    images: Vec<Attachment>,
    /// The `/` and `@` popup over the chat input.
    completion: Completion,
    /// A path listing the popup needs, for the app to ask the core for.
    wants_paths: Option<Option<String>>,
    reading_width: u16,
    layout: Layout,
    /// [`BufferKind::Conversation`] in the main region, [`BufferKind::SidePanel`] inside the
    /// side panel.
    kind: BufferKind,
    /// Added to the pane title (the side panel's "2/3 ⇥").
    heading_suffix: Option<String>,
}

impl Default for ConversationBuffer {
    fn default() -> Self {
        Self {
            agent: None,
            previous: None,
            views: HashMap::new(),
            chat: None,
            images: Vec::new(),
            completion: Completion::default(),
            wants_paths: None,
            reading_width: DEFAULT_WIDTH,
            layout: Layout::default(),
            kind: BufferKind::Conversation,
            heading_suffix: None,
        }
    }
}

impl ConversationBuffer {
    /// The conversation view of the side panel.
    pub fn for_panel() -> Self {
        Self {
            kind: BufferKind::SidePanel,
            ..Self::default()
        }
    }

    /// The conversation view of reading mode's Ask column: its answers' citation markers are
    /// coloured as in the reading column.
    pub fn for_ask() -> Self {
        Self {
            kind: BufferKind::Ask,
            ..Self::default()
        }
    }

    /// Text added to the pane title, after the agent's.
    pub fn set_heading_suffix(&mut self, suffix: Option<String>) {
        self.heading_suffix = suffix;
    }

    /// Shows no agent (the side panel when its last side chat closes).
    pub fn clear(&mut self) {
        self.park_chat();
        self.previous = self.agent.take();
        self.layout = Layout::default();
    }

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

    /// Starts typing to `agent`, picking up its unsent draft.
    pub fn begin_chat(&mut self, agent: AgentId) {
        self.show(agent.clone());
        let view = self.views.entry(agent).or_default();
        let mut chat = TextInput::multiline(&view.draft);
        chat.set_cursor(view.draft_cursor.min(view.draft.len()));
        self.chat = Some(chat);
        self.images = view.draft_images.clone();
        // Paths are listed afresh each time: files come and go between chats.
        self.completion.reset();
    }

    /// How many images the message being typed carries.
    pub fn attachment_count(&self) -> usize {
        self.images.len()
    }

    /// Puts the core's answer to a Ctrl-V into `agent`'s message: at the cursor while its chat
    /// bar is open, else at the end of its draft (the user may have pressed Esc while the
    /// clipboard was read). Returns a notice for the status bar, if any.
    pub fn receive_paste(&mut self, agent: &AgentId, paste: Paste) -> Option<String> {
        let live = self.chat.is_some() && self.agent.as_ref() == Some(agent);
        let full = format!("at most {MAX_ATTACHMENTS} images per message");
        match paste {
            Paste::Refused(notice) => Some(notice),
            Paste::Text(text) if live => {
                if let Some(chat) = self.chat.as_mut() {
                    chat.insert_str(&text);
                }
                None
            }
            Paste::Text(text) => {
                self.views
                    .entry(agent.clone())
                    .or_default()
                    .draft
                    .push_str(&text);
                None
            }
            Paste::Image(image) => {
                let images = if live {
                    &mut self.images
                } else {
                    &mut self.views.entry(agent.clone()).or_default().draft_images
                };
                if images.len() >= MAX_ATTACHMENTS {
                    return Some(full);
                }
                images.push(image);
                None
            }
        }
    }

    /// Whether the chat input is open.
    pub fn is_chatting(&self) -> bool {
        self.chat.is_some()
    }

    /// Drops the last image chip when the chat input is open and empty
    /// ([`Action::DropLastAttachment`]). Returns whether one was dropped.
    pub fn drop_last_attachment(&mut self) -> bool {
        let empty = self.chat.as_ref().is_some_and(|c| c.text().is_empty());
        empty && self.images.pop().is_some()
    }

    /// Applies a key to the chat input. `Enter` returns the prompt and its images to send;
    /// `Esc` keeps both as the agent's draft. While the `/` or `@` popup is open it takes its
    /// keys first; `commands` are the session's, for `/`.
    pub fn chat_key(
        &mut self,
        key: KeyEvent,
        commands: &[SlashCommand],
    ) -> Option<(AgentId, String, Vec<Attachment>)> {
        let chat = self.chat.as_mut()?;
        let agent = self.agent.clone()?;
        let outcome = match self.completion.key(key) {
            KeyResult::Handled => return None,
            KeyResult::Accept(accept) => {
                chat.replace(accept.range, &accept.text);
                InputOutcome::Edited
            }
            KeyResult::Pass => chat.handle(key),
        };
        if outcome == InputOutcome::Edited {
            let (text, cursor) = (chat.text().to_string(), chat.cursor());
            if let Some(dir) = self.completion.update(&text, cursor, commands) {
                self.wants_paths = Some(dir);
            }
        }
        let chat = self.chat.as_mut()?;
        match outcome {
            InputOutcome::Submit => {
                let text = chat.text().trim().to_string();
                self.chat = None;
                let images = std::mem::take(&mut self.images);
                let view = self.views.entry(agent.clone()).or_default();
                if text.is_empty() && images.is_empty() {
                    return None;
                }
                view.draft.clear();
                view.draft_images.clear();
                view.follow = true;
                Some((agent, text, images))
            }
            InputOutcome::Cancel => {
                self.park_chat();
                None
            }
            InputOutcome::Edited | InputOutcome::Ignored => None,
        }
    }

    /// Types `text` at the chat input's cursor (a command picked with `:skills`).
    pub fn insert_text(&mut self, text: &str) {
        if let Some(chat) = self.chat.as_mut() {
            chat.insert_str(text);
        }
    }

    /// A path listing the `@` popup is waiting for: the agent and directory to ask about.
    pub fn take_path_request(&mut self) -> Option<(AgentId, Option<String>)> {
        let dir = self.wants_paths.take()?;
        Some((self.agent.clone()?, dir))
    }

    /// Takes a path listing from the core, if it is for the chat bar now open.
    pub fn receive_paths(
        &mut self,
        agent: &AgentId,
        dir: Option<String>,
        paths: Vec<String>,
        commands: &[SlashCommand],
    ) {
        if self.agent.as_ref() != Some(agent) {
            return;
        }
        let Some(chat) = &self.chat else { return };
        let (text, cursor) = (chat.text().to_string(), chat.cursor());
        self.completion.receive(dir, paths);
        if let Some(dir) = self.completion.update(&text, cursor, commands) {
            self.wants_paths = Some(dir);
        }
    }

    /// Closes the chat input, keeping its text, cursor and images as the agent's draft.
    pub fn park_chat(&mut self) {
        let (Some(chat), Some(agent)) = (self.chat.take(), self.agent.clone()) else {
            return;
        };
        let view = self.views.entry(agent).or_default();
        view.draft = chat.text().to_string();
        view.draft_cursor = chat.cursor();
        view.draft_images = std::mem::take(&mut self.images);
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

    /// The `/` or `@` popup, along the bottom of `body`, just over the chat bar's rule.
    fn render_popup(&self, frame: &mut Frame, body: Rect, cx: &ViewCx) {
        if !self.completion.is_open() {
            return;
        }
        let theme = cx.theme;
        let (rows, selected) = self.completion.rows();
        let width = usize::from(body.width);
        let lines: Vec<Line<'static>> = if rows.is_empty() {
            let note = if self.completion.waiting() {
                "  listing files…"
            } else {
                "  nothing matches · Esc closes"
            };
            vec![Line::from(Span::styled(note, theme.dim()))]
        } else {
            // Keep the selected row in view within the window of rows shown.
            let first = (selected + 1).saturating_sub(complete::MAX_ROWS);
            rows.iter()
                .enumerate()
                .skip(first)
                .take(complete::MAX_ROWS)
                .map(|(i, row)| {
                    let label = format!("  {}", row.label);
                    let room = width.saturating_sub(label.chars().count() + 2);
                    let detail = if row.detail.is_empty() || room < 8 {
                        String::new()
                    } else {
                        format!("  {}", fmt::truncate(&row.detail, room))
                    };
                    let line = Line::from(vec![
                        Span::styled(label, theme.accent()),
                        Span::styled(detail, theme.dim()),
                    ]);
                    if i == selected {
                        line.style(theme.selected())
                    } else {
                        line
                    }
                })
                .collect()
        };
        let height = (lines.len() as u16).min(body.height);
        let area = Rect {
            y: body.y + body.height - height,
            height,
            ..body
        };
        frame.render_widget(Clear, area);
        for (n, line) in lines.into_iter().take(usize::from(height)).enumerate() {
            let rect = Rect {
                y: area.y + n as u16,
                height: 1,
                ..area
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
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

/// The chip row(s) for pending images, indented under the `› ` prompt and wrapped to `width`.
fn chip_rows(
    images: &[Attachment],
    width: usize,
    theme: &crate::theme::Theme,
) -> Vec<Line<'static>> {
    let chips: Vec<Span<'static>> = images
        .iter()
        .enumerate()
        .map(|(i, a)| Span::styled(fmt::image_chip(i + 1, a.size), theme.chip()))
        .collect();
    let mut rows = Vec::new();
    for line in crate::render::transcript::chip_lines(chips, width) {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(line.spans);
        rows.push(Line::from(spans));
    }
    rows
}

impl Buffer for ConversationBuffer {
    fn kind(&self) -> BufferKind {
        self.kind
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
                parts.extend(self.heading_suffix.clone());
                parts.join(" · ")
            }
            None => "no agent".into(),
        };
        // The chat bar is a pane of its own under the conversation, like the inbox and the
        // tree: bordered, titled, and the one in the focus colour while you type (ADR 0010).
        let chatting = self.chat.is_some();
        let pad = crate::theme::CHAT_PADDING.x;
        // Border and the pane's own padding column on each side, then the chat padding.
        let chat_width = area.width.saturating_sub(4 + 2 * pad);
        let text_width = usize::from(chat_width.saturating_sub(2));
        let input_rows = self.chat.as_mut().map_or(1, |c| {
            c.set_wrap_width(text_width);
            c.rows(text_width).len().clamp(1, MAX_INPUT_ROWS)
        });
        let chips = if chatting && !self.images.is_empty() {
            chip_rows(&self.images, text_width, theme)
        } else {
            Vec::new()
        };
        let chat_height = (2 + chips.len() as u16 + input_rows as u16).min(area.height / 2);
        let conversation_area = Rect {
            height: area.height - chat_height,
            ..area
        };
        let chat_area = Rect {
            y: area.y + conversation_area.height,
            height: chat_height,
            ..area
        };

        let reading = focused && !chatting;
        let inner = pane(
            frame,
            conversation_area,
            title(&heading, reading, theme),
            reading,
            theme,
        );
        let body = crate::theme::CHAT_PADDING.shrink(inner);

        let typing = focused && chatting;
        let name = agent.map_or("no agent", |a| a.name.as_str());
        let chat_title = if chatting {
            format!("chat › {name} · ⏎ send · ^k read")
        } else {
            format!("chat › {name}")
        };
        let chat_inner = pane(
            frame,
            chat_area,
            title(&chat_title, typing, theme),
            typing,
            theme,
        );
        let chat_inner = Rect {
            x: chat_inner.x + pad.min(chat_inner.width / 2),
            width: chat_inner.width.saturating_sub(2 * pad),
            ..chat_inner
        };
        let chip_area = Rect {
            height: (chips.len() as u16).min(chat_inner.height),
            ..chat_inner
        };
        let input_area = Rect {
            y: chip_area.y + chip_area.height,
            height: chat_inner.height - chip_area.height,
            ..chat_inner
        };
        for (n, line) in chips
            .into_iter()
            .take(usize::from(chip_area.height))
            .enumerate()
        {
            let rect = Rect {
                y: chip_area.y + n as u16,
                height: 1,
                ..chip_area
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        if chatting {
            self.render_input(frame, input_area, cx);
        } else if input_area.height > 0 {
            let hint = Rect {
                height: 1,
                ..input_area
            };
            // A draft left with Ctrl-k or Esc stays in view, so it is clear what `i` goes back to.
            let draft = self
                .agent
                .as_ref()
                .and_then(|a| self.views.get(a))
                .map(|v| v.draft.lines().next().unwrap_or("").trim())
                .unwrap_or("");
            let text = if draft.is_empty() && self.kind == BufferKind::Ask {
                "› i to ask · ]s [s cite · gd open · dd close".to_string()
            } else if draft.is_empty() && self.kind == BufferKind::SidePanel {
                "› i or ^j to type · ⇥ next chat · dd close · ␣? hide".to_string()
            } else if draft.is_empty() {
                "› i or ^j to type · ]] [[ turns · zc zo fold · gg G".to_string()
            } else {
                format!("› {draft} · i or ^j to go on typing")
            };
            let text = fmt::truncate(&text, usize::from(hint.width));
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
        // Views of agents that are gone (archived, removed) go with them.
        self.views
            .retain(|id, _| *id == agent_id || cx.snap.agents.iter().any(|a| a.id == *id));
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
        if self.kind == BufferKind::Ask {
            built.lines = crate::render::citations::colour(built.lines, theme);
        }
        if older.is_some() {
            let note = Line::from(Span::styled(
                "↑ older history · k or gg at the top loads it",
                theme.dim(),
            ));
            // The transcript already opens with a blank row before its first header.
            built.lines.insert(0, note);
            for turn in &mut built.turns {
                turn.start_line += 1;
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
        if self.chat.is_some() {
            self.render_popup(frame, body, cx);
        }
        self.layout = Layout {
            lines: built.lines,
            turns: built.turns,
            max_top,
            page: height,
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
            // A page keeps two rows of the last one in view, to read on from.
            Action::PageDown => self.scroll_to(top + self.layout.page.saturating_sub(2).max(1)),
            Action::PageUp => {
                self.scroll_to(top.saturating_sub(self.layout.page.saturating_sub(2).max(1)));
                self.maybe_load_older(effects);
            }
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

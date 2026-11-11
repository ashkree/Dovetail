//! The side panel (decided 2026-11-11, refining ADR 0011 and ADR 0017): side chats in the side
//! column, beside the coordinator's conversation. It shows one side chat at a time as a
//! conversation with its own chat bar; `Tab` and `Shift-Tab` switch between them and `dd`
//! closes the one shown. Later, tangents and the grilling helper show here too.

use dovetail_api::{Agent, AgentId, AgentKind, Command, Snapshot};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Buffer, ConversationBuffer, Effect, ViewCx, pane, title};
use crate::action::Action;
use crate::region::BufferKind;

/// What the panel says when it has nothing to show.
const EMPTY: &str = "No side chats. ␣sq or :ask <question> starts one; ␣? hides this panel.";

/// The agents the panel switches between, in the snapshot's order (oldest first).
pub fn side_chats(snap: &Snapshot) -> Vec<&Agent> {
    snap.agents
        .iter()
        .filter(|a| a.kind == AgentKind::SideChat)
        .collect()
}

/// The side panel buffer: a conversation view kept on a side chat.
pub struct SidePanelBuffer {
    chat: ConversationBuffer,
    /// Where the shown chat stood among the side chats, so closing it shows its neighbour.
    index: usize,
}

impl Default for SidePanelBuffer {
    fn default() -> Self {
        Self {
            chat: ConversationBuffer::for_panel(),
            index: 0,
        }
    }
}

impl SidePanelBuffer {
    /// The side chat shown.
    pub fn agent(&self) -> Option<&AgentId> {
        self.chat.agent()
    }

    /// The conversation view inside the panel, with its chat bar.
    pub fn conversation(&self) -> &ConversationBuffer {
        &self.chat
    }

    /// The conversation view inside the panel, with its chat bar.
    pub fn conversation_mut(&mut self) -> &mut ConversationBuffer {
        &mut self.chat
    }

    /// Shows a side chat.
    pub fn show(&mut self, agent: AgentId) {
        self.chat.park_chat();
        self.chat.show(agent);
    }

    /// Keeps the panel on a live side chat: when the one shown is gone (closed), the one that
    /// took its place, or the last; nothing when there are none. Returns whether it changed.
    pub fn settle(&mut self, snap: &Snapshot) -> bool {
        let chats = side_chats(snap);
        let shown = self.agent().cloned();
        if let Some(i) = shown
            .as_ref()
            .and_then(|id| chats.iter().position(|a| &a.id == id))
        {
            self.index = i;
            return false;
        }
        let next = chats
            .get(self.index.min(chats.len().saturating_sub(1)))
            .map(|a| a.id.clone());
        match next {
            Some(id) => {
                self.show(id);
                true
            }
            None => {
                self.chat.clear();
                shown.is_some()
            }
        }
    }

    /// Shows the next (`delta` 1) or previous (-1) side chat, wrapping around.
    fn step(&mut self, snap: &Snapshot, delta: isize, effects: &mut Vec<Effect>) {
        let chats = side_chats(snap);
        if chats.len() < 2 {
            effects.push(Effect::Note(
                "one side chat: ␣sq or :ask starts another".into(),
            ));
            return;
        }
        let len = chats.len() as isize;
        let at = self
            .agent()
            .and_then(|id| chats.iter().position(|a| &a.id == id))
            .unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(len) as usize;
        self.index = next;
        let id = chats[next].id.clone();
        self.show(id);
    }

    /// "2/3 · ⇥ next" for the pane title, when there is more than one.
    fn position(&self, snap: &Snapshot) -> Option<String> {
        let chats = side_chats(snap);
        let at = self
            .agent()
            .and_then(|id| chats.iter().position(|a| &a.id == id))?;
        (chats.len() > 1).then(|| format!("{}/{} ⇥", at + 1, chats.len()))
    }
}

impl Buffer for SidePanelBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::SidePanel
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        if self.agent().is_none() {
            let theme = cx.theme;
            let inner = pane(
                frame,
                area,
                title("side chats", focused, theme),
                focused,
                theme,
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(EMPTY, theme.dim())))
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
        let position = self.position(cx.snap);
        self.chat.set_heading_suffix(position);
        self.chat.render(cx, frame, area, focused);
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        match action {
            Action::NextSideChat => self.step(cx.snap, 1, effects),
            Action::PrevSideChat => self.step(cx.snap, -1, effects),
            Action::CloseSideChat => match self.agent() {
                Some(agent) => effects.push(Effect::Command(Command::CloseSlice {
                    agent: agent.clone(),
                })),
                None => effects.push(Effect::Note("no side chat to close".into())),
            },
            _ => return self.chat.on_action(action, cx, effects),
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        self.chat.search_rows(cx)
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        self.chat.current_row(cx)
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, effects: &mut Vec<Effect>) {
        self.chat.goto_row(row, cx, effects);
    }
}

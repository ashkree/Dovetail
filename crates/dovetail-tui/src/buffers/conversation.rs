//! One agent's conversation. M2 draws plain text only; M3 adds the reading column, markdown
//! and scrolling.

use dovetail_api::{AgentId, EntryKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Buffer, Effect, ViewCx, footer, pane, status_glyph, title};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;

/// The conversation buffer.
#[derive(Default)]
pub struct ConversationBuffer {
    agent: Option<AgentId>,
}

impl ConversationBuffer {
    /// The agent shown.
    pub fn agent(&self) -> Option<&AgentId> {
        self.agent.as_ref()
    }

    /// Shows another agent.
    pub fn show(&mut self, agent: AgentId) {
        self.agent = Some(agent);
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
                let mut parts = vec![
                    a.name.clone(),
                    format!("{glyph} {}", super::status_word(a.status)),
                ];
                if let Some(mb) = a.memory_mb {
                    parts.push(fmt::memory(mb));
                }
                parts.push(fmt::cost(a.cost_usd));
                parts.join(" · ")
            }
            None => "no agent".into(),
        };
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        let width = usize::from(inner.width);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if let Some(tail) = agent.and_then(|a| cx.snap.conversations.get(&a.id)) {
            for entry in &tail.entries {
                let (label, style) = match entry.kind {
                    EntryKind::User => ("you", theme.accent()),
                    EntryKind::Assistant => {
                        (agent.map_or("agent", |a| a.name.as_str()), theme.accent())
                    }
                    EntryKind::ToolSummary => ("", theme.dim()),
                    EntryKind::Notice => ("", theme.dim()),
                };
                if !label.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{label} · {}", fmt::hhmm(entry.at_ms, cx.tz)),
                        style,
                    )));
                }
                let body_style = if label.is_empty() {
                    theme.dim()
                } else {
                    theme.body()
                };
                for text in fmt::wrap(&entry.text, width) {
                    lines.push(Line::from(Span::styled(text, body_style)));
                }
                lines.push(Line::default());
            }
        }
        let height = usize::from(inner.height.saturating_sub(u16::from(inner.height >= 5)));
        let skip = lines.len().saturating_sub(height);
        for (n, line) in lines.into_iter().skip(skip).enumerate() {
            let rect = Rect {
                y: inner.y + n as u16,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        footer(frame, inner, &["› i to chat"], theme);
    }

    fn on_action(&mut self, _action: Action, _cx: &ViewCx, _effects: &mut Vec<Effect>) -> bool {
        false
    }
}

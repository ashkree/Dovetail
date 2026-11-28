//! Reading mode's Ask column (ADR 0018): the open report's Ask session as a conversation with
//! its own chat bar. It follows the research task of the document in the reading column; with
//! no session yet it says how to start one (`i` in the reading column).
//!
//! `]s`/`[s` pick a citation id in the latest answer (shown in the status bar), and `gd`/`gx`
//! open it as in the reading column, so the answer's markers resolve to the same findings and
//! sources.

use dovetail_api::{
    Command, Snapshot,
    agents::agent::{Agent, AgentId, AgentKind},
    agents::conversation::EntryKind,
    tracking::task::TaskId,
};
use dovetail_api::{agents::agent::AgentCommand, research::Evidence};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Paragraph, Wrap};

use super::reader::ReadTarget;
use super::{Buffer, ConversationBuffer, Effect, ViewCx, pane, title};
use crate::action::Action;
use crate::region::BufferKind;
use crate::render::citations;

/// Task `task`'s Ask session in the snapshot, if there is one.
pub fn ask_agent(snap: &Snapshot, task: TaskId) -> Option<&Agent> {
    snap.agents
        .iter()
        .find(|a| a.kind == AgentKind::Ask && a.task == Some(task))
}

/// The Ask column.
pub struct AskBuffer {
    chat: ConversationBuffer,
    task: Option<TaskId>,
    /// The citation picked in the latest answer, by its place there.
    cite: Option<usize>,
}

impl Default for AskBuffer {
    fn default() -> Self {
        Self {
            chat: ConversationBuffer::for_ask(),
            task: None,
            cite: None,
        }
    }
}

impl AskBuffer {
    /// The Ask session shown.
    pub fn agent(&self) -> Option<&AgentId> {
        self.chat.agent()
    }

    /// The research task it follows.
    pub fn task(&self) -> Option<TaskId> {
        self.task
    }

    /// The conversation view, with its chat bar.
    pub fn conversation(&self) -> &ConversationBuffer {
        &self.chat
    }

    /// The conversation view, with its chat bar.
    pub fn conversation_mut(&mut self) -> &mut ConversationBuffer {
        &mut self.chat
    }

    /// Follows the reading column's task: shows its Ask session, or none. Returns whether the
    /// session shown changed.
    pub fn follow(&mut self, task: Option<TaskId>, snap: &Snapshot) -> bool {
        self.task = task;
        let want = task.and_then(|t| ask_agent(snap, t)).map(|a| a.id.clone());
        if want.as_ref() == self.chat.agent() {
            return false;
        }
        self.cite = None;
        match want {
            Some(id) => {
                self.chat.park_chat();
                self.chat.show(id);
            }
            None => self.chat.clear(),
        }
        true
    }

    /// Citation ids in the session's latest answer, in order.
    fn answer_citations(&self, snap: &Snapshot) -> Vec<Evidence> {
        let Some(agent) = self.agent() else {
            return Vec::new();
        };
        let Some(entry) = snap.conversations.get(agent).and_then(|t| {
            t.entries
                .iter()
                .rev()
                .find(|e| e.kind == EntryKind::Assistant)
        }) else {
            return Vec::new();
        };
        let text = &entry.text;
        let chars: Vec<char> = text.chars().collect();
        citations::markers(text)
            .into_iter()
            .flat_map(|(a, b)| {
                let inner: String = chars[a + 1..b - 1].iter().collect();
                inner
                    .split(',')
                    .filter_map(|p| Evidence::parse(p.trim()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn select(&mut self, forward: bool, snap: &Snapshot, effects: &mut Vec<Effect>) {
        let cites = self.answer_citations(snap);
        if cites.is_empty() {
            effects.push(Effect::Note("the latest answer cites nothing".into()));
            return;
        }
        let next = match self.cite.filter(|&i| i < cites.len()) {
            Some(i) if forward => (i + 1) % cites.len(),
            Some(i) => (i + cites.len() - 1) % cites.len(),
            None if forward => 0,
            None => cites.len() - 1,
        };
        self.cite = Some(next);
        effects.push(Effect::Note(format!(
            "[{}] {}/{} in the answer · gd opens it · gx opens a web source",
            cites[next],
            next + 1,
            cites.len()
        )));
    }
}

impl Buffer for AskBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Ask
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        if self.agent().is_some() {
            self.chat.render(cx, frame, area, focused);
            return;
        }
        let theme = cx.theme;
        let inner = pane(frame, area, title("ask", focused, theme), focused, theme);
        let has_report = |t: TaskId| {
            cx.snap
                .research
                .iter()
                .any(|r| r.task == t && r.report_ms.is_some())
        };
        let text = match self.task {
            Some(t) if has_report(t) => format!(
                "› i to ask about #{}'s report. A read-only session answers from the report and \
                 its sources, and cites them as the report does.",
                t.0
            ),
            Some(t) => format!(
                "#{} has no report yet, so there is nothing to ask about.",
                t.0
            ),
            None => "Open a report in the reading column to ask about it.".to_string(),
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme.dim())).wrap(Wrap { trim: true }),
            inner,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        match action {
            Action::CloseAsk => match self.agent() {
                Some(agent) => {
                    effects.push(Effect::Command(Command::Agent(AgentCommand::CloseSlice {
                        agent: agent.clone(),
                    })))
                }
                None => effects.push(Effect::Note("no Ask session to close".into())),
            },
            Action::NextCitation => self.select(true, cx.snap, effects),
            Action::PrevCitation => self.select(false, cx.snap, effects),
            Action::OpenCitation | Action::BrowseCitation => {
                let cites = self.answer_citations(cx.snap);
                let evidence = self.cite.and_then(|i| cites.get(i)).or(cites.first());
                let Some(task) = self.task else {
                    effects.push(Effect::Note("no report open".into()));
                    return true;
                };
                effects.push(Effect::Cite {
                    task: Some(task),
                    evidence: evidence.copied(),
                    browse: action == Action::BrowseCitation,
                    from: ReadTarget::Research {
                        task,
                        what: dovetail_api::research::ResearchWhat::Report,
                    },
                });
            }
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

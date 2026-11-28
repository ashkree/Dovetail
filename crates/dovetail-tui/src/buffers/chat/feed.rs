//! The Activity feed and the message timeline: newest first, one row each, `Enter` opens the
//! agent involved.

use dovetail_api::{Snapshot, activity::ActivityKind, agents::agent::AgentId};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::super::{Buffer, Effect, ListCursor, ViewCx, body_height, footer, pane, spread, title};
use crate::fmt;
use crate::keymap::action::Action;
use crate::layout::region::BufferKind;

/// One row of a feed.
struct Row {
    id: u64,
    /// `HH:MM` and the rest of the line.
    time: String,
    body: String,
    agent: Option<AgentId>,
}

impl Row {
    fn text(&self) -> String {
        format!("{} {}", self.time, self.body)
    }
}

/// What a feed draws: its heading, its rows and what to say when it has none.
struct FeedView<'a> {
    heading: &'a str,
    rows: &'a [Row],
    empty: &'a str,
}

/// Selection and scroll shared by both feeds.
#[derive(Default)]
struct Feed {
    cursor: ListCursor<u64>,
}

impl Feed {
    fn resolve(&self, rows: &[Row]) -> Option<usize> {
        self.cursor.resolve(rows, |r, k| r.id == *k)
    }

    fn select(&mut self, rows: &[Row], index: usize) {
        self.cursor.select_index(rows, index, |r| r.id);
    }

    fn render(
        &mut self,
        cx: &ViewCx,
        frame: &mut Frame,
        area: Rect,
        focused: bool,
        view: FeedView,
    ) {
        let FeedView {
            heading,
            rows,
            empty,
        } = view;
        let theme = cx.theme;
        let inner = pane(frame, area, title(heading, focused, theme), focused, theme);
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(empty.to_string(), theme.dim())),
                inner,
            );
            return;
        }
        let selected = self.cursor.sync(rows, |r, k| r.id == *k, |r| r.id);
        let height = body_height(inner);
        if let Some(i) = selected {
            self.cursor.scroll_into_view(i, rows.len(), height);
        } else {
            self.cursor.clamp_offset(rows.len(), height);
        }
        let width = usize::from(inner.width);
        for (n, (i, row)) in rows
            .iter()
            .enumerate()
            .skip(self.cursor.offset())
            .take(height)
            .enumerate()
        {
            let left = vec![
                Span::styled(format!("{}  ", row.time), theme.dim()),
                Span::styled(
                    fmt::truncate(&row.body, width.saturating_sub(7)),
                    theme.body(),
                ),
            ];
            let rect = Rect {
                y: inner.y + n as u16,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(spread(left, vec![], width)), rect);
            if focused && selected == Some(i) {
                frame.buffer_mut().set_style(rect, theme.selected());
            }
        }
        footer(
            frame,
            inner,
            &["⏎ open agent · / search · esc back", "⏎ open · esc back"],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, rows: &[Row], effects: &mut Vec<Effect>) -> bool {
        if self
            .cursor
            .handle_nav(action, rows, |r, k| r.id == *k, |r| r.id)
        {
            return true;
        }
        match action {
            Action::FeedOpen => match self.resolve(rows).and_then(|i| rows[i].agent.clone()) {
                Some(agent) => effects.push(Effect::OpenAgent(agent)),
                None => effects.push(Effect::Note("no agent on this row".into())),
            },
            _ => return false,
        }
        true
    }
}

fn name_of(snap: &Snapshot, id: &AgentId) -> String {
    snap.agents
        .iter()
        .find(|a| &a.id == id)
        .map_or_else(|| id.0.clone(), |a| a.name.clone())
}

fn activity_rows(cx: &ViewCx) -> Vec<Row> {
    cx.snap
        .activity
        .iter()
        .rev()
        .map(|item| {
            let tag = match item.kind {
                ActivityKind::Pr => "PR  ",
                ActivityKind::Ci => "CI  ",
                ActivityKind::Message => "msg ",
                ActivityKind::Lifecycle => "·   ",
                ActivityKind::Permission => "perm",
                ActivityKind::Task => "task",
            };
            let who = item
                .agent
                .as_ref()
                .map_or(String::new(), |a| format!("{}  ", name_of(cx.snap, a)));
            Row {
                id: item.id,
                time: fmt::hhmm(item.at_ms, cx.tz),
                body: format!("{tag}{who}{}", item.text),
                agent: item.agent.clone(),
            }
        })
        .collect()
}

fn message_rows(cx: &ViewCx) -> Vec<Row> {
    cx.snap
        .messages
        .iter()
        .rev()
        .map(|m| Row {
            id: m.id,
            time: fmt::hhmm(m.at_ms, cx.tz),
            body: format!(
                "{} → {}  {}",
                name_of(cx.snap, &m.from),
                name_of(cx.snap, &m.to),
                fmt::first_line(&m.text)
            ),
            agent: Some(m.from.clone()),
        })
        .collect()
}

macro_rules! feed_buffer {
    ($name:ident, $kind:expr, $rows:ident, $heading:expr, $empty:expr) => {
        /// A feed buffer.
        #[derive(Default)]
        pub struct $name(Feed);

        impl Buffer for $name {
            fn kind(&self) -> BufferKind {
                $kind
            }

            fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
                let rows = $rows(cx);
                let view = FeedView {
                    heading: $heading,
                    rows: &rows,
                    empty: $empty,
                };
                self.0.render(cx, frame, area, focused, view);
            }

            fn on_action(
                &mut self,
                action: Action,
                cx: &ViewCx,
                effects: &mut Vec<Effect>,
            ) -> bool {
                self.0.on_action(action, &$rows(cx), effects)
            }

            fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
                $rows(cx).iter().map(Row::text).collect()
            }

            fn current_row(&self, cx: &ViewCx) -> usize {
                self.0.resolve(&$rows(cx)).unwrap_or(0)
            }

            fn goto_row(&mut self, row: usize, cx: &ViewCx, _effects: &mut Vec<Effect>) {
                let rows = $rows(cx);
                if row < rows.len() {
                    self.0.select(&rows, row);
                }
            }
        }
    };
}

feed_buffer!(
    ActivityBuffer,
    BufferKind::Activity,
    activity_rows,
    "activity · today",
    "nothing has happened yet · esc to go back"
);
feed_buffer!(
    MessagesBuffer,
    BufferKind::Messages,
    message_rows,
    "messages between agents",
    "no messages yet · esc to go back"
);

//! The tangents list (ADR 0017, amended 2026-11-11): Tangents' tangents in the main region, new
//! ones first, to triage. `:tangents` opens it any time; the morning's "new tangents" item opens
//! it too. Each can be read (`Enter`, reading mode), filed into a project (`f`), started as a
//! task in a project (`s`), attached to a task of this project (`a`) or dropped (`d`).

use dovetail_api::{
    Command, Snapshot,
    agents::agent::AgentStatus,
    agents::tangent::{TangentCommand, TangentSummary, tangent_agent},
    tracking::task::{TaskId, TaskStatus},
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::{Buffer, Effect, ListCursor, ViewCx, clip_spans, footer, pane, title};
use crate::fmt;
use crate::keymap::action::Action;
use crate::layout::region::BufferKind;
use crate::overlay::switcher::TANGENT_MARKER;

/// What the list says when Tangents is empty.
const EMPTY: &str = "No tangents. T in the side panel (␣?) or :tangent <question> asks one: a \
                     question that does not belong to this project, answered with sources and \
                     kept in Tangents.";

/// The tangents in list order: the new ones (not triaged) first, then the rest, oldest first.
pub fn ordered(snap: &Snapshot) -> Vec<&TangentSummary> {
    let mut out: Vec<&TangentSummary> = snap.tangents.iter().collect();
    out.sort_by_key(|t| (!t.is_new(), t.task.id));
    out
}

/// One word for where a tangent stands, as the list shows it.
pub fn state_word(snap: &Snapshot, t: &TangentSummary) -> &'static str {
    let answering = snap
        .agents
        .iter()
        .any(|a| a.id == tangent_agent(t.task.id) && a.status == AgentStatus::Working);
    if answering {
        "answering"
    } else if t.is_new() {
        "new"
    } else if t.task.status == TaskStatus::Done {
        "started"
    } else {
        "kept"
    }
}

/// `2 findings · 5 sources`, or that nothing is filed yet.
pub fn counts(t: &TangentSummary) -> String {
    match &t.research {
        Some(r) => format!(
            "{} finding{} · {} source{}",
            r.findings,
            if r.findings == 1 { "" } else { "s" },
            r.sources,
            if r.sources == 1 { "" } else { "s" }
        ),
        None => "no answer yet".into(),
    }
}

/// The tangents list buffer.
#[derive(Default)]
pub struct TangentsBuffer {
    /// The selected tangent, kept by id so a live update never moves the cursor.
    cursor: ListCursor<TaskId>,
}

impl TangentsBuffer {
    /// The selected tangent's task in Tangents.
    pub fn selected(&self, snap: &Snapshot) -> Option<TaskId> {
        let list = ordered(snap);
        self.index(&list).map(|i| list[i].task.id)
    }

    fn index(&self, list: &[&TangentSummary]) -> Option<usize> {
        self.cursor.resolve(list, |t, k| t.task.id == *k)
    }
}

impl Buffer for TangentsBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Tangents
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let list = ordered(cx.snap);
        let new = list.iter().filter(|t| t.is_new()).count();
        let inner = pane(
            frame,
            area,
            title(
                &format!("{TANGENT_MARKER} tangents · {new} new of {}", list.len()),
                focused,
                theme,
            ),
            focused,
            theme,
        );
        let width = usize::from(inner.width);
        if list.is_empty() {
            let lines = fmt::wrapped_lines(EMPTY, width, theme.dim());
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }
        let at = self
            .cursor
            .sync(&list, |t, k| t.task.id == *k, |t| t.task.id);
        // The rows, then a blank line and the selected tangent's notes, then the footer.
        let height = usize::from(inner.height.saturating_sub(1));
        let rows_height = (height / 2).max(1).min(list.len().max(1));
        if let Some(i) = at {
            self.cursor.scroll_into_view(i, list.len(), rows_height);
        }
        for (n, t) in list
            .iter()
            .enumerate()
            .skip(self.cursor.offset())
            .take(rows_height)
        {
            let word = state_word(cx.snap, t);
            let color = match word {
                "new" => theme.needs_you,
                "answering" => theme.working,
                _ => theme.dim,
            };
            let spans = vec![
                Span::styled(if Some(n) == at { "▸ " } else { "  " }, theme.dim()),
                Span::styled(format!("{word:<9} "), theme.fg(color)),
                Span::styled(t.task.title.clone(), theme.body()),
                Span::styled(format!("  {}", counts(t)), theme.dim()),
            ];
            let mut line = Line::from(clip_spans(spans, width));
            if Some(n) == at && focused {
                line = line.style(theme.selected());
            }
            let rect = Rect {
                y: inner.y + (n - self.cursor.offset()) as u16,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        if let Some(t) = at.map(|i| list[i]) {
            let top = inner.y + rows_height as u16 + 1;
            let room = inner.height.saturating_sub(rows_height as u16 + 2);
            if room > 0 && !t.task.notes.trim().is_empty() {
                let mut lines = fmt::wrapped_lines(t.task.notes.trim(), width, theme.dim());
                lines.truncate(usize::from(room));
                let rect = Rect {
                    y: top,
                    height: room,
                    ..inner
                };
                frame.render_widget(Paragraph::new(lines), rect);
            }
        }
        footer(
            frame,
            inner,
            &[
                "⏎ read · f file into a project · s start a task · a attach to a task · d drop · \
                 esc back",
                "⏎ read · f file · s start · a attach · d drop",
                "⏎ f s a d",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let list = ordered(cx.snap);
        if self
            .cursor
            .handle_nav(action, &list, |t, k| t.task.id == *k, |t| t.task.id)
        {
            return true;
        }
        match action {
            Action::PageDown => {
                self.cursor
                    .step(&list, 10, |t, k| t.task.id == *k, |t| t.task.id);
            }
            Action::PageUp => {
                self.cursor
                    .step(&list, -10, |t, k| t.task.id == *k, |t| t.task.id);
            }
            Action::TangentOpen
            | Action::TangentFile
            | Action::TangentStart
            | Action::AttachTangent
            | Action::TangentDrop => {
                let Some(t) = self.index(&list).map(|i| list[i]) else {
                    effects.push(Effect::Note(
                        "no tangents: :tangent <question> asks one".into(),
                    ));
                    return true;
                };
                let tangent = t.task.id;
                effects.push(match action {
                    Action::TangentOpen if t.research.is_none() => {
                        Effect::Note("no answer yet: it shows here when the turn ends".into())
                    }
                    Action::TangentOpen => Effect::ReadTangent(tangent),
                    Action::TangentFile => Effect::PickTangentProject {
                        tangent,
                        start: false,
                    },
                    Action::TangentStart => Effect::PickTangentProject {
                        tangent,
                        start: true,
                    },
                    Action::AttachTangent => Effect::AttachTangent(tangent),
                    _ => Effect::Confirm {
                        prompt: format!(
                            "Drop the tangent \"{}\"? It is archived in Tangents, not deleted.",
                            t.task.title
                        ),
                        then: vec![Command::Tangent(TangentCommand::Drop { tangent })],
                    },
                });
            }
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        ordered(cx.snap)
            .iter()
            .map(|t| format!("{} {}", t.task.title, t.task.notes))
            .collect()
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        self.index(&ordered(cx.snap)).unwrap_or(0)
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, _effects: &mut Vec<Effect>) {
        let list = ordered(cx.snap);
        self.cursor.select_index(&list, row, |t| t.task.id);
    }
}

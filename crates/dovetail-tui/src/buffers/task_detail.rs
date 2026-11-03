//! One task in full: title, fields, notes and the agents working on it. The decision log is
//! v1.1 (plan A4); notes are edited in `$EDITOR` from M5.

use dovetail_api::{Agent, Command, Snapshot, Task, TaskChange, TaskId};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{
    Buffer, Effect, ViewCx, bucket_label, footer, next_bucket, next_p_level, next_status, pane,
    status_glyph, status_label, status_word, title, tree::p_label,
};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;
use crate::theme::Theme;

/// The detail view of one task, as plain lines (shared with the picker's preview).
pub fn lines(snap: &Snapshot, task: &Task, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let label = |text: &str| Line::from(Span::styled(text.to_string(), theme.accent()));
    let blank = Line::default;

    out.push(Line::from(Span::styled(
        task.title.clone(),
        theme.body().add_modifier(ratatui::style::Modifier::BOLD),
    )));
    let mut fields = vec![
        task.p_level.map_or("no P-level", p_label).to_string(),
        bucket_label(task.bucket).to_string(),
        status_label(task.status).to_string(),
    ];
    if let Some(pr) = task.pr {
        fields.push(format!("PR #{pr}"));
    }
    out.push(Line::from(Span::styled(fields.join(" · "), theme.dim())));
    if let Some(parent) = task.follow_up_of {
        let name = snap
            .tasks
            .iter()
            .find(|t| t.id == parent)
            .map_or(String::new(), |t| format!(" {}", t.title));
        out.push(Line::from(Span::styled(
            format!("follow-up of #{}{name}", parent.0),
            theme.dim(),
        )));
    }
    if task.coordinator_created {
        out.push(Line::from(Span::styled(
            "filed by the coordinator",
            theme.fg(theme.bold),
        )));
    }
    if !task.deps.is_empty() {
        let deps: Vec<String> = task
            .deps
            .iter()
            .map(|d| {
                let title = snap
                    .tasks
                    .iter()
                    .find(|t| t.id == *d)
                    .map_or("", |t| t.title.as_str());
                format!("#{} {title}", d.0)
            })
            .collect();
        out.push(Line::from(Span::styled(
            format!("waits for {}", deps.join(", ")),
            theme.dim(),
        )));
    }

    out.push(blank());
    out.push(label("Notes"));
    out.push(blank());
    if task.notes.trim().is_empty() {
        out.push(Line::from(Span::styled("(no notes)", theme.dim())));
    } else {
        for text in fmt::wrap(&task.notes, width) {
            out.push(Line::from(Span::styled(text, theme.body())));
        }
    }

    out.push(blank());
    out.push(label("Agents"));
    out.push(blank());
    let agents: Vec<&Agent> = snap
        .agents
        .iter()
        .filter(|a| a.task == Some(task.id))
        .collect();
    if agents.is_empty() {
        out.push(Line::from(Span::styled("(none yet)", theme.dim())));
    }
    for agent in agents {
        let (glyph, color) = status_glyph(agent.status, theme);
        let kind = if agent.kind.is_session() {
            "◆"
        } else {
            "◇"
        };
        let mut spans = vec![
            Span::styled(glyph, theme.fg(color)),
            Span::raw(" "),
            Span::styled(kind, theme.fg(color)),
            Span::raw(" "),
            Span::styled(agent.name.clone(), theme.body()),
            Span::styled(format!("  {}", status_word(agent.status)), theme.dim()),
        ];
        if let Some(wt) = &agent.worktree {
            spans.push(Span::styled(format!("  ⎇ {}", wt.branch), theme.dim()));
        }
        out.push(Line::from(spans));
    }
    out
}

/// The task detail buffer.
#[derive(Default)]
pub struct TaskDetailBuffer {
    task: Option<TaskId>,
    top: usize,
    last_lines: usize,
}

impl TaskDetailBuffer {
    /// The task shown.
    pub fn task(&self) -> Option<TaskId> {
        self.task
    }

    /// Shows a task, from its first line.
    pub fn show(&mut self, task: TaskId) {
        if self.task != Some(task) {
            self.top = 0;
        }
        self.task = Some(task);
    }
}

impl Buffer for TaskDetailBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::TaskDetail
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let task = self
            .task
            .and_then(|id| cx.snap.tasks.iter().find(|t| t.id == id));
        let heading = task.map_or("task".to_string(), |t| format!("task · #{}", t.id.0));
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        let Some(task) = task else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "that task is gone · esc to go back",
                    theme.dim(),
                )),
                inner,
            );
            return;
        };
        let width = usize::from(inner.width).min(100);
        let all = lines(cx.snap, task, width, theme);
        let height = usize::from(inner.height.saturating_sub(u16::from(inner.height >= 5)));
        self.last_lines = all.len();
        self.top = self.top.min(all.len().saturating_sub(height));
        for (n, line) in all.into_iter().skip(self.top).take(height).enumerate() {
            let rect = Rect {
                y: inner.y + n as u16,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        footer(
            frame,
            inner,
            &[
                "s status · b bucket · p P-level · e notes · esc back",
                "s b p cycle · esc back",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let task = self
            .task
            .and_then(|id| cx.snap.tasks.iter().find(|t| t.id == id));
        let change = |c: TaskChange, t: &Task| {
            Effect::Command(Command::UpdateTask {
                id: t.id,
                patch: vec![c],
            })
        };
        match (action, task) {
            (Action::MoveDown, _) => {
                self.top = (self.top + 1).min(self.last_lines.saturating_sub(1))
            }
            (Action::MoveUp, _) => self.top = self.top.saturating_sub(1),
            (Action::GoTop, _) => self.top = 0,
            (Action::GoBottom, _) => self.top = self.last_lines.saturating_sub(1),
            (Action::TaskCycleStatus, Some(t)) => {
                effects.push(change(TaskChange::Status(next_status(t.status)), t));
            }
            (Action::TaskCycleBucket, Some(t)) => {
                effects.push(change(TaskChange::Bucket(next_bucket(t.bucket)), t));
            }
            (Action::TaskCyclePLevel, Some(t)) => {
                effects.push(change(TaskChange::PLevel(next_p_level(t.p_level)), t));
            }
            (Action::TaskEditNotes, _) => {
                effects.push(Effect::Note(
                    "editing notes in $EDITOR arrives with storage (M5)".into(),
                ));
            }
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        self.task
            .and_then(|id| cx.snap.tasks.iter().find(|t| t.id == id))
            .map(|t| {
                lines(cx.snap, t, 100, cx.theme)
                    .iter()
                    .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn current_row(&self, _cx: &ViewCx) -> usize {
        self.top
    }

    fn goto_row(&mut self, row: usize, _cx: &ViewCx, _effects: &mut Vec<Effect>) {
        self.top = row;
    }
}

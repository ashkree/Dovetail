//! One task in full: title, fields, related tasks, notes, the agents working on it and its
//! decision log (in v1 since 2026-11-03, ADR 0003). Notes are edited in `$EDITOR` from M5.

use dovetail_api::research::{ResearchContent, ResearchWhat};
use dovetail_api::{
    Agent, Command, Decision, DecisionAuthor, Event, Snapshot, Task, TaskChange, TaskId,
};
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::task_research::{self, Doc, Item, Loaded};
use super::{
    Buffer, Effect, ViewCx, bucket_label, footer, next_bucket, next_p_level, next_status, pane,
    scroll_to, status_glyph, status_label, status_word, title, tree::p_label,
};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;
use crate::render::{citations, markdown, reading_column};
use crate::theme::Theme;

/// The detail view of one task, as plain lines (shared with the picker's preview).
pub fn lines(
    snap: &Snapshot,
    task: &Task,
    width: usize,
    theme: &Theme,
    tz: &TimeZone,
) -> Vec<Line<'static>> {
    build(snap, task, width, theme, tz, None).lines
}

/// The detail's lines and the research rows `Enter` can open.
struct Built {
    lines: Vec<Line<'static>>,
    items: Vec<(usize, Item)>,
}

fn build(
    snap: &Snapshot,
    task: &Task,
    width: usize,
    theme: &Theme,
    tz: &TimeZone,
    loaded: Option<&Loaded>,
) -> Built {
    let mut items = Vec::new();
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

    if !task.related.is_empty() {
        out.push(blank());
        out.push(label("Related"));
        out.push(blank());
        for id in &task.related {
            let title = snap
                .tasks
                .iter()
                .find(|t| t.id == *id)
                .map_or("(gone)", |t| t.title.as_str());
            out.push(Line::from(vec![
                Span::styled("↔ ", theme.dim()),
                Span::styled(title.to_string(), theme.body()),
            ]));
        }
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

    let research = task_research::section(snap, task, loaded, theme, tz, width);
    items.extend(
        research
            .items
            .into_iter()
            .map(|(row, item)| (out.len() + row, item)),
    );
    out.extend(research.lines);

    out.push(blank());
    out.push(label("Decisions"));
    out.push(blank());
    let mut decisions: Vec<&Decision> = snap
        .decisions
        .iter()
        .filter(|d| d.task == task.id)
        .collect();
    decisions.sort_by_key(|d| std::cmp::Reverse((d.at_ms, d.id)));
    if decisions.is_empty() {
        out.push(Line::from(Span::styled(
            "(none yet · a to add one)",
            theme.dim(),
        )));
    }
    for d in decisions {
        decision_lines(snap, d, width, theme, tz, &mut out);
    }
    Built { lines: out, items }
}

/// One decision: who and when, the decision, why, and the options it beat. A superseded entry
/// is dimmed and struck through, and says what replaced it.
fn decision_lines(
    snap: &Snapshot,
    d: &Decision,
    width: usize,
    theme: &Theme,
    tz: &TimeZone,
    out: &mut Vec<Line<'static>>,
) {
    let replaced_by = snap
        .decisions
        .iter()
        .find(|later| later.supersedes == Some(d.id))
        .map(|later| later.id);
    let who = match &d.by {
        DecisionAuthor::User => "you".to_string(),
        DecisionAuthor::Agent(id) => snap
            .agents
            .iter()
            .find(|a| &a.id == id)
            .map_or_else(|| id.0.clone(), |a| a.name.clone()),
    };
    out.push(Line::from(Span::styled(
        format!("{} · {who} · #{}", fmt::when(d.at_ms, tz), d.id.0),
        theme.dim(),
    )));
    let style = if replaced_by.is_some() {
        theme.dim().add_modifier(Modifier::CROSSED_OUT)
    } else {
        theme.body().add_modifier(Modifier::BOLD)
    };
    for text in fmt::wrap(&d.text, width) {
        out.push(Line::from(Span::styled(text, style)));
    }
    if let Some(next) = replaced_by {
        out.push(Line::from(Span::styled(
            format!("superseded by #{}", next.0),
            theme.dim(),
        )));
    }
    if let Some(why) = &d.why {
        let why_style = if replaced_by.is_some() {
            theme.dim()
        } else {
            theme.body()
        };
        for text in fmt::wrap(why, width.saturating_sub(2)) {
            out.push(Line::from(Span::styled(format!("  {text}"), why_style)));
        }
    }
    for alt in &d.alternatives {
        let text = match &alt.why_not {
            Some(why_not) => format!("considered: {} — {why_not}", alt.option),
            None => format!("considered: {}", alt.option),
        };
        for line in fmt::wrap(&text, width.saturating_sub(2)) {
            out.push(Line::from(Span::styled(format!("  {line}"), theme.dim())));
        }
    }
    // The findings and sources a research decision rests on (ADR 0015).
    if !d.evidence.is_empty() {
        let ids: Vec<String> = d.evidence.iter().map(|e| format!("[{e}]")).collect();
        out.push(Line::from(Span::styled(
            format!("  evidence: {}", ids.join(" ")),
            theme.accent(),
        )));
    }
    out.push(Line::default());
}

/// The task detail buffer. It also reads: `Enter` on a research row swaps the detail for that
/// document in the reading column, `Esc` swaps it back (R3).
#[derive(Default)]
pub struct TaskDetailBuffer {
    task: Option<TaskId>,
    top: usize,
    last_lines: usize,
    last_height: usize,
    /// The research rows of the last draw, and the one `Enter` opens.
    items: Vec<(usize, Item)>,
    /// The research row picked with `Tab`; `None` until one is, and after the task changes.
    selected: Option<usize>,
    /// Scroll to show the selected row at the next draw (after `Tab`).
    follow: bool,
    /// The listing the core last answered with.
    loaded: Option<Loaded>,
    /// A document asked for and not yet delivered: task, what, pane title.
    awaiting: Option<(TaskId, ResearchWhat, String)>,
    /// The document being read, in place of the detail.
    reading: Option<Reading>,
    /// Solid background behind the reading column, as in the conversation.
    solid: bool,
}

/// A document open in the reading view.
struct Reading {
    doc: Doc,
    top: usize,
    /// The rendered lines and the column width they were rendered for.
    lines: Vec<Line<'static>>,
    width: usize,
    page: usize,
}

impl TaskDetailBuffer {
    /// The task shown.
    pub fn task(&self) -> Option<TaskId> {
        self.task
    }

    /// Shows a task, from its first line. A document being read is closed.
    pub fn show(&mut self, task: TaskId) {
        if self.task != Some(task) {
            self.top = 0;
            self.selected = None;
        }
        self.task = Some(task);
        self.reading = None;
        self.awaiting = None;
    }

    /// Whether a document is open in place of the detail.
    pub fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Closes the document and shows the detail again.
    pub fn close_reading(&mut self) {
        self.reading = None;
    }

    /// Mirrors the conversation's solid reading background.
    pub fn set_solid(&mut self, solid: bool) {
        self.solid = solid;
    }

    /// Notes that `what` was asked for, so its answer opens the reading view.
    pub fn await_document(&mut self, task: TaskId, what: ResearchWhat, title: String) {
        self.awaiting = Some((task, what, title));
    }

    /// Takes in a core answer. Returns whether a document arrived that should now be shown.
    pub fn on_event(&mut self, event: &Event) -> bool {
        match event {
            Event::ResearchLoaded {
                task,
                what,
                content,
            } => match content {
                ResearchContent::Listing(listing) => {
                    self.loaded = Some(Loaded {
                        task: *task,
                        listing: listing.clone(),
                    });
                    false
                }
                ResearchContent::Document(document) => {
                    let wanted = self
                        .awaiting
                        .as_ref()
                        .is_some_and(|(t, w, _)| t == task && w == what);
                    if !wanted {
                        return false;
                    }
                    let Some((_, _, title)) = self.awaiting.take() else {
                        return false;
                    };
                    self.task = Some(*task);
                    self.reading = Some(Reading {
                        doc: Doc::new(title, document.clone()),
                        top: 0,
                        lines: Vec::new(),
                        width: 0,
                        page: 10,
                    });
                    true
                }
            },
            Event::CommandFailed { .. } => {
                self.awaiting = None;
                false
            }
            _ => false,
        }
    }

    fn render_reading(&mut self, cx: &ViewCx, frame: &mut Frame, inner: Rect) {
        let theme = cx.theme;
        let solid = self.solid;
        let Some(reading) = self.reading.as_mut() else {
            return;
        };
        let body = Rect {
            height: inner.height.saturating_sub(u16::from(inner.height >= 5)),
            ..inner
        };
        let column = reading_column::column(body, reading_column::DEFAULT_WIDTH);
        let width = usize::from(column.width);
        if reading.width != width || reading.lines.is_empty() {
            let lines = if reading.doc.markdown.trim().is_empty() {
                vec![Line::from(Span::styled("(empty)", theme.dim()))]
            } else {
                citations::colour(markdown::render(&reading.doc.markdown, width, theme), theme)
            };
            reading.lines = lines;
            reading.width = width;
        }
        let height = usize::from(body.height);
        reading.page = height.max(1);
        reading.top = reading.top.min(reading.lines.len().saturating_sub(height));
        for (n, line) in reading
            .lines
            .iter()
            .skip(reading.top)
            .take(height)
            .enumerate()
        {
            let rect = Rect {
                x: column.x,
                y: body.y + n as u16,
                width: column.width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(line.clone()), rect);
        }
        if solid {
            reading_column::paint_background(
                frame.buffer_mut(),
                column,
                Style::new().bg(theme.reading_bg),
            );
        }
        footer(
            frame,
            inner,
            &[
                "j k scroll · gg G · e edit file · / search · esc back",
                "j k scroll · e edit · esc back",
                "e edit · esc back",
            ],
            theme,
        );
    }

    fn selected_item(&self) -> Option<&Item> {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(|(_, item)| item)
    }

    fn open_selected(&self, task: TaskId, effects: &mut Vec<Effect>) {
        match self.selected_item() {
            Some(item) => effects.push(Effect::OpenResearch {
                task,
                what: item.what.clone(),
                title: item.title.clone(),
            }),
            None if self.items.is_empty() => {
                effects.push(Effect::Note("no research to open here".into()));
            }
            None => effects.push(Effect::Note(
                "Tab picks a research row, then ⏎ opens it".into(),
            )),
        }
    }

    /// `Tab` / `Shift-Tab`: the next or previous research row, wrapping round.
    fn pick(&mut self, forward: bool) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        self.selected = Some(match (self.selected, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        });
        self.follow = true;
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
        let mut heading = task.map_or("task".to_string(), |t| format!("task · #{}", t.id.0));
        if let Some(reading) = &self.reading {
            heading = format!("{heading} · {}", reading.doc.title);
        }
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
        if self.reading.is_some() {
            self.render_reading(cx, frame, inner);
            return;
        }
        let width = usize::from(inner.width).min(100);
        let built = build(cx.snap, task, width, theme, cx.tz, self.loaded.as_ref());
        let all = built.lines;
        self.items = built.items;
        self.selected = self.selected.filter(|i| *i < self.items.len());
        let height = usize::from(inner.height.saturating_sub(u16::from(inner.height >= 5)));
        self.last_lines = all.len();
        self.last_height = height;
        if std::mem::take(&mut self.follow)
            && let Some((row, _)) = self.selected.and_then(|i| self.items.get(i))
        {
            self.top = scroll_to(self.top, *row, *row, height);
        }
        self.top = self.top.min(all.len().saturating_sub(height));
        let selected_row = self
            .selected
            .and_then(|i| self.items.get(i))
            .map(|(row, _)| *row);
        for (n, line) in all.into_iter().enumerate().skip(self.top).take(height) {
            let rect = Rect {
                y: inner.y + (n - self.top) as u16,
                height: 1,
                ..inner
            };
            let line = if Some(n) == selected_row && focused {
                line.style(theme.selected())
            } else {
                line
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        let research = !self.items.is_empty();
        footer(
            frame,
            inner,
            &if research {
                [
                    "j k scroll · ⇥ research row · ⏎ read · e edit/notes · s b p · R relate · a decision · esc back",
                    "j k scroll · ⇥ row · ⏎ read · e edit · R relate · a decision · esc back",
                    "⇥ row · ⏎ read · e edit · esc back",
                ]
            } else {
                [
                    "s status · b bucket · p P-level · R relate · a decision · e notes · esc back",
                    "s b p cycle · R relate · a decision · esc back",
                    "R relate · a decision · esc back",
                ]
            },
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        if let Some(reading) = &mut self.reading {
            let last = reading.lines.len().saturating_sub(1);
            match action {
                Action::MoveDown => reading.top = (reading.top + 1).min(last),
                Action::MoveUp => reading.top = reading.top.saturating_sub(1),
                Action::PageDown => reading.top = (reading.top + reading.page).min(last),
                Action::PageUp => reading.top = reading.top.saturating_sub(reading.page),
                Action::GoTop => reading.top = 0,
                Action::GoBottom => reading.top = last,
                Action::TaskEditNotes => match &reading.doc.path {
                    Some(path) => effects.push(Effect::OpenEditor(path.clone())),
                    None => effects.push(Effect::Note(
                        "this source has no file: it was listed, not cached".into(),
                    )),
                },
                _ => return false,
            }
            return true;
        }
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
            // `j`/`k` scroll line by line; `Tab`/`Shift-Tab` pick between the research rows.
            (Action::ResearchNext, _) => self.pick(true),
            (Action::ResearchPrev, _) => self.pick(false),
            (Action::MoveDown, _) => {
                self.top = (self.top + 1).min(self.last_lines.saturating_sub(1))
            }
            (Action::MoveUp, _) => self.top = self.top.saturating_sub(1),
            (Action::PageDown, _) => {
                self.top =
                    (self.top + self.last_height.max(1)).min(self.last_lines.saturating_sub(1))
            }
            (Action::PageUp, _) => self.top = self.top.saturating_sub(self.last_height.max(1)),
            (Action::GoTop, _) => self.top = 0,
            (Action::GoBottom, _) => self.top = self.last_lines.saturating_sub(1),
            (Action::ResearchOpen, Some(t)) => self.open_selected(t.id, effects),
            (Action::TaskCycleStatus, Some(t)) => {
                effects.push(change(TaskChange::Status(next_status(t.status)), t));
            }
            (Action::TaskCycleBucket, Some(t)) => {
                effects.push(change(TaskChange::Bucket(next_bucket(t.bucket)), t));
            }
            (Action::TaskCyclePLevel, Some(t)) => {
                effects.push(change(TaskChange::PLevel(next_p_level(t.p_level)), t));
            }
            (Action::TaskRelate, Some(t)) => effects.push(Effect::PickRelated(t.id)),
            (Action::TaskAddDecision, Some(t)) => effects.push(Effect::StartInput {
                target: super::InputTarget::Decision(t.id),
                initial: String::new(),
            }),
            // On a selected research row `e` edits its file; with none selected it is the notes.
            (Action::TaskEditNotes, _) if self.selected.is_some() => {
                match self.selected_item().and_then(|item| item.path.clone()) {
                    Some(path) => effects.push(Effect::OpenEditor(path)),
                    None => effects.push(Effect::Note(
                        "this source has no file: it was listed, not cached".into(),
                    )),
                }
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
        let text = |lines: &[Line<'static>]| -> Vec<String> {
            lines
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect()
        };
        if let Some(reading) = &self.reading {
            return text(&reading.lines);
        }
        self.task
            .and_then(|id| cx.snap.tasks.iter().find(|t| t.id == id))
            .map(|t| text(&build(cx.snap, t, 100, cx.theme, cx.tz, self.loaded.as_ref()).lines))
            .unwrap_or_default()
    }

    fn current_row(&self, _cx: &ViewCx) -> usize {
        self.reading.as_ref().map_or(self.top, |r| r.top)
    }

    fn goto_row(&mut self, row: usize, _cx: &ViewCx, _effects: &mut Vec<Effect>) {
        match &mut self.reading {
            Some(reading) => reading.top = row,
            None => self.top = row,
        }
    }
}

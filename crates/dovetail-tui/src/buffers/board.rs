//! The board: the project's tasks as a kanban in the main region, one column per status (todo,
//! in progress, done). A task that needs the user shows a marker on its card, in its own status
//! column; there is no needs-you column (plan B4).

use std::collections::HashSet;

use dovetail_api::{
    AgentKind, AgentStatus, Bucket, Command, Snapshot, Task, TaskChange, TaskId, TaskStatus,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use super::tree::{agents_waiting, p_label};
use super::{Buffer, Effect, ViewCx, clip_spans, footer, next_status, pane, scroll_to, title};
use crate::action::Action;
use crate::region::BufferKind;
use crate::theme::Theme;

/// The board's columns, left to right.
pub const COLUMNS: [TaskStatus; 3] = [TaskStatus::Todo, TaskStatus::InProgress, TaskStatus::Done];

/// Rows a card takes: its border and two lines of text.
const CARD_HEIGHT: u16 = 4;
/// Blank rows between two cards.
const CARD_GAP: u16 = 1;

/// The board buffer: which column has the cursor, and the card selected in each column.
#[derive(Default)]
pub struct BoardBuffer {
    column: usize,
    /// The card selected in each column, kept by id so a live update never moves the cursor.
    selected: [Option<TaskId>; 3],
    /// Where each column's selection was last, for when its card leaves the column.
    last_index: [usize; 3],
    /// First card shown in each column.
    offset: [usize; 3],
    /// A card sent to another column: the cursor follows it once the core confirms the new
    /// status, and stays put if the change is refused.
    pending: Option<(TaskId, TaskStatus)>,
}

impl BoardBuffer {
    /// The task on the selected card, if the cursor's column has any card.
    pub fn selected_task(&self, snap: &Snapshot) -> Option<TaskId> {
        let cards = cards(snap, COLUMNS[self.column]);
        self.resolve(self.column, &cards).map(|i| cards[i].id)
    }

    /// The column the cursor is in.
    pub fn column(&self) -> TaskStatus {
        COLUMNS[self.column]
    }

    /// Index of the selected card in `column`, or the nearest one to where it was.
    fn resolve(&self, column: usize, cards: &[&Task]) -> Option<usize> {
        if cards.is_empty() {
            return None;
        }
        if let Some(id) = self.selected[column]
            && let Some(i) = cards.iter().position(|t| t.id == id)
        {
            return Some(i);
        }
        Some(self.last_index[column].min(cards.len() - 1))
    }

    fn select(&mut self, column: usize, cards: &[&Task], index: usize) {
        self.selected[column] = Some(cards[index].id);
        self.last_index[column] = index;
    }

    /// Moves the cursor onto a card whose status change the core has confirmed.
    fn follow(&mut self, snap: &Snapshot) {
        let Some((id, status)) = self.pending else {
            return;
        };
        if snap.tasks.iter().any(|t| t.id == id && t.status == status)
            && let Some(column) = COLUMNS.iter().position(|&s| s == status)
        {
            self.column = column;
            self.selected[column] = Some(id);
            self.pending = None;
        }
    }

    /// `<`/`>`: moves the card past its neighbour in the column. The order is the task's place
    /// in its bucket ([`Command::ReorderTask`]), so a card moves past neighbours of its own
    /// bucket only; a column lists Now, Next, Later, then Unsorted.
    fn reorder(&mut self, snap: &Snapshot, delta: isize, effects: &mut Vec<Effect>) {
        let cards = cards(snap, COLUMNS[self.column]);
        let Some(i) = self.resolve(self.column, &cards) else {
            effects.push(Effect::Note("no card in this column".into()));
            return;
        };
        let task = cards[i];
        let Some(neighbour) = i
            .checked_add_signed(delta)
            .and_then(|j| cards.get(j))
            .filter(|n| n.bucket == task.bucket)
        else {
            let note = if i.checked_add_signed(delta).is_some_and(|j| j < cards.len()) {
                "cards move within their bucket; b changes the bucket in the tree"
            } else if delta < 0 {
                "already at the top"
            } else {
                "already at the bottom"
            };
            effects.push(Effect::Note(note.into()));
            return;
        };
        let mut peers: Vec<&Task> = snap
            .tasks
            .iter()
            .filter(|t| t.bucket == task.bucket && t.status != TaskStatus::Archived)
            .collect();
        peers.sort_by_key(|t| (t.order, t.id));
        let place = |id: TaskId| peers.iter().position(|t| t.id == id);
        if let (Some(from), Some(to)) = (place(task.id), place(neighbour.id)) {
            effects.push(Effect::Command(Command::ReorderTask {
                id: task.id,
                delta: to as i32 - from as i32,
            }));
        }
    }

    fn move_card(&mut self, snap: &Snapshot, delta: isize, effects: &mut Vec<Effect>) {
        let cards = cards(snap, COLUMNS[self.column]);
        let Some(i) = self.resolve(self.column, &cards) else {
            effects.push(Effect::Note("no card in this column".into()));
            return;
        };
        let Some(to) = self
            .column
            .checked_add_signed(delta)
            .filter(|&c| c < COLUMNS.len())
        else {
            let edge = if delta < 0 { "first" } else { "last" };
            effects.push(Effect::Note(format!("already in the {edge} column")));
            return;
        };
        self.send_to(cards[i].id, to, effects);
    }

    /// Asks the core to give the task the status of column `to`. The cursor follows the card
    /// once the core confirms it ([`Self::follow`]).
    fn send_to(&mut self, id: TaskId, to: usize, effects: &mut Vec<Effect>) {
        self.pending = Some((id, COLUMNS[to]));
        effects.push(Effect::Command(Command::UpdateTask {
            id,
            patch: vec![TaskChange::Status(COLUMNS[to])],
        }));
    }

    fn draw_column(
        &mut self,
        cx: &ViewCx,
        frame: &mut Frame,
        area: Rect,
        column: usize,
        focused: bool,
        waiting: &HashSet<TaskId>,
    ) {
        let theme = cx.theme;
        let status = COLUMNS[column];
        let cards = cards(cx.snap, status);
        let (glyph, color) = column_glyph(status, theme);
        let here = focused && column == self.column;
        let heading = vec![
            Span::styled(glyph, theme.fg(color)),
            Span::raw(" "),
            Span::styled(
                column_label(status),
                if here {
                    theme.fg(theme.focus).add_modifier(Modifier::BOLD)
                } else {
                    theme.body().add_modifier(Modifier::BOLD)
                },
            ),
            Span::styled(format!(" {}", cards.len()), theme.dim()),
        ];
        let width = usize::from(area.width);
        frame.render_widget(
            Paragraph::new(Line::from(clip_spans(heading, width))),
            Rect { height: 1, ..area },
        );
        if area.height < 2 {
            return;
        }
        frame.render_widget(
            Paragraph::new(Span::styled("─".repeat(width), theme.fg(theme.line))),
            Rect {
                y: area.y + 1,
                height: 1,
                ..area
            },
        );
        let body = Rect {
            y: area.y + 3,
            height: area.height.saturating_sub(3),
            ..area
        };
        if body.height == 0 {
            return;
        }
        if cards.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled("empty", theme.dim())),
                Rect { height: 1, ..body },
            );
            return;
        }
        let selected = self.resolve(column, &cards);
        let fit = usize::from((body.height + CARD_GAP) / (CARD_HEIGHT + CARD_GAP)).max(1);
        if let Some(i) = selected.filter(|_| column == self.column) {
            self.offset[column] = scroll_to(self.offset[column], i, i, fit);
        }
        self.offset[column] = self.offset[column].min(cards.len().saturating_sub(fit));
        for (n, (i, task)) in cards
            .iter()
            .enumerate()
            .skip(self.offset[column])
            .take(fit)
            .enumerate()
        {
            let y = body.y + n as u16 * (CARD_HEIGHT + CARD_GAP);
            let height = CARD_HEIGHT.min(body.y + body.height - y);
            let rect = Rect { y, height, ..body };
            let cursor = here && selected == Some(i);
            draw_card(cx, frame, rect, task, waiting.contains(&task.id), cursor);
        }
    }
}

impl Buffer for BoardBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Board
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        self.follow(cx.snap);
        let theme = cx.theme;
        let open = cx
            .snap
            .tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Todo | TaskStatus::InProgress))
            .count();
        let heading = match &cx.snap.project {
            Some(p) => format!("board · {} · {open} open", p.name),
            None => format!("board · {open} open"),
        };
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        // The footer takes the last row and a blank one above it, when there is room for it.
        let reserved = if inner.height >= 5 { 2 } else { 0 };
        let columns_area = Rect {
            y: inner.y + 1.min(inner.height),
            height: inner.height.saturating_sub(reserved + 1),
            ..inner
        };
        let gap: u16 = if inner.width >= 60 { 2 } else { 1 };
        let n = COLUMNS.len() as u16;
        let col_width = inner.width.saturating_sub(gap * (n - 1)) / n;
        let waiting = waiting_tasks(cx.snap);
        for column in 0..COLUMNS.len() {
            let x = inner.x + column as u16 * (col_width + gap);
            let rect = Rect {
                x,
                width: col_width,
                ..columns_area
            };
            self.draw_column(cx, frame, rect, column, focused, &waiting);
        }
        footer(
            frame,
            inner,
            &[
                "j/k card · ⇥ column · H/L move card · < > reorder · ⏎ open · a add · s status",
                "j/k card · ⇥ column · H/L move · < > reorder · ⏎ open · a add",
                "jk ⇥ HL <> ⏎ a s",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        self.follow(cx.snap);
        // A refused change is forgotten at the next key, so a later one never moves the cursor.
        self.pending = None;
        let column = self.column;
        let cards = cards(cx.snap, COLUMNS[column]);
        let current = self.resolve(column, &cards);
        match action {
            Action::MoveDown | Action::MoveUp => {
                if let Some(i) = current {
                    let delta = if action == Action::MoveDown { 1 } else { -1 };
                    self.select(column, &cards, super::step(i, cards.len(), delta));
                }
            }
            Action::GoTop if !cards.is_empty() => self.select(column, &cards, 0),
            Action::GoBottom if !cards.is_empty() => {
                self.select(column, &cards, cards.len() - 1);
            }
            // `Tab` and `Shift-Tab` wrap around, as a cycle of three columns.
            Action::BoardColumnLeft => self.column = (column + COLUMNS.len() - 1) % COLUMNS.len(),
            Action::BoardColumnRight => self.column = (column + 1) % COLUMNS.len(),
            Action::BoardMoveCardLeft => self.move_card(cx.snap, -1, effects),
            Action::BoardMoveCardRight => self.move_card(cx.snap, 1, effects),
            Action::BoardCardUp => self.reorder(cx.snap, -1, effects),
            Action::BoardCardDown => self.reorder(cx.snap, 1, effects),
            Action::TaskCycleStatus => match current {
                Some(i) => {
                    let next = next_status(cards[i].status);
                    let to = COLUMNS.iter().position(|&s| s == next).unwrap_or(0);
                    self.send_to(cards[i].id, to, effects);
                }
                None => effects.push(Effect::Note("no card in this column".into())),
            },
            Action::BoardOpen => match current {
                Some(i) => effects.push(Effect::OpenTask(cards[i].id)),
                None => effects.push(Effect::Note("no card in this column".into())),
            },
            _ => return false,
        }
        true
    }
}

/// The tasks in a status column, by bucket (Now, Next, Later, Unsorted), then the user's order
/// within the bucket, which `<`/`>` change. Archived tasks never show.
fn cards(snap: &Snapshot, status: TaskStatus) -> Vec<&Task> {
    let mut tasks: Vec<&Task> = snap.tasks.iter().filter(|t| t.status == status).collect();
    tasks.sort_by_key(|t| (bucket_rank(t.bucket), t.order, t.id));
    tasks
}

fn bucket_rank(bucket: Option<Bucket>) -> u8 {
    match bucket {
        Some(Bucket::Now) => 0,
        Some(Bucket::Next) => 1,
        Some(Bucket::Later) => 2,
        None => 3,
    }
}

/// Tasks that need the user, derived from the inbox (plan B4). Done wins: a done task never
/// shows the marker, though its item still shows in the inbox.
fn waiting_tasks(snap: &Snapshot) -> HashSet<TaskId> {
    let done: HashSet<TaskId> = snap
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .map(|t| t.id)
        .collect();
    agents_waiting(snap)
        .into_iter()
        .filter(|id| !done.contains(id))
        .collect()
}

fn column_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "Todo",
        TaskStatus::InProgress | TaskStatus::Review => "In progress",
        TaskStatus::Done => "Done",
        TaskStatus::Archived => "Archived",
    }
}

fn column_glyph(status: TaskStatus, theme: &Theme) -> (&'static str, Color) {
    match status {
        TaskStatus::Todo | TaskStatus::Archived => ("○", theme.dim),
        TaskStatus::InProgress | TaskStatus::Review => ("◐", theme.working),
        TaskStatus::Done => ("✓", theme.ok),
    }
}

/// One card: a border, then `⚠ #id title` and `P1 · ◆ slice-name`.
fn draw_card(
    cx: &ViewCx,
    frame: &mut Frame,
    area: Rect,
    task: &Task,
    needs_you: bool,
    cursor: bool,
) {
    let theme = cx.theme;
    let border = if cursor {
        theme.focus
    } else if needs_you {
        theme.needs_you
    } else {
        theme.line
    };
    let block = Block::bordered().border_style(theme.fg(border));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect {
        x: inner.x.saturating_add(1),
        width: inner.width.saturating_sub(2),
        ..inner
    };
    let width = usize::from(inner.width);
    let done = task.status == TaskStatus::Done;

    let mut first = Vec::new();
    if needs_you {
        first.push(Span::styled("⚠ ", theme.fg(theme.needs_you)));
    }
    first.push(Span::styled(format!("#{} ", task.id.0), theme.dim()));
    first.push(Span::styled(
        task.title.clone(),
        if done {
            theme.dim()
        } else {
            theme.body().add_modifier(Modifier::BOLD)
        },
    ));

    let mut second = Vec::new();
    if let Some(p) = task.p_level {
        second.push(Span::styled(p_label(p), theme.dim()));
    }
    let slices = live_slices(cx.snap, task.id);
    if slices.is_empty() {
        if second.is_empty() {
            second.push(Span::styled("no slice", theme.dim()));
        }
    } else {
        for (name, waiting) in slices {
            if !second.is_empty() {
                second.push(Span::styled(" · ", theme.dim()));
            }
            let color = if waiting { theme.needs_you } else { theme.body };
            second.push(Span::styled(format!("◆ {name}"), theme.fg(color)));
        }
    }

    let lines = [first, second];
    for (row, spans) in lines.into_iter().enumerate() {
        if row as u16 >= inner.height {
            break;
        }
        frame.render_widget(
            Paragraph::new(Line::from(clip_spans(spans, width))),
            Rect {
                y: inner.y + row as u16,
                height: 1,
                ..inner
            },
        );
    }
    if cursor {
        frame.buffer_mut().set_style(inner, theme.selected());
    }
}

/// The task's slices that are still running, by name, and whether each is waiting on the user.
fn live_slices(snap: &Snapshot, task: TaskId) -> Vec<(String, bool)> {
    snap.agents
        .iter()
        .filter(|a| a.task == Some(task) && a.kind == AgentKind::Slice)
        .filter(|a| a.status != AgentStatus::Done)
        .map(|a| (a.name.clone(), a.status == AgentStatus::NeedsYou))
        .collect()
}

//! The task tree: project tasks grouped by bucket, each with its agents, their branch and
//! their to-do items.

use std::collections::{HashMap, HashSet};

use dovetail_api::{
    Agent, AgentId, AgentKind, AgentStatus, Bucket, Command, PLevel, PermissionMode, Snapshot,
    Task, TaskChange, TaskId, TaskStatus, TodoStatus,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    Buffer, Effect, InputTarget, ViewCx, footer, next_bucket, next_p_level, next_status, pane,
    scroll_to, spans_width, spread_fit, status_glyph, status_word, step, title,
};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;
use crate::theme::Theme;

/// The groups the tree sorts tasks into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// No bucket yet; hidden when empty.
    Unsorted,
    /// Doing it now.
    Now,
    /// Up next.
    Next,
    /// Some time later.
    Later,
    /// Marked done.
    Done,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Group::Unsorted => "Unsorted",
            Group::Now => "Now",
            Group::Next => "Next",
            Group::Later => "Later",
            Group::Done => "Done",
        }
    }

    fn expanded_by_default(self) -> bool {
        !matches!(self, Group::Later | Group::Done)
    }
}

/// Identifies a tree row across rebuilds, so a live update never moves the cursor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowKey {
    /// A bucket header.
    Group(Group),
    /// A task.
    Task(TaskId),
    /// An agent.
    Agent(AgentId),
    /// The branch line under a slice.
    Branch(AgentId),
    /// A to-do item.
    Todo(AgentId, String),
}

struct Row {
    key: RowKey,
    depth: usize,
    /// `Some(expanded)` for nodes that fold.
    fold: Option<bool>,
    left: Vec<Span<'static>>,
    /// Versions of the right-hand side, fullest first; a narrow pane uses a shorter one.
    right: Vec<Vec<Span<'static>>>,
    /// An agent's one-line status (`report_status`): dim, after the name, cut to the room left
    /// by the fullest right side, and the first thing dropped in a narrow pane.
    status: Option<String>,
}

impl Row {
    fn selectable(&self) -> bool {
        !matches!(self.key, RowKey::Branch(_))
    }
}

/// The task tree buffer.
#[derive(Default)]
pub struct TreeBuffer {
    selected: Option<RowKey>,
    /// Where the selection was last, for when its row disappears.
    last_index: usize,
    /// Fold overrides; absent means the default for the row.
    folds: HashMap<RowKey, bool>,
    offset: usize,
}

impl TreeBuffer {
    /// The agent under the cursor: an agent row, or a to-do item's creator.
    pub fn selected_agent(&self, cx: &ViewCx) -> Option<AgentId> {
        match self.selected_key(cx)? {
            RowKey::Agent(id) | RowKey::Branch(id) | RowKey::Todo(id, _) => Some(id),
            _ => None,
        }
    }

    /// The task the cursor is on or under.
    pub fn selected_task(&self, cx: &ViewCx) -> Option<TaskId> {
        match self.selected_key(cx)? {
            RowKey::Task(id) => Some(id),
            RowKey::Agent(id) | RowKey::Branch(id) | RowKey::Todo(id, _) => {
                cx.snap.agents.iter().find(|a| a.id == id)?.task
            }
            RowKey::Group(_) => None,
        }
    }

    /// The selected row's key, falling back to the row nearest the old position.
    fn selected_key(&self, cx: &ViewCx) -> Option<RowKey> {
        let rows = self.rows(cx);
        self.resolve(&rows).map(|i| rows[i].key.clone())
    }

    /// Index of the selected row, or of the nearest selectable row when it disappeared.
    fn resolve(&self, rows: &[Row]) -> Option<usize> {
        if let Some(key) = &self.selected
            && let Some(i) = rows.iter().position(|r| &r.key == key)
        {
            return Some(i);
        }
        let from = self.last_index.min(rows.len().saturating_sub(1));
        (from..rows.len())
            .chain((0..from).rev())
            .find(|&i| rows[i].selectable())
    }

    fn is_expanded(&self, key: &RowKey) -> bool {
        self.folds.get(key).copied().unwrap_or(match key {
            RowKey::Group(g) => g.expanded_by_default(),
            _ => true,
        })
    }

    fn rows(&self, cx: &ViewCx) -> Vec<Row> {
        let snap = cx.snap;
        let theme = cx.theme;
        let mut rows = Vec::new();
        let waiting = agents_waiting(snap);

        for agent in snap
            .agents
            .iter()
            .filter(|a| a.task.is_none() && a.parent.is_none())
        {
            self.agent_rows(snap, theme, agent, 0, &mut rows);
        }

        for group in [
            Group::Unsorted,
            Group::Now,
            Group::Next,
            Group::Later,
            Group::Done,
        ] {
            let tasks = tasks_in(snap, group);
            if tasks.is_empty() && matches!(group, Group::Unsorted | Group::Done) {
                continue;
            }
            let key = RowKey::Group(group);
            let open = self.is_expanded(&key);
            rows.push(Row {
                key,
                depth: 0,
                fold: Some(open),
                left: vec![Span::styled(
                    format!("{} ({})", group.label(), tasks.len()),
                    theme.dim().add_modifier(Modifier::BOLD),
                )],
                right: vec![],
                status: None,
            });
            if !open {
                continue;
            }
            for task in tasks {
                self.task_rows(snap, theme, task, group, &waiting, &mut rows);
            }
        }
        rows
    }

    fn task_rows(
        &self,
        snap: &Snapshot,
        theme: &Theme,
        task: &Task,
        group: Group,
        waiting: &HashSet<TaskId>,
        rows: &mut Vec<Row>,
    ) {
        let key = RowKey::Task(task.id);
        let open = self.is_expanded(&key);
        let (glyph, color) = task_glyph(task, waiting.contains(&task.id), theme);
        let done = task.status == TaskStatus::Done;
        let mut left = vec![
            Span::styled(glyph, theme.fg(color)),
            Span::raw(" "),
            Span::styled(
                task.title.clone(),
                if done {
                    theme.dim()
                } else {
                    theme.body().add_modifier(Modifier::BOLD)
                },
            ),
        ];
        if let Some(p) = task.p_level {
            left.push(Span::styled(format!(" {}", p_label(p)), theme.dim()));
            if mismatch(p, task.bucket) {
                left.push(Span::styled(" ≠", theme.fg(theme.needs_you)));
            }
        }
        if task.coordinator_created {
            left.push(Span::styled(" ✦", theme.accent()));
        }
        if let Some(parent) = task.follow_up_of {
            left.push(Span::styled(format!(" ↳#{}", parent.0), theme.dim()));
        }
        if !task.related.is_empty() {
            left.push(Span::styled(
                format!(" ↔ {}", task.related.len()),
                theme.dim(),
            ));
        }
        let right = task
            .pr
            .map(|pr| vec![vec![Span::styled(format!("PR #{pr}"), theme.dim())]])
            .unwrap_or_default();
        let sessions: Vec<&Agent> = snap
            .agents
            .iter()
            .filter(|a| a.task == Some(task.id) && a.parent.is_none())
            .collect();
        rows.push(Row {
            key,
            depth: 1,
            fold: (!sessions.is_empty()).then_some(open),
            left,
            right,
            status: None,
        });
        if open && group != Group::Done {
            for agent in sessions {
                self.agent_rows(snap, theme, agent, 2, rows);
            }
        }
    }

    fn agent_rows(
        &self,
        snap: &Snapshot,
        theme: &Theme,
        agent: &Agent,
        depth: usize,
        rows: &mut Vec<Row>,
    ) {
        let key = RowKey::Agent(agent.id.clone());
        let open = self.is_expanded(&key);
        let todos: Vec<_> = snap
            .todos
            .iter()
            .filter(|t| t.created_by.as_ref() == Some(&agent.id))
            .collect();
        let children: Vec<&Agent> = snap
            .agents
            .iter()
            .filter(|a| a.parent.as_ref() == Some(&agent.id))
            .collect();
        let has_branch = agent.worktree.is_some() && agent.kind == AgentKind::Slice;
        let foldable = has_branch || !todos.is_empty() || !children.is_empty();
        let (left, right) = agent_line(theme, agent);
        let status = Some(agent.status_line.trim().to_owned()).filter(|s| !s.is_empty());
        rows.push(Row {
            key,
            depth,
            fold: foldable.then_some(open),
            left,
            right,
            status,
        });
        if !open {
            return;
        }
        if let (true, Some(wt)) = (has_branch, &agent.worktree) {
            rows.push(Row {
                key: RowKey::Branch(agent.id.clone()),
                depth: depth + 2,
                fold: None,
                left: vec![Span::styled(format!("⎇ {}", wt.branch), theme.dim())],
                right: vec![],
                status: None,
            });
        }
        for todo in todos {
            let (glyph, color) = match todo.status {
                TodoStatus::Completed => ("✓", theme.ok),
                TodoStatus::InProgress => ("◐", theme.working),
                TodoStatus::Pending => ("○", theme.dim),
            };
            let mut left = vec![
                Span::styled(glyph, theme.fg(color)),
                Span::raw(" "),
                Span::styled(todo.subject.clone(), theme.body()),
            ];
            if !todo.blocked_by.is_empty() && todo.status != TodoStatus::Completed {
                left.push(Span::styled(" ⊘ blocked", theme.dim()));
            }
            rows.push(Row {
                key: RowKey::Todo(agent.id.clone(), todo.id.clone()),
                depth: depth + 2,
                fold: None,
                left,
                right: vec![],
                status: None,
            });
        }
        for child in children {
            self.agent_rows(snap, theme, child, depth + 1, rows);
        }
    }

    fn move_selection(&mut self, cx: &ViewCx, delta: isize, effects: &mut Vec<Effect>) {
        let rows = self.rows(cx);
        let Some(current) = self.resolve(&rows) else {
            return;
        };
        let mut index = current;
        loop {
            let next = step(index, rows.len(), delta.signum());
            if next == index {
                break;
            }
            index = next;
            if rows[index].selectable() {
                break;
            }
        }
        if !rows[index].selectable() {
            index = current;
        }
        self.select(&rows, index, cx, effects);
    }

    fn select_edge(&mut self, cx: &ViewCx, last: bool, effects: &mut Vec<Effect>) {
        let rows = self.rows(cx);
        let index = if last {
            rows.iter().rposition(Row::selectable)
        } else {
            rows.iter().position(Row::selectable)
        };
        if let Some(index) = index {
            self.select(&rows, index, cx, effects);
        }
    }

    /// Moves the cursor. A coordinator-created task is marked seen once the cursor rests on it.
    fn select(&mut self, rows: &[Row], index: usize, cx: &ViewCx, effects: &mut Vec<Effect>) {
        self.selected = Some(rows[index].key.clone());
        self.last_index = index;
        if let RowKey::Task(id) = rows[index].key
            && cx
                .snap
                .tasks
                .iter()
                .any(|t| t.id == id && t.coordinator_created)
        {
            effects.push(Effect::Command(Command::UpdateTask {
                id,
                patch: vec![TaskChange::Seen],
            }));
        }
    }

    fn task_action(&self, cx: &ViewCx, effects: &mut Vec<Effect>, f: impl FnOnce(&Task) -> Effect) {
        match self
            .selected_task(cx)
            .and_then(|id| cx.snap.tasks.iter().find(|t| t.id == id))
        {
            Some(task) => effects.push(f(task)),
            None => effects.push(Effect::Note("select a task first".into())),
        }
    }
}

/// Agents with an item in Needs you, mapped to their tasks. A coordinator item marks no task
/// (plan B4).
fn agents_waiting(snap: &Snapshot) -> HashSet<TaskId> {
    snap.inbox
        .iter()
        .filter_map(|item| snap.agents.iter().find(|a| a.id == item.agent)?.task)
        .collect()
}

/// Tasks of a group in display order. Now keeps the user's own order; the other buckets sort
/// by P-level, most important first, then by order.
fn tasks_in(snap: &Snapshot, group: Group) -> Vec<&Task> {
    let mut tasks: Vec<&Task> = snap
        .tasks
        .iter()
        .filter(|t| match (group, t.status) {
            (_, TaskStatus::Archived) => false,
            (Group::Done, status) => status == TaskStatus::Done,
            (_, TaskStatus::Done) => false,
            (Group::Unsorted, _) => t.bucket.is_none(),
            (Group::Now, _) => t.bucket == Some(Bucket::Now),
            (Group::Next, _) => t.bucket == Some(Bucket::Next),
            (Group::Later, _) => t.bucket == Some(Bucket::Later),
        })
        .collect();
    if group == Group::Now {
        tasks.sort_by_key(|t| (t.order, t.id));
    } else {
        tasks.sort_by_key(|t| (t.p_level.is_none(), t.p_level, t.order, t.id));
    }
    tasks
}

/// A P0 left for later, or a P3 in Now, deserves a second look.
fn mismatch(p: PLevel, bucket: Option<Bucket>) -> bool {
    matches!(
        (p, bucket),
        (PLevel::P0, Some(Bucket::Later)) | (PLevel::P3, Some(Bucket::Now))
    )
}

/// `P0` to `P3`.
pub fn p_label(p: PLevel) -> &'static str {
    match p {
        PLevel::P0 => "P0",
        PLevel::P1 => "P1",
        PLevel::P2 => "P2",
        PLevel::P3 => "P3",
    }
}

/// Glyph for a task. Done wins; needs you is derived from the task's agents (plan B4).
fn task_glyph(task: &Task, waiting: bool, theme: &Theme) -> (&'static str, ratatui::style::Color) {
    match task.status {
        TaskStatus::Done => ("✓", theme.ok),
        _ if waiting => ("⚠", theme.needs_you),
        TaskStatus::InProgress => ("◐", theme.working),
        _ => ("○", theme.dim),
    }
}

/// Fewest characters of a status line worth showing; below this it is dropped.
const STATUS_MIN: usize = 8;

/// The status line's spans for a row, cut to fit beside `left` and the fullest right side, or
/// nothing when the pane is too narrow for even [`STATUS_MIN`] characters.
fn status_spans(
    status: &str,
    left: &[Span<'static>],
    rights: &[Vec<Span<'static>>],
    width: usize,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let right_w = rights.first().map_or(0, |r| spans_width(r) + 1);
    let room = width.saturating_sub(spans_width(left) + right_w + 2);
    if room < STATUS_MIN {
        return Vec::new();
    }
    let text = if UnicodeWidthStr::width(status) <= room {
        status.to_owned()
    } else {
        let mut cut = String::new();
        for c in status.chars() {
            if UnicodeWidthStr::width(cut.as_str()) + c.width().unwrap_or(0) + 1 > room {
                break;
            }
            cut.push(c);
        }
        format!("{}…", cut.trim_end())
    };
    vec![Span::raw("  "), Span::styled(text, theme.dim())]
}

fn agent_line(theme: &Theme, agent: &Agent) -> (Vec<Span<'static>>, Vec<Vec<Span<'static>>>) {
    let (glyph, color) = status_glyph(agent.status, theme);
    let kind = if agent.kind.is_session() {
        "◆"
    } else {
        "◇"
    };
    let mut left = vec![
        Span::styled(glyph, theme.fg(color)),
        Span::raw(" "),
        Span::styled(kind, theme.fg(color)),
        Span::raw(" "),
        Span::styled(agent.name.clone(), theme.body()),
    ];
    match agent.permission_mode {
        PermissionMode::Auto => {}
        PermissionMode::AcceptEdits => {
            left.push(Span::raw(" "));
            left.push(Span::styled(" edits ", theme.badge(theme.italic)));
        }
        PermissionMode::Manual => {
            left.push(Span::raw(" "));
            left.push(Span::styled(" manual ", theme.badge(theme.needs_you)));
        }
    }
    let word_style = if agent.status == AgentStatus::NeedsYou {
        theme.fg(theme.needs_you)
    } else {
        theme.dim()
    };
    let word = match (&agent.kind, &agent.progress) {
        (AgentKind::Subagent { background: true }, Some(p)) if p.summary.is_none() => {
            let last = p.last_tool.as_deref().unwrap_or("starting");
            Span::styled(format!("{} tools · {last}", p.tool_uses), theme.dim())
        }
        _ => Span::styled(status_word(agent.status), word_style),
    };
    let memory = agent
        .memory_mb
        .map(|mb| Span::styled(format!(" {}", fmt::memory(mb)), theme.dim()));
    let cost = (agent.cost_usd > 0.0)
        .then(|| Span::styled(format!(" {}", fmt::cost(agent.cost_usd)), theme.dim()));
    // Context fill shows for slices (Decision 22): plain, amber above 60 %, red above 80 %.
    let context = agent
        .context_pct
        .filter(|_| agent.kind == AgentKind::Slice)
        .map(|p| Span::styled(format!(" {p}%"), theme.context(p)));
    // Fullest first; a narrow pane drops cost, then memory, then the rest but the status.
    let versions = [
        vec![Some(word.clone()), memory.clone(), context.clone(), cost],
        vec![Some(word.clone()), memory, context.clone()],
        vec![Some(word.clone()), context],
        vec![Some(word)],
    ];
    let right = versions
        .into_iter()
        .map(|v| v.into_iter().flatten().collect())
        .collect();
    (left, right)
}

impl Buffer for TreeBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Tree
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let name = cx
            .snap
            .project
            .as_ref()
            .map_or("no project", |p| p.name.as_str());
        let inner = pane(
            frame,
            area,
            title(&format!("tasks · {name}"), focused, cx.theme),
            focused,
            cx.theme,
        );
        let rows = self.rows(cx);
        let selected = self.resolve(&rows);
        if let Some(i) = selected {
            self.selected = Some(rows[i].key.clone());
            self.last_index = i;
        }
        let body_height = usize::from(inner.height.saturating_sub(u16::from(inner.height >= 5)));
        if let Some(i) = selected {
            self.offset = scroll_to(self.offset, i, i, body_height);
        }
        self.offset = self.offset.min(rows.len().saturating_sub(body_height));
        let width = usize::from(inner.width);
        for (n, (i, row)) in rows
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(body_height)
            .enumerate()
        {
            let mut left = Vec::new();
            left.push(Span::raw("  ".repeat(row.depth.saturating_sub(1))));
            if row.depth > 0 || row.fold.is_some() {
                let glyph = match row.fold {
                    Some(true) => "▾ ",
                    Some(false) => "▸ ",
                    None => "  ",
                };
                left.push(Span::styled(glyph, cx.theme.dim()));
            }
            left.extend(row.left.iter().cloned());
            if let Some(status) = &row.status {
                left.extend(status_spans(status, &left, &row.right, width, cx.theme));
            }
            let line = spread_fit(left, &row.right, width);
            let rect = Rect {
                y: inner.y + n as u16,
                height: 1,
                ..inner
            };
            frame.render_widget(Paragraph::new(line), rect);
            if focused && selected == Some(i) {
                frame.buffer_mut().set_style(rect, cx.theme.selected());
            } else if selected == Some(i) {
                frame
                    .buffer_mut()
                    .set_style(rect, Style::new().add_modifier(Modifier::UNDERLINED));
            }
        }
        footer(
            frame,
            inner,
            &[
                "⏎ open · o fold · s status · b bucket · p P-level · dd archive",
                "⏎ open · o fold · s b p cycle · dd archive",
                "⏎ open · o fold · dd archive",
            ],
            cx.theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        match action {
            Action::MoveDown => self.move_selection(cx, 1, effects),
            Action::MoveUp => self.move_selection(cx, -1, effects),
            Action::GoTop => self.select_edge(cx, false, effects),
            Action::GoBottom => self.select_edge(cx, true, effects),
            Action::TreeToggleFold => {
                let rows = self.rows(cx);
                if let Some(i) = self.resolve(&rows)
                    && let Some(open) = rows[i].fold
                {
                    self.folds.insert(rows[i].key.clone(), !open);
                }
            }
            Action::TreeOpen => {
                let rows = self.rows(cx);
                let Some(i) = self.resolve(&rows) else {
                    return true;
                };
                match &rows[i].key {
                    RowKey::Agent(id) => effects.push(Effect::OpenAgent(id.clone())),
                    RowKey::Task(id) => effects.push(Effect::OpenTask(*id)),
                    RowKey::Group(_) => {
                        if let Some(open) = rows[i].fold {
                            self.folds.insert(rows[i].key.clone(), !open);
                        }
                    }
                    RowKey::Branch(_) | RowKey::Todo(..) => {}
                }
            }
            Action::TaskEditTitle => self.task_action(cx, effects, |t| Effect::StartInput {
                target: InputTarget::Title(t.id),
                initial: t.title.clone(),
            }),
            Action::TaskCycleStatus => self.task_action(cx, effects, |t| {
                update(t.id, TaskChange::Status(next_status(t.status)))
            }),
            Action::TaskCycleBucket => self.task_action(cx, effects, |t| {
                update(t.id, TaskChange::Bucket(next_bucket(t.bucket)))
            }),
            Action::TaskCyclePLevel => self.task_action(cx, effects, |t| {
                update(t.id, TaskChange::PLevel(next_p_level(t.p_level)))
            }),
            Action::TreeMoveDown | Action::TreeMoveUp => {
                let delta = if action == Action::TreeMoveDown {
                    1
                } else {
                    -1
                };
                self.task_action(cx, effects, |t| {
                    if t.bucket == Some(Bucket::Now) {
                        Effect::Command(Command::ReorderTask { id: t.id, delta })
                    } else {
                        Effect::Note("J and K reorder tasks within Now".into())
                    }
                });
            }
            Action::TaskArchive => {
                // On a slice's row, `dd` archives that slice; elsewhere, the task with its slices.
                let slice = match self.selected_key(cx) {
                    Some(RowKey::Agent(id)) => cx
                        .snap
                        .agents
                        .iter()
                        .find(|a| a.id == id && a.kind == AgentKind::Slice)
                        .map(|a| a.id.clone()),
                    _ => None,
                };
                match slice {
                    Some(agent) => effects.push(Effect::Command(Command::CloseSlice { agent })),
                    None => self.task_action(cx, effects, |t| {
                        Effect::Command(Command::ArchiveTask { id: t.id })
                    }),
                }
            }
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        // A row the cursor cannot rest on (a branch line) never matches.
        self.rows(cx)
            .iter()
            .map(|r| {
                if r.selectable() {
                    r.left.iter().map(|s| s.content.as_ref()).collect()
                } else {
                    String::new()
                }
            })
            .collect()
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        let rows = self.rows(cx);
        self.resolve(&rows).unwrap_or(0)
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, effects: &mut Vec<Effect>) {
        let rows = self.rows(cx);
        if row < rows.len() && rows[row].selectable() {
            self.select(&rows, row, cx, effects);
        }
    }
}

fn update(id: TaskId, change: TaskChange) -> Effect {
    Effect::Command(Command::UpdateTask {
        id,
        patch: vec![change],
    })
}

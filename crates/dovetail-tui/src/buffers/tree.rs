//! The task tree: project tasks grouped by bucket, each with its agents, their branch and
//! their to-do items.

use std::collections::{HashMap, HashSet};

use dovetail_api::{
    Agent, AgentId, AgentKind, AgentStatus, Bucket, Command, PLevel, PermissionMode, SessionLock,
    Snapshot, Task, TaskChange, TaskId, TaskStatus, TodoStatus,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    Buffer, Effect, InputTarget, ViewCx, footer, next_bucket, next_p_level, next_status, pane,
    pick_right, scroll_to, spans_width, spread_fit, status_glyph, status_word, step, title,
};
use crate::action::Action;
use crate::activity;
use crate::fmt;
use crate::region::BufferKind;
use crate::render::working;
use crate::theme::Theme;
use crate::usage;

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
    /// An agent's one-line status (`core_report_status`): dim, after the name, cut to the room left
    /// by the fullest right side, and the first thing dropped in a narrow pane.
    status: Option<String>,
    /// How many of the leading `right` versions carry a spinner; a draw that picks one of them
    /// needs the animation tick, any other does not.
    spinner_versions: usize,
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
    /// The last draw put a spinner on screen (a working agent's row).
    shows_working: bool,
}

impl TreeBuffer {
    /// Whether the last draw put a spinner on screen, so the UI must keep animating.
    pub fn shows_working(&self) -> bool {
        self.shows_working
    }

    /// Forgets the spinner; the next draw says again.
    pub fn clear_working(&mut self) {
        self.shows_working = false;
    }

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
            // A tangent belongs to Tangents, not this project: it shows in the side panel.
            .filter(|a| a.task.is_none() && a.parent.is_none() && a.kind != AgentKind::Tangent)
        {
            self.agent_rows(cx, agent, 0, &mut rows);
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
                spinner_versions: 0,
            });
            if !open {
                continue;
            }
            for task in tasks {
                self.task_rows(cx, task, group, &waiting, &mut rows);
            }
        }
        rows
    }

    fn task_rows(
        &self,
        cx: &ViewCx,
        task: &Task,
        group: Group,
        waiting: &HashSet<TaskId>,
        rows: &mut Vec<Row>,
    ) {
        let (snap, theme) = (cx.snap, cx.theme);
        let key = RowKey::Task(task.id);
        let open = self.is_expanded(&key);
        let (glyph, color) = task_glyph(task, waiting.contains(&task.id), theme);
        let done = task.status == TaskStatus::Done;
        let mut left = vec![
            Span::styled(glyph, theme.fg(color)),
            Span::raw(" "),
            Span::styled(format!("#{} ", task.id.0), theme.dim()),
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
            // Reading mode's Ask lives in its column only (ADR 0018).
            .filter(|a| a.task == Some(task.id) && a.parent.is_none() && a.kind != AgentKind::Ask)
            .collect();
        rows.push(Row {
            key,
            depth: 1,
            fold: (!sessions.is_empty()).then_some(open),
            left,
            right,
            status: None,
            spinner_versions: 0,
        });
        if open && group != Group::Done {
            for agent in sessions {
                self.agent_rows(cx, agent, 2, rows);
            }
        }
    }

    fn agent_rows(&self, cx: &ViewCx, agent: &Agent, depth: usize, rows: &mut Vec<Row>) {
        let (snap, theme) = (cx.snap, cx.theme);
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
        let working = cx
            .activity
            .working_for(&agent.id, cx.now_ms)
            .map(|since_ms| {
                let entries = snap
                    .conversations
                    .get(&agent.id)
                    .map_or(&[][..], |t| &t.entries);
                Working {
                    label: working::label_of(agent, entries),
                    since_ms,
                    now_ms: cx.now_ms,
                }
            });
        let (left, right, spinner_versions) = agent_line(theme, agent, working.as_ref());
        let status = Some(agent.status_line.trim().to_owned()).filter(|s| !s.is_empty());
        rows.push(Row {
            key,
            depth,
            fold: foldable.then_some(open),
            left,
            right,
            status,
            spinner_versions,
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
                spinner_versions: 0,
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
                spinner_versions: 0,
            });
        }
        for child in children {
            self.agent_rows(cx, child, depth + 1, rows);
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
pub(crate) fn agents_waiting(snap: &Snapshot) -> HashSet<TaskId> {
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

/// The open tasks and the live sessions, for the pane title.
pub(crate) fn counts(snap: &Snapshot) -> (usize, usize) {
    let tasks = snap
        .tasks
        .iter()
        .filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Archived))
        .count();
    let agents = snap
        .agents
        .iter()
        .filter(|a| a.kind.is_session() && a.status != AgentStatus::Done)
        .count();
    (tasks, agents)
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

/// A working subagent with progress shows that instead of a spinner.
fn word_is_progress(agent: &Agent) -> bool {
    matches!(
        (&agent.kind, &agent.progress),
        (AgentKind::Subagent { .. }, Some(p)) if p.summary.is_none()
    )
}

/// A working agent's spinner, as the chat's working line shows it.
struct Working {
    label: String,
    since_ms: i64,
    now_ms: i64,
}

/// Longest working label in a tree row, in cells.
const LABEL_MAX: usize = 24;
/// Longest model name in a tree row, in cells.
const MODEL_MAX: usize = 12;

/// Model and effort, dim: `  sonnet m`. Nothing for the coordinator (the usage line shows its
/// model) or while neither is known.
fn model_span(theme: &Theme, agent: &Agent, with_effort: bool) -> Option<Span<'static>> {
    if agent.kind == AgentKind::Coordinator {
        return None;
    }
    let model = agent
        .model
        .as_deref()
        .map(|m| fmt::truncate(usage::model_family(m), MODEL_MAX));
    let effort = agent
        .effort
        .filter(|_| with_effort)
        .map(usage::effort_shorthand);
    let text = [model.as_deref(), effort]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then(|| Span::styled(format!("  {text}"), theme.dim()))
}

fn agent_line(
    theme: &Theme,
    agent: &Agent,
    working: Option<&Working>,
) -> (Vec<Span<'static>>, Vec<Vec<Span<'static>>>, usize) {
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
        // A side chat runs read-only whatever its mode shows (decided 2026-11-11).
        _ if agent.kind == AgentKind::SideChat => {
            left.push(Span::raw(" "));
            left.push(Span::styled(" read-only ", theme.badge(theme.italic)));
        }
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
    // Taken over (M14): native claude drives it, not Dovetail.
    if agent.lock == SessionLock::TakenOver {
        left.push(Span::raw(" "));
        left.push(Span::styled(" native ", theme.badge(theme.needs_you)));
    }
    let word_style = if agent.status == AgentStatus::NeedsYou {
        theme.fg(theme.needs_you)
    } else if agent.status == AgentStatus::Working {
        // Working reads differently from idle at a glance; the glyph itself stays still.
        theme.fg(theme.working)
    } else {
        theme.dim()
    };
    // A running subagent shows its progress instead of a status word; a narrow pane drops the
    // token count first.
    let (word, short_word) = match (&agent.kind, &agent.progress) {
        (AgentKind::Subagent { .. }, Some(p))
            if p.summary.is_none() && agent.status == AgentStatus::Working =>
        {
            let last = p.last_tool.as_deref().unwrap_or("starting");
            let short = format!("{} tools · {last}", p.tool_uses);
            let full = if p.total_tokens > 0 {
                format!("{short} · {} tok", fmt::tokens(p.total_tokens))
            } else {
                short.clone()
            };
            (
                Span::styled(full, theme.dim()),
                Span::styled(short, theme.dim()),
            )
        }
        _ => {
            let word = Span::styled(status_word(agent.status), word_style);
            (word.clone(), word)
        }
    };
    // Context fill shows for slices (Decision 22): plain, amber above 60 %, red above 80 %.
    let context = agent
        .context_pct
        .filter(|_| agent.kind == AgentKind::Slice)
        .map(|p| Span::styled(format!(" {p}%"), theme.context(p)));
    // A working slice or subagent shows the chat's spinner line in place of the status word; its
    // timer is the first thing a narrow pane drops. The coordinator keeps its word.
    let spinner = working.filter(|_| {
        agent.status == AgentStatus::Working
            && agent.kind != AgentKind::Coordinator
            && !word_is_progress(agent)
    });
    let status = |timed: bool| -> Vec<Span<'static>> {
        match spinner {
            Some(w) => {
                let time = timed.then(|| activity::elapsed(w.since_ms));
                let label = fmt::truncate(&w.label, LABEL_MAX);
                working::spinner_spans(theme, &label, time, w.now_ms)
            }
            None => vec![word.clone()],
        }
    };
    let (timed_status, plain_status) = (status(true), status(false));
    // Fullest first. A narrow pane drops the timer, then the effort, then the model, then the
    // spinner's label for the plain word (a slice keeps its context fill as long as it can), then
    // the context fill.
    let build = |status: &[Span<'static>], ctx: bool, model: Option<Span<'static>>| {
        let mut spans = status.to_vec();
        spans.extend(context.clone().filter(|_| ctx));
        spans.extend(model);
        spans
    };
    let model = |effort| model_span(theme, agent, effort);
    let word = std::slice::from_ref(&word);
    let mut right = vec![
        build(&timed_status, true, model(true)),
        build(&plain_status, true, model(true)),
        build(&plain_status, true, model(false)),
        build(&plain_status, true, None),
    ];
    // Without a spinner these four repeat the word, which is harmless.
    let spinner_versions = if spinner.is_some() { right.len() } else { 0 };
    right.extend([
        build(word, true, None),
        build(word, false, None),
        vec![short_word],
    ]);
    (left, right, spinner_versions)
}

impl Buffer for TreeBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Tree
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let (tasks, agents) = counts(cx.snap);
        let inner = pane(
            frame,
            area,
            title(
                &format!("tasks {tasks} · agents {agents}"),
                focused,
                cx.theme,
            ),
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
            if pick_right(spans_width(&left), &row.right, width) < row.spinner_versions {
                self.shows_working = true;
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
                // On a slice's or side chat's row, `dd` archives it; elsewhere, the task with its
                // slices.
                let slice = match self.selected_key(cx) {
                    Some(RowKey::Agent(id)) => cx
                        .snap
                        .agents
                        .iter()
                        .find(|a| {
                            a.id == id && matches!(a.kind, AgentKind::Slice | AgentKind::SideChat)
                        })
                        .map(|a| a.id.clone()),
                    _ => None,
                };
                match slice {
                    Some(agent) => effects.push(Effect::Command(Command::CloseSlice { agent })),
                    None => self.task_action(cx, effects, |t| {
                        let archive = Command::ArchiveTask { id: t.id };
                        // An open or queued research round is closed or dropped by archiving
                        // (findings are kept): ask first, like unsaved work.
                        match super::blocking_round(cx.snap, t.id) {
                            Some(round) => Effect::Confirm {
                                prompt: format!(
                                    "Task #{} has {round}. Close it and archive? y/n",
                                    t.id.0
                                ),
                                then: vec![archive],
                            },
                            None => Effect::Command(archive),
                        }
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

#[cfg(test)]
mod tests {
    use dovetail_api::{AgentId, SessionLock, SubagentProgress};

    use super::*;

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn subagent(status: AgentStatus, summary: Option<&str>) -> Agent {
        Agent {
            id: AgentId("coordinator:toolu_1".into()),
            name: "Background sleep task".into(),
            kind: AgentKind::Subagent { background: true },
            parent: Some(AgentId("coordinator".into())),
            task: None,
            worktree: None,
            status,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: Some(SubagentProgress {
                description: "Background sleep task".into(),
                last_tool: Some("Bash".into()),
                tool_uses: 2,
                summary: summary.map(str::to_owned),
                total_tokens: 17_164,
                ..SubagentProgress::default()
            }),
            last_activity_ms: 0,
            lock: SessionLock::Headless,
            model: None,
            effort: None,
            subagent_tokens: 0,
            tool: None,
            driver: dovetail_api::DriverKind::Claude,
        }
    }

    #[test]
    fn a_running_subagent_shows_its_progress_and_drops_tokens_when_narrow() {
        let theme = Theme::default();
        let (left, right, _) = agent_line(&theme, &subagent(AgentStatus::Working, None), None);
        assert_eq!(text(&left), "◐ ◇ Background sleep task");
        assert_eq!(text(&right[0]), "2 tools · Bash · 17k tok");
        assert_eq!(text(right.last().expect("narrowest")), "2 tools · Bash");
    }

    #[test]
    fn a_finished_subagent_shows_its_status_word() {
        let theme = Theme::default();
        let (_, right, _) = agent_line(&theme, &subagent(AgentStatus::Done, Some("bg-done")), None);
        assert_eq!(text(&right[0]), "done");
    }

    fn slice(status: AgentStatus) -> Agent {
        Agent {
            id: AgentId("slice-a".into()),
            name: "slice-a".into(),
            kind: AgentKind::Slice,
            parent: None,
            progress: None,
            status,
            context_pct: Some(41),
            model: Some("claude-sonnet-5-5".into()),
            effort: Some(dovetail_api::ReasoningEffort::Medium),
            ..subagent(status, None)
        }
    }

    /// The right side `agent_line` picks at `width`, as `spread_fit` would.
    fn right_at(agent: &Agent, working: Option<&Working>, width: usize) -> String {
        let theme = Theme::default();
        let (left, right, _) = agent_line(&theme, agent, working);
        let line = spread_fit(left.clone(), &right, width);
        let all = text(&line.spans);
        let left_w = text(&left).width();
        all.chars()
            .skip(left_w.min(all.chars().count()))
            .collect::<String>()
    }

    fn working_now() -> Working {
        Working {
            label: "Dovetailing…".into(),
            since_ms: 12_000,
            now_ms: 0,
        }
    }

    #[test]
    fn a_working_slice_row_reads_status_then_context_then_model() {
        let agent = slice(AgentStatus::Working);
        let wide = right_at(&agent, Some(&working_now()), 120);
        assert!(
            wide.ends_with("⠋ Dovetailing… 12s 41%  sonnet m"),
            "{wide:?}"
        );
    }

    #[test]
    fn an_idle_slice_row_keeps_its_status_word() {
        let agent = slice(AgentStatus::Idle);
        let wide = right_at(&agent, None, 120);
        assert!(wide.ends_with("idle 41%  sonnet m"), "{wide:?}");
    }

    #[test]
    fn a_narrow_row_drops_timer_then_effort_then_model() {
        let agent = slice(AgentStatus::Working);
        let w = Some(&working_now());
        let left = text(&agent_line(&Theme::default(), &agent, None).0).width();
        let right = |extra: usize| right_at(&agent, w, left + extra).trim().to_owned();
        assert_eq!(right(34), "⠋ Dovetailing… 12s 41%  sonnet m");
        assert_eq!(right(30), "⠋ Dovetailing… 41%  sonnet m");
        assert_eq!(right(28), "⠋ Dovetailing… 41%  sonnet");
        assert_eq!(right(19), "⠋ Dovetailing… 41%");
        assert_eq!(right(15), "working 41%");
        assert_eq!(right(8), "working");
    }

    #[test]
    fn unknown_model_and_effort_are_left_out() {
        let mut agent = slice(AgentStatus::Idle);
        agent.effort = None;
        assert!(right_at(&agent, None, 120).ends_with("idle 41%  sonnet"));
        agent.model = None;
        assert!(right_at(&agent, None, 120).ends_with("idle 41%"));
    }

    #[test]
    fn the_coordinator_row_shows_no_model() {
        let mut agent = slice(AgentStatus::Idle);
        agent.kind = AgentKind::Coordinator;
        agent.context_pct = None;
        assert_eq!(right_at(&agent, None, 120).trim(), "idle");
        agent.status = AgentStatus::Working;
        assert_eq!(
            right_at(&agent, Some(&working_now()), 120).trim(),
            "working"
        );
    }
}

//! The worktree view (`<leader>g`): one row per slice worktree with its branch, sync state,
//! uncommitted changes, unpushed commits, pull request and setup, and the selected row in full
//! below. The data is the core's ([`WorktreeInfo`], M12b); opening the view asks for a fresh
//! look ([`Command::RefreshWorktrees`]) and the title says "refreshing…" until every worktree
//! has answered. Anything the core could not tell shows as `—`.

use std::collections::HashSet;

use dovetail_api::{
    Agent, AgentId, AgentKind, Changes, Command, Event, PrLookup, PrState, Snapshot, SyncState,
    WorktreeInfo, WorktreeRef, WorktreeSetup,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::{
    Buffer, Effect, ViewCx, clip_spans, footer, pane, scroll_to, spans_width, status_glyph, step,
    title,
};
use crate::action::Action;
use crate::region::BufferKind;
use crate::theme::Theme;

/// What a cell shows when the core could not tell.
const UNKNOWN: &str = "—";

/// Column headings, left to right.
const HEADINGS: [&str; 7] = [
    "session", "branch", "sync", "changes", "unpushed", "PR", "setup",
];
/// Index of the branch column, the one left out when the row would not fit.
const BRANCH: usize = 1;
/// Blank cells between two columns.
const GAP: usize = 2;
/// Rows the detail section takes: a blank, a rule, a blank, the name line and eight fields.
const DETAIL_ROWS: u16 = 12;

/// The worktree buffer: the selected slice, kept by id so live updates never move the cursor.
#[derive(Default)]
pub struct WorktreesBuffer {
    selected: Option<AgentId>,
    /// Where the selection was last, for when its row goes away.
    last_index: usize,
    offset: usize,
    /// Worktrees asked about by the last refresh that have not answered yet.
    refreshing: HashSet<AgentId>,
}

/// One slice worktree and what the core last found in it.
struct Row<'a> {
    agent: &'a Agent,
    worktree: &'a WorktreeRef,
    info: Option<&'a WorktreeInfo>,
}

impl Row<'_> {
    fn path(&self) -> &std::path::Path {
        self.info.map_or(&self.worktree.path, |i| &i.path)
    }

    fn branch(&self) -> &str {
        self.info.map_or(&self.worktree.branch, |i| &i.branch)
    }
}

impl WorktreesBuffer {
    /// The view opened: every slice worktree is waiting for the refresh the app sends now.
    pub fn begin_refresh(&mut self, snap: &Snapshot) {
        self.refreshing = rows(snap).iter().map(|r| r.agent.id.clone()).collect();
    }

    /// Whether some worktree has not answered the last refresh yet.
    pub fn is_refreshing(&self) -> bool {
        !self.refreshing.is_empty()
    }

    /// Notes a worktree's answer (or its agent's removal) to the refresh.
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::WorktreeUpserted(info) => {
                self.refreshing.remove(&info.agent);
            }
            Event::AgentRemoved(id) => {
                self.refreshing.remove(id);
            }
            _ => {}
        }
    }

    /// The slice on the selected row.
    pub fn selected_agent(&self, snap: &Snapshot) -> Option<AgentId> {
        let rows = rows(snap);
        self.resolve(&rows).map(|i| rows[i].agent.id.clone())
    }

    fn resolve(&self, rows: &[Row]) -> Option<usize> {
        if rows.is_empty() {
            return None;
        }
        if let Some(id) = &self.selected
            && let Some(i) = rows.iter().position(|r| &r.agent.id == id)
        {
            return Some(i);
        }
        Some(self.last_index.min(rows.len() - 1))
    }

    fn select(&mut self, rows: &[Row], index: usize) {
        self.selected = Some(rows[index].agent.id.clone());
        self.last_index = index;
    }
}

impl Buffer for WorktreesBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Worktrees
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let rows = rows(cx.snap);
        let mut heading = match &cx.snap.project {
            Some(p) => format!("worktrees · {} · {}", p.name, rows.len()),
            None => format!("worktrees · {}", rows.len()),
        };
        if self.is_refreshing() {
            heading.push_str(" · refreshing…");
        }
        let inner = pane(frame, area, title(&heading, focused, theme), focused, theme);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let width = usize::from(inner.width);
        if rows.is_empty() {
            let text = "No slice worktrees. A slice gets one when it starts (␣sn).";
            frame.render_widget(
                Paragraph::new(Span::styled(crate::fmt::truncate(text, width), theme.dim())),
                Rect { height: 1, ..inner },
            );
            return;
        }

        // The footer takes the last row and a blank above it, when there is room.
        let reserved = if inner.height >= 5 { 2 } else { 0 };
        let usable = inner.height.saturating_sub(reserved);
        // List first; the detail goes below it only if the list still gets two rows.
        let wanted = 1 + rows.len() as u16;
        let detail = usable >= DETAIL_ROWS + 3;
        let list_height = if detail {
            wanted.min(usable - DETAIL_ROWS)
        } else {
            usable
        };

        let cells: Vec<[Vec<Span<'static>>; 7]> =
            rows.iter().map(|r| row_cells(r, theme)).collect();
        let widths = column_widths(&cells, width);
        let selected = self.resolve(&rows);
        let fit = usize::from(list_height.saturating_sub(1)).max(1);
        if let Some(i) = selected {
            self.offset = scroll_to(self.offset, i, i, fit);
        }
        self.offset = self.offset.min(rows.len().saturating_sub(fit));

        let header: [Vec<Span<'static>>; 7] =
            HEADINGS.map(|h| vec![Span::styled(h.to_owned(), theme.dim())]);
        let mut lines = vec![table_line(header, &widths, width)];
        for cells in cells.into_iter().skip(self.offset).take(fit) {
            lines.push(table_line(cells, &widths, width));
        }
        let list_area = Rect {
            height: list_height,
            ..inner
        };
        frame.render_widget(Paragraph::new(lines), list_area);
        if focused && let Some(i) = selected {
            // The whole row is highlighted, not just its text.
            let y = list_area.y + 1 + (i - self.offset) as u16;
            if y < list_area.y + list_area.height {
                frame.buffer_mut().set_style(
                    Rect {
                        y,
                        height: 1,
                        ..inner
                    },
                    theme.selected(),
                );
            }
        }

        if detail && let Some(i) = selected {
            let lines = detail_lines(cx, &rows[i], width);
            let detail_area = Rect {
                y: inner.y + list_height,
                height: usable - list_height,
                ..inner
            };
            frame.render_widget(Paragraph::new(lines), detail_area);
        }
        footer(
            frame,
            inner,
            &[
                "⏎ open session · e open in editor · c copy path · X remove (asks if work is unpushed)",
                "⏎ open · e editor · c copy path · X remove",
                "⏎ e c X",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let rows = rows(cx.snap);
        let current = self.resolve(&rows);
        match action {
            Action::MoveDown | Action::MoveUp => {
                if let Some(i) = current {
                    let delta = if action == Action::MoveDown { 1 } else { -1 };
                    self.select(&rows, step(i, rows.len(), delta));
                }
            }
            Action::GoTop if !rows.is_empty() => self.select(&rows, 0),
            Action::GoBottom if !rows.is_empty() => self.select(&rows, rows.len() - 1),
            Action::WorktreeOpen
            | Action::WorktreeEdit
            | Action::WorktreeCopyPath
            | Action::WorktreeRemove => {
                let Some(row) = current.map(|i| &rows[i]) else {
                    effects.push(Effect::Note("no worktree here".into()));
                    return true;
                };
                let path = row.path().to_path_buf();
                let agent = row.agent.id.clone();
                match action {
                    Action::WorktreeOpen => effects.push(Effect::OpenAgent(agent)),
                    Action::WorktreeEdit => effects.push(Effect::OpenEditor(path)),
                    Action::WorktreeCopyPath => {
                        // A failure comes back as `CommandFailed`, which replaces the note.
                        effects.push(Effect::Command(Command::CopyToClipboard {
                            text: path.display().to_string(),
                        }));
                        effects.push(Effect::Note(format!("copied {}", path.display())));
                    }
                    // The archive path, as `dd` on a slice in the tree: the core removes a
                    // clean, pushed worktree at once and asks first (a Permission item) when
                    // work is unpushed or uncommitted.
                    _ => {
                        effects.push(Effect::Command(Command::CloseSlice { agent }));
                        effects.push(Effect::Note(format!("removing {}…", row.agent.name)));
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        rows(cx.snap)
            .iter()
            .map(|r| format!("{} {}", r.agent.name, r.branch()))
            .collect()
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        self.resolve(&rows(cx.snap)).unwrap_or(0)
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, _effects: &mut Vec<Effect>) {
        let rows = rows(cx.snap);
        if row < rows.len() {
            self.select(&rows, row);
        }
    }
}

/// Every slice that has a worktree, by name, with what the core last found in it.
fn rows(snap: &Snapshot) -> Vec<Row<'_>> {
    let mut rows: Vec<Row> = snap
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Slice)
        .filter_map(|agent| {
            Some(Row {
                agent,
                worktree: agent.worktree.as_ref()?,
                info: snap.worktrees.iter().find(|w| w.agent == agent.id),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.agent.name.cmp(&b.agent.name));
    rows
}

fn unknown(theme: &Theme) -> Vec<Span<'static>> {
    vec![Span::styled(UNKNOWN, theme.dim())]
}

/// The seven cells of a row.
fn row_cells(row: &Row, theme: &Theme) -> [Vec<Span<'static>>; 7] {
    let (glyph, color) = status_glyph(row.agent.status, theme);
    let name = vec![
        Span::styled(glyph, theme.fg(color)),
        Span::raw(" "),
        Span::styled(row.agent.name.clone(), theme.body()),
    ];
    let branch = vec![Span::styled(row.branch().to_owned(), theme.fg(theme.code))];
    let Some(info) = row.info else {
        return [
            name,
            branch,
            unknown(theme),
            unknown(theme),
            unknown(theme),
            unknown(theme),
            unknown(theme),
        ];
    };
    let changes = match info.changes {
        None => unknown(theme),
        Some(c) if c.is_clean() => vec![Span::styled("clean", theme.dim())],
        Some(c) => {
            let color = if c.conflicted > 0 {
                theme.error
            } else {
                theme.needs_you
            };
            vec![Span::styled(
                format!("●{}", c.staged + c.unstaged + c.untracked + c.conflicted),
                theme.fg(color),
            )]
        }
    };
    let unpushed = match info.unpushed {
        None => unknown(theme),
        Some(0) => vec![Span::styled("0", theme.dim())],
        Some(n) => vec![Span::styled(n.to_string(), theme.fg(theme.needs_you))],
    };
    [
        name,
        branch,
        sync_spans(info.sync, theme),
        changes,
        unpushed,
        pr_spans(&info.pr, theme),
        setup_spans(&info.setup, theme, false),
    ]
}

/// `↑ahead ↓behind`, plus `origin ↓n` when the remote base has moved on.
fn sync_spans(sync: Option<SyncState>, theme: &Theme) -> Vec<Span<'static>> {
    let Some(s) = sync else {
        return unknown(theme);
    };
    let count = |n: u32, warn: bool| {
        if n > 0 && warn {
            theme.fg(theme.needs_you)
        } else if n > 0 {
            theme.body()
        } else {
            theme.dim()
        }
    };
    let mut spans = vec![
        Span::styled(format!("↑{}", s.ahead), count(s.ahead, false)),
        Span::raw(" "),
        Span::styled(format!("↓{}", s.behind), count(s.behind, true)),
    ];
    if let Some(n) = s.behind_origin.filter(|&n| n > 0) {
        spans.push(Span::styled(
            format!(" origin ↓{n}"),
            theme.fg(theme.needs_you),
        ));
    }
    spans
}

fn pr_spans(pr: &PrLookup, theme: &Theme) -> Vec<Span<'static>> {
    match pr {
        PrLookup::Unknown { .. } => unknown(theme),
        PrLookup::None => vec![Span::styled("none", theme.dim())],
        PrLookup::Found(pr) => {
            let (word, color) = match pr.state {
                PrState::Open => ("open", theme.working),
                PrState::Merged => ("merged", theme.ok),
                PrState::Closed => ("closed", theme.dim),
            };
            vec![
                Span::styled(format!("#{} ", pr.number), theme.body()),
                Span::styled(word, theme.fg(color)),
            ]
        }
    }
}

/// The setup state; `full` adds a failure's message.
fn setup_spans(setup: &WorktreeSetup, theme: &Theme, full: bool) -> Vec<Span<'static>> {
    match setup {
        WorktreeSetup::None => vec![Span::styled("none", theme.dim())],
        WorktreeSetup::Running => vec![Span::styled("◌ running", theme.fg(theme.working))],
        WorktreeSetup::Done => vec![Span::styled("✓ done", theme.fg(theme.ok))],
        WorktreeSetup::Failed { error } => {
            let mut spans = vec![Span::styled("✗ failed", theme.fg(theme.error))];
            if full && let Some(e) = error {
                spans.push(Span::styled(
                    format!(": {}", crate::fmt::first_line(e)),
                    theme.body(),
                ));
            }
            spans
        }
    }
}

fn changes_spans(changes: Option<Changes>, theme: &Theme) -> Vec<Span<'static>> {
    let Some(c) = changes else {
        return unknown(theme);
    };
    if c.is_clean() {
        return vec![Span::styled("clean", theme.dim())];
    }
    let parts = [
        (c.staged, "staged", theme.needs_you),
        (c.unstaged, "changed", theme.needs_you),
        (c.untracked, "untracked", theme.needs_you),
        (c.conflicted, "conflicted", theme.error),
    ];
    let mut spans = Vec::new();
    for (n, word, color) in parts.into_iter().filter(|(n, ..)| *n > 0) {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme.dim()));
        }
        spans.push(Span::styled(format!("{n} {word}"), theme.fg(color)));
    }
    spans
}

/// Column widths: each column's widest cell. When the row would not fit, the branch column goes
/// (width 0): it is `dovetail/<name>` for every slice and the detail below shows it. Anything
/// still too wide is cut at the end of the row.
fn column_widths(rows: &[[Vec<Span<'static>>; 7]], width: usize) -> [usize; 7] {
    let mut widths = HEADINGS.map(UnicodeWidthStr::width);
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(spans_width(cell));
        }
    }
    let total: usize = widths.iter().sum::<usize>() + GAP * (widths.len() - 1);
    if total > width {
        widths[BRANCH] = 0;
    }
    widths
}

/// One table row: cells padded to their column widths (a column of width 0 is left out), cut
/// to `width`.
fn table_line(cells: [Vec<Span<'static>>; 7], widths: &[usize; 7], width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let shown = cells.into_iter().zip(widths).filter(|(_, w)| **w > 0);
    for (n, (cell, &w)) in shown.enumerate() {
        if n > 0 {
            spans.push(Span::raw(" ".repeat(GAP)));
        }
        let cell = clip_spans(cell, w);
        let pad = w.saturating_sub(spans_width(&cell));
        spans.extend(cell);
        spans.push(Span::raw(" ".repeat(pad)));
    }
    Line::from(clip_spans(spans, width))
}

/// The selected worktree in full: a blank, a dashed rule, then its name, path, base, sync, changes,
/// unpushed commits, pull request, setup and when it was last looked at.
fn detail_lines(cx: &ViewCx, row: &Row, width: usize) -> Vec<Line<'static>> {
    let theme = cx.theme;
    let label = |text: &str| Span::styled(format!("{text:<9}"), theme.dim());
    let mut lines = vec![
        Line::default(),
        Line::from(Span::styled("╌".repeat(width), theme.fg(theme.line))),
        Line::default(),
        Line::from(vec![
            Span::styled(
                row.agent.name.clone(),
                theme.fg(theme.bold).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ⎇ ", theme.dim()),
            Span::styled(row.branch().to_owned(), theme.fg(theme.code)),
        ]),
        Line::from(vec![
            label("path"),
            Span::styled(row.path().display().to_string(), theme.fg(theme.code)),
        ]),
    ];
    let info = row.info;
    let base = info.map_or_else(
        || unknown(theme),
        |i| vec![Span::styled(i.base.clone(), theme.body())],
    );
    lines.push(Line::from([vec![label("base")], base].concat()));
    let sync = info.map_or_else(|| unknown(theme), |i| sync_spans(i.sync, theme));
    lines.push(Line::from([vec![label("sync")], sync].concat()));
    let changes = info.map_or_else(|| unknown(theme), |i| changes_spans(i.changes, theme));
    lines.push(Line::from([vec![label("changes")], changes].concat()));
    let unpushed = match info.and_then(|i| i.unpushed) {
        None => unknown(theme),
        Some(0) => vec![Span::styled("none", theme.dim())],
        Some(1) => vec![Span::styled("1 commit", theme.fg(theme.needs_you))],
        Some(n) => vec![Span::styled(
            format!("{n} commits"),
            theme.fg(theme.needs_you),
        )],
    };
    lines.push(Line::from([vec![label("unpushed")], unpushed].concat()));
    let pr = match info.map(|i| &i.pr) {
        None => unknown(theme),
        Some(PrLookup::Unknown { reason }) => vec![
            Span::styled(UNKNOWN, theme.dim()),
            Span::styled(format!(" ({reason})"), theme.dim()),
        ],
        Some(PrLookup::Found(pr)) => {
            let mut spans = pr_spans(&PrLookup::Found(pr.clone()), theme);
            spans.push(Span::styled(" · ", theme.dim()));
            spans.push(Span::styled(pr.url.clone(), theme.fg(theme.code)));
            spans
        }
        Some(other) => pr_spans(other, theme),
    };
    lines.push(Line::from([vec![label("PR")], pr].concat()));
    let setup = info.map_or_else(|| unknown(theme), |i| setup_spans(&i.setup, theme, true));
    lines.push(Line::from([vec![label("setup")], setup].concat()));
    let checked = info.map_or_else(
        || unknown(theme),
        |i| {
            vec![Span::styled(
                crate::fmt::when(i.checked_ms, cx.tz),
                theme.dim(),
            )]
        },
    );
    lines.push(Line::from([vec![label("checked")], checked].concat()));
    lines
        .into_iter()
        .map(|l| Line::from(clip_spans(l.spans, width)))
        .collect()
}

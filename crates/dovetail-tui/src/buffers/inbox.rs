//! Needs you: permission requests, questions, finished turns and errors, with a context
//! preview of the selected item.

use dovetail_api::{
    AgentId, Command, EntryKind, InboxId, InboxItem, InboxKind, PermissionAnswer,
    REMOVE_WORKTREE_PERMISSION, START_SLICE_PERMISSION, Snapshot,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{Buffer, Effect, InputTarget, ViewCx, footer, pane, scroll_to, spread, step, title};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;

/// Inbox items in display order: permission requests first, then questions, errors and finished
/// turns; newest first within each.
pub fn ordered(snap: &Snapshot) -> Vec<&InboxItem> {
    let mut items: Vec<&InboxItem> = snap.inbox.iter().collect();
    items.sort_by_key(|i| (rank(&i.kind), std::cmp::Reverse((i.at_ms, i.id))));
    items
}

fn rank(kind: &InboxKind) -> u8 {
    match kind {
        InboxKind::Permission { .. } => 0,
        InboxKind::Question { .. } => 1,
        InboxKind::Error { .. } => 2,
        InboxKind::FinishedTurn { .. } => 3,
    }
}

/// The inbox buffer.
#[derive(Default)]
pub struct InboxBuffer {
    selected: Option<InboxId>,
    last_index: usize,
    offset: usize,
}

impl InboxBuffer {
    /// Index of the selected item, or of the item nearest where it was.
    fn resolve(&self, items: &[&InboxItem]) -> Option<usize> {
        if items.is_empty() {
            return None;
        }
        if let Some(id) = self.selected
            && let Some(i) = items.iter().position(|item| item.id == id)
        {
            return Some(i);
        }
        Some(self.last_index.min(items.len() - 1))
    }

    /// The selected item.
    pub fn selected<'a>(&self, snap: &'a Snapshot) -> Option<&'a InboxItem> {
        let items = ordered(snap);
        self.resolve(&items).map(|i| items[i])
    }

    /// Moves the cursor to an item, if it is still there.
    pub fn select_item(&mut self, snap: &Snapshot, id: InboxId) {
        let items = ordered(snap);
        if let Some(i) = items.iter().position(|item| item.id == id) {
            self.select(&items, i);
        }
    }

    fn select(&mut self, items: &[&InboxItem], index: usize) {
        self.selected = Some(items[index].id);
        self.last_index = index;
    }
}

fn last_message(snap: &Snapshot, agent: &AgentId) -> Option<String> {
    snap.conversations
        .get(agent)?
        .entries
        .iter()
        .rev()
        .find(|e| e.kind == EntryKind::Assistant && !e.text.trim().is_empty())
        .map(|e| e.text.trim().to_string())
}

/// What a permission request is about, in full: a command, a path or the raw input.
pub(crate) fn input_summary(snap: &Snapshot, tool: &str, input: &serde_json::Value) -> String {
    let field = |name: &str| input.get(name).and_then(|v| v.as_str());
    match tool {
        "Bash" => field("command").map(str::to_string),
        "Edit" | "Write" | "Read" | "MultiEdit" => field("file_path").map(str::to_string),
        START_SLICE_PERMISSION => Some(start_slice_summary(snap, input)),
        REMOVE_WORKTREE_PERMISSION => Some(remove_worktree_summary(input)),
        _ => None,
    }
    .unwrap_or_else(|| input.to_string())
}

/// `start slice demo-1 on #1 demo`.
fn start_slice_summary(snap: &Snapshot, input: &serde_json::Value) -> String {
    let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("?");
    let task = input.get("task").and_then(|v| v.as_i64());
    match task {
        Some(id) => {
            let title = snap
                .tasks
                .iter()
                .find(|t| t.id.0 == id)
                .map(|t| format!(" {}", t.title))
                .unwrap_or_default();
            format!("start slice {name} on #{id}{title}")
        }
        None => format!("start slice {name}"),
    }
}

/// `dovetail/demo-1 · 1 unpushed commit · no uncommitted changes`.
fn remove_worktree_summary(input: &serde_json::Value) -> String {
    let branch = input
        .get("branch")
        .and_then(|v| v.as_str())
        .unwrap_or("the worktree");
    let unpushed = input
        .get("unpushed_commits")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let uncommitted = input
        .get("uncommitted")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let unpushed = match unpushed {
        0 => "nothing unpushed".to_owned(),
        n => plural(n as usize, "unpushed commit", "unpushed commits"),
    };
    let uncommitted = match uncommitted {
        0 => "no uncommitted changes".to_owned(),
        n => plural(n, "uncommitted change", "uncommitted changes"),
    };
    // A folder that is not a git worktree: what `y` would delete is its files.
    if let Some(files) = input.get("files").and_then(|v| v.as_u64()) {
        let folder = input
            .get("worktree")
            .and_then(|v| v.as_str())
            .unwrap_or("the folder");
        let files = plural(files as usize, "file", "files");
        return format!("{folder} · not a git worktree · {files}");
    }
    format!("{branch} · {unpushed} · {uncommitted}")
}

fn item_label(snap: &Snapshot, item: &InboxItem) -> (&'static str, String) {
    let _ = snap;
    match &item.kind {
        InboxKind::Permission { tool, .. } if tool == START_SLICE_PERMISSION => {
            ("⚠", "Start slice".into())
        }
        InboxKind::Permission { tool, .. } => ("⚠", tool.clone()),
        InboxKind::Question { .. } => ("?", "Question".into()),
        InboxKind::FinishedTurn { .. } => ("✓", "Finished turn".into()),
        InboxKind::Error { .. } => ("✗", "Error".into()),
    }
}

impl Buffer for InboxBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::Inbox
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let items = ordered(cx.snap);
        let inner = pane(
            frame,
            area,
            title(
                &format!("inbox · needs you ({})", items.len()),
                focused,
                theme,
            ),
            focused,
            theme,
        );
        let width = usize::from(inner.width);
        let selected = self.resolve(&items);
        if let Some(i) = selected {
            self.select(&items, i);
        }

        // Build every line, remembering where the selected row and its preview sit.
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut selected_rows = (0, 0);
        for (i, item) in items.iter().enumerate() {
            let name = cx
                .snap
                .agents
                .iter()
                .find(|a| a.id == item.agent)
                .map_or(item.agent.0.as_str(), |a| a.name.as_str());
            let (glyph, label) = item_label(cx.snap, item);
            let color = match &item.kind {
                InboxKind::Permission { .. } | InboxKind::Question { .. } => theme.needs_you,
                InboxKind::FinishedTurn { .. } => theme.ok,
                InboxKind::Error { .. } => theme.error,
            };
            let is_selected = selected == Some(i);
            let left = vec![
                Span::styled(
                    if is_selected { "▸ " } else { "  " },
                    theme.fg(theme.needs_you),
                ),
                Span::styled(glyph, theme.fg(color)),
                Span::raw(" "),
                Span::styled(name.to_string(), theme.fg(color)),
                Span::styled(format!("  {label}"), theme.body()),
            ];
            let right = vec![Span::styled(fmt::hhmm(item.at_ms, cx.tz), theme.dim())];
            let row = lines.len();
            lines.push(spread(left, right, width));
            if is_selected {
                selected_rows.0 = row;
                lines.extend(preview(cx, item, width));
                selected_rows.1 = lines.len() - 1;
            }
        }

        let body_height = usize::from(inner.height.saturating_sub(u16::from(inner.height >= 5)));
        if selected.is_some() {
            self.offset = scroll_to(self.offset, selected_rows.0, selected_rows.1, body_height);
        }
        self.offset = self.offset.min(lines.len().saturating_sub(body_height));
        for (n, line) in lines
            .into_iter()
            .skip(self.offset)
            .take(body_height)
            .enumerate()
        {
            let rect = Rect {
                y: inner.y + n as u16,
                height: 1,
                ..inner
            };
            let is_row = self.offset + n == selected_rows.0 && selected.is_some();
            frame.render_widget(Paragraph::new(line), rect);
            if is_row && focused {
                frame.buffer_mut().set_style(rect, theme.selected());
            }
        }
        footer(
            frame,
            inner,
            &[
                "y allow · A always · d deny · D reason · r reply · x dismiss · ⏎ open",
                "y allow · A always · d deny · D reason · ⏎ open",
                "y allow · d deny · r reply · x dismiss",
            ],
            theme,
        );
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let items = ordered(cx.snap);
        let current = self.resolve(&items);
        match action {
            Action::MoveDown | Action::MoveUp => {
                if let Some(i) = current {
                    let delta = if action == Action::MoveDown { 1 } else { -1 };
                    self.select(&items, step(i, items.len(), delta));
                }
                return true;
            }
            Action::GoTop => {
                if !items.is_empty() {
                    self.select(&items, 0);
                }
                return true;
            }
            Action::GoBottom => {
                if !items.is_empty() {
                    self.select(&items, items.len() - 1);
                }
                return true;
            }
            _ => {}
        }
        let Some(item) = current.map(|i| items[i]) else {
            effects.push(Effect::Note("the inbox is empty".into()));
            return true;
        };
        match (action, &item.kind) {
            (Action::InboxAllow, InboxKind::Permission { .. }) => {
                effects.push(answer(item.id, PermissionAnswer::Allow));
            }
            (Action::InboxAllowAlways, InboxKind::Permission { suggestions, .. }) => {
                if suggestions.is_empty() {
                    // Plan B21.
                    effects.push(Effect::Note("no rule was offered to always allow".into()));
                } else {
                    effects.push(answer(item.id, PermissionAnswer::AllowAlways));
                }
            }
            (Action::InboxDeny, InboxKind::Permission { .. }) => {
                effects.push(answer(item.id, PermissionAnswer::Deny { message: None }));
            }
            (Action::InboxDenyWithReason, InboxKind::Permission { .. }) => {
                effects.push(Effect::StartInput {
                    target: InputTarget::Reason(item.id),
                    initial: String::new(),
                });
            }
            (Action::InboxAllow | Action::InboxAllowAlways | Action::InboxDeny, _)
            | (Action::InboxDenyWithReason, _) => {
                effects.push(Effect::Note("that item is not a permission request".into()))
            }
            (Action::InboxReply, InboxKind::Question { .. }) => effects.push(Effect::StartInput {
                target: InputTarget::Reply(item.id),
                initial: String::new(),
            }),
            // Plan B22: replying to a finished turn sends the agent a normal prompt.
            (Action::InboxReply, InboxKind::FinishedTurn { .. }) => {
                effects.push(Effect::StartInput {
                    target: InputTarget::Prompt(item.agent.clone()),
                    initial: String::new(),
                });
            }
            (Action::InboxReply, _) => effects.push(Effect::Note(
                "only questions and finished turns take a reply".into(),
            )),
            (Action::InboxDismiss, InboxKind::FinishedTurn { .. } | InboxKind::Error { .. }) => {
                effects.push(Effect::Command(Command::DismissInbox { item: item.id }));
            }
            (Action::InboxDismiss, _) => {
                effects.push(Effect::Note("answer permission requests and questions; x dismisses finished turns and errors".into()));
            }
            (Action::InboxOpen, _) => effects.push(Effect::OpenAgent(item.agent.clone())),
            _ => return false,
        }
        true
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        ordered(cx.snap)
            .iter()
            .map(|item| {
                let name = cx
                    .snap
                    .agents
                    .iter()
                    .find(|a| a.id == item.agent)
                    .map_or(item.agent.0.as_str(), |a| a.name.as_str());
                let (_, label) = item_label(cx.snap, item);
                let body = match &item.kind {
                    InboxKind::Permission { tool, input, .. } => {
                        input_summary(cx.snap, tool, input)
                    }
                    InboxKind::Question { text, .. } | InboxKind::Error { text } => text.clone(),
                    InboxKind::FinishedTurn { preview } => preview.clone(),
                };
                format!("{name} {label} {body}")
            })
            .collect()
    }

    fn current_row(&self, cx: &ViewCx) -> usize {
        self.resolve(&ordered(cx.snap)).unwrap_or(0)
    }

    fn goto_row(&mut self, row: usize, cx: &ViewCx, _effects: &mut Vec<Effect>) {
        let items = ordered(cx.snap);
        if row < items.len() {
            self.select(&items, row);
        }
    }
}

fn answer(item: InboxId, answer: PermissionAnswer) -> Effect {
    Effect::Command(Command::AnswerPermission { item, answer })
}

/// The context preview under the selected item: the tool and its full input, why it was
/// escalated, the agent's last message and the rule `A` would add.
fn preview(cx: &ViewCx, item: &InboxItem, width: usize) -> Vec<Line<'static>> {
    let theme = cx.theme;
    let text_width = width.saturating_sub(5);
    let mut body: Vec<Line<'static>> = Vec::new();
    let mut push = |text: &str, style: ratatui::style::Style| {
        for line in fmt::wrap(text, text_width) {
            body.push(Line::from(Span::styled(line, style)));
        }
    };
    match &item.kind {
        InboxKind::Permission {
            tool,
            input,
            reason,
            suggestions,
        } => {
            push(&input_summary(cx.snap, tool, input), theme.fg(theme.code));
            // A slice's brief reads as text under its one-line summary.
            if tool == START_SLICE_PERMISSION
                && let Some(brief) = input.get("brief").and_then(|v| v.as_str())
            {
                push(brief, theme.body());
            }
            if let Some(reason) = reason {
                push(reason, theme.dim());
            }
            if let Some(message) = last_message(cx.snap, &item.agent) {
                push(
                    &format!(
                        "“{}”",
                        fmt::truncate(fmt::first_line(&message), text_width.saturating_sub(2))
                    ),
                    theme.dim(),
                );
            }
            if let Some(rule) = suggestions.first() {
                push(
                    &format!("A → always allow: {}", rule.label),
                    theme.fg(theme.italic),
                );
            }
        }
        InboxKind::Question { text, .. } => push(text, theme.body()),
        InboxKind::FinishedTurn { preview } => push(preview, theme.body()),
        InboxKind::Error { text } => push(text, theme.fg(theme.error)),
    }
    body.into_iter()
        .map(|line| {
            let mut spans = vec![Span::styled("  │ ", theme.fg(theme.needs_you))];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

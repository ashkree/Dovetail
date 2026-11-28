//! One inbox item in full, opened with Enter from the inbox: the whole question, error or
//! permission input in the reading column, with the inbox's answer keys.

use dovetail_api::research::ProposalKind;
use dovetail_api::{
    Snapshot,
    activity::inbox::{InboxId, InboxItem, InboxKind, START_SLICE_PERMISSION},
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::inbox::{
    answer_action, input_summary, item_label, kind_color, last_message, source_name,
};
use super::{
    Buffer, Effect, ViewCx, body_height, footer, pane, tangents_title, title, unfiled_title,
};
use crate::action::Action;
use crate::fmt;
use crate::region::BufferKind;
use crate::render::{markdown, reading_column};

/// The inbox detail buffer.
#[derive(Default)]
pub struct InboxDetailBuffer {
    item: Option<InboxId>,
    top: usize,
    last_lines: usize,
    last_height: usize,
}

impl InboxDetailBuffer {
    /// The item shown.
    pub fn item(&self) -> Option<InboxId> {
        self.item
    }

    /// The item shown, if it is still in the inbox.
    pub fn current<'a>(&self, snap: &'a Snapshot) -> Option<&'a InboxItem> {
        let id = self.item?;
        snap.inbox.iter().find(|i| i.id == id)
    }

    /// Shows an item, from its first line.
    pub fn show(&mut self, item: InboxId) {
        if self.item != Some(item) {
            self.top = 0;
        }
        self.item = Some(item);
    }
}

/// Every line of an item's detail, `width` cells wide at most.
fn lines(cx: &ViewCx, item: &InboxItem, width: usize) -> Vec<Line<'static>> {
    let theme = cx.theme;
    let name = source_name(cx.snap, item);
    let (glyph, label) = item_label(item);
    let mut out = vec![
        Line::from(vec![
            Span::styled(format!("{glyph} {name}"), theme.fg(kind_color(item, theme))),
            Span::styled(format!("  {label}"), theme.accent()),
            Span::styled(format!("  {}", fmt::hhmm(item.at_ms, cx.tz)), theme.dim()),
        ]),
        Line::default(),
    ];
    let plain = |out: &mut Vec<Line<'static>>, text: &str, style| {
        out.extend(fmt::wrapped_lines(text, width, style));
        out.push(Line::default());
    };
    match &item.kind {
        InboxKind::Permission {
            tool,
            input,
            reason,
            suggestions,
        } => {
            plain(
                &mut out,
                &input_summary(cx.snap, tool, input),
                theme.fg(theme.code),
            );
            // A slice's brief reads as text under its one-line summary.
            if tool == START_SLICE_PERMISSION
                && let Some(brief) = input.get("brief").and_then(|v| v.as_str())
            {
                out.extend(markdown::render(brief, width, theme));
                out.push(Line::default());
            }
            if let Some(reason) = reason {
                plain(&mut out, reason, theme.dim());
            }
            if let Some(message) = last_message(cx.snap, &item.agent) {
                plain(
                    &mut out,
                    &format!("“{}”", fmt::first_line(&message)),
                    theme.dim(),
                );
            }
            if let Some(rule) = suggestions.first() {
                plain(
                    &mut out,
                    &format!("A → always allow: {}", rule.label),
                    theme.fg(theme.italic),
                );
            }
        }
        InboxKind::Question { text, .. } | InboxKind::FinishedTurn { preview: text } => {
            out.extend(markdown::render(text, width, theme));
        }
        InboxKind::Error { text } => plain(&mut out, text, theme.fg(theme.error)),
        InboxKind::UnfiledFinding { .. } => {
            plain(&mut out, &unfiled_title(&item.kind), theme.body());
            plain(
                &mut out,
                "A subagent of the coordinator researched something while no round was open, so \
                 the finding belongs to no task yet. Filing it moves it, with its sources, to \
                 that task's research folder; dropping it deletes it.",
                theme.dim(),
            );
        }
        InboxKind::NewTangents { count } => {
            plain(&mut out, &tangents_title(*count), theme.body());
            plain(
                &mut out,
                "Questions you asked as tangents and have not sorted yet. Each can be filed into \
                 a project (its answers and sources move there), started as a task in a project \
                 (it stays in Tangents, linked), or dropped. :tangents lists them any time.",
                theme.dim(),
            );
        }
        InboxKind::Proposal {
            task,
            text,
            evidence,
            why,
            alternatives,
            ..
        } => {
            plain(&mut out, text, theme.body());
            plain(&mut out, &format!("research task #{}", task.0), theme.dim());
            if let Some(why) = why {
                plain(&mut out, &format!("Why: {why}"), theme.body());
            }
            for alt in alternatives {
                let line = match &alt.why_not {
                    Some(why_not) => format!("Rejected: {}, because {why_not}", alt.option),
                    None => format!("Rejected: {}", alt.option),
                };
                plain(&mut out, &line, theme.dim());
            }
            if !evidence.is_empty() {
                let ids: Vec<String> = evidence.iter().map(ToString::to_string).collect();
                plain(
                    &mut out,
                    &format!("Evidence: {}", ids.join(" ")),
                    theme.accent(),
                );
            }
        }
    }
    while out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    out
}

fn hints(item: &InboxItem) -> &'static [&'static str] {
    match &item.kind {
        InboxKind::Permission { .. } => &[
            "y allow · A always · d deny · D deny with reason · ⏎ open agent · esc back",
            "y allow · A always · d deny · D reason · esc back",
            "y allow · d deny · esc back",
        ],
        InboxKind::Question { .. } => &["r reply · ⏎ open agent · esc back", "r reply · esc back"],
        InboxKind::FinishedTurn { .. } => &[
            "r reply · x dismiss · ⏎ open agent · esc back",
            "r reply · x dismiss · esc back",
        ],
        InboxKind::Error { .. } => &[
            "x dismiss · ⏎ open agent · esc back",
            "x dismiss · esc back",
        ],
        InboxKind::Proposal {
            kind: ProposalKind::Idea,
            ..
        } => &[
            "y follow-up task · A new round · d drop · esc back",
            "y task · A round · d drop · esc back",
        ],
        InboxKind::Proposal { .. } => &["y accept · d drop · esc back", "y accept · d drop"],
        InboxKind::UnfiledFinding { .. } => &[
            "y or ⏎ file under a task · d drop · esc back",
            "y file · d drop · esc back",
        ],
        InboxKind::NewTangents { .. } => &[
            "y or ⏎ triage them · x put away · esc back",
            "y triage · x away · esc back",
        ],
    }
}

impl Buffer for InboxDetailBuffer {
    fn kind(&self) -> BufferKind {
        BufferKind::InboxDetail
    }

    fn render(&mut self, cx: &ViewCx, frame: &mut Frame, area: Rect, focused: bool) {
        let theme = cx.theme;
        let inner = pane(
            frame,
            area,
            title("inbox item", focused, theme),
            focused,
            theme,
        );
        let Some(item) = self.current(cx.snap) else {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "this item was answered or dismissed · esc to go back",
                    theme.dim(),
                )),
                inner,
            );
            return;
        };
        let column = reading_column::column(inner, reading_column::DEFAULT_WIDTH);
        let all = lines(cx, item, usize::from(column.width));
        let height = body_height(inner);
        self.last_lines = all.len();
        self.last_height = height;
        self.top = self.top.min(all.len().saturating_sub(height));
        for (n, line) in all.into_iter().skip(self.top).take(height).enumerate() {
            let rect = Rect {
                y: column.y + n as u16,
                height: 1,
                ..column
            };
            frame.render_widget(Paragraph::new(line), rect);
        }
        footer(frame, column, hints(item), theme);
    }

    fn on_action(&mut self, action: Action, cx: &ViewCx, effects: &mut Vec<Effect>) -> bool {
        let max_top = self.last_lines.saturating_sub(self.last_height.max(1));
        match action {
            Action::MoveDown => {
                self.top = (self.top + 1).min(max_top);
                return true;
            }
            Action::MoveUp => {
                self.top = self.top.saturating_sub(1);
                return true;
            }
            Action::GoTop => {
                self.top = 0;
                return true;
            }
            Action::GoBottom => {
                self.top = max_top;
                return true;
            }
            _ => {}
        }
        let Some(item) = self.current(cx.snap) else {
            effects.push(Effect::Note("that item is gone".into()));
            return true;
        };
        match action {
            Action::InboxOpen if matches!(item.kind, InboxKind::UnfiledFinding { .. }) => {
                effects.push(Effect::PickFileTarget(item.id));
                true
            }
            Action::InboxOpen if matches!(item.kind, InboxKind::NewTangents { .. }) => {
                effects.push(Effect::OpenTangents);
                true
            }
            Action::InboxOpen => {
                effects.push(Effect::OpenAgent(item.agent.clone()));
                true
            }
            _ => answer_action(action, item, effects),
        }
    }

    fn search_rows(&self, cx: &ViewCx) -> Vec<String> {
        self.current(cx.snap)
            .map(|item| {
                lines(cx, item, usize::from(reading_column::DEFAULT_WIDTH))
                    .iter()
                    .map(fmt::line_plain_text)
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

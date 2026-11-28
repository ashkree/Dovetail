//! The spinner lines: an agent's working line under its conversation, and the status bar's
//! long-command indicator. Pure functions of the clock and the data, so tests need no timer.

use dovetail_api::{agents::agent::Agent, agents::conversation::ConversationEntry};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::activity::{self, Busy};
use crate::fmt;
use crate::theme::Theme;

/// `⠋ Reading plan.md… 12s`, cut to `width` cells. `since_ms` is how long the agent has been
/// working; the label is the running tool, or the turn's woodworking word.
pub fn working_line(
    theme: &Theme,
    agent: &Agent,
    entries: &[ConversationEntry],
    since_ms: i64,
    now_ms: i64,
    width: usize,
) -> Line<'static> {
    spinner_line(theme, &label_of(agent, entries), since_ms, now_ms, width)
}

/// What an agent's working line says: the running tool, or the turn's woodworking word.
pub fn label_of(agent: &Agent, entries: &[ConversationEntry]) -> String {
    activity::label(agent.tool.as_ref(), activity::turn_seed(entries))
}

/// `⠋ Closing the day… 12s` for the status bar.
pub fn busy_spans(theme: &Theme, busy: Busy, since_ms: i64, now_ms: i64) -> Vec<Span<'static>> {
    spinner_line(theme, busy.wording(), since_ms, now_ms, usize::MAX).spans
}

fn spinner_line(
    theme: &Theme,
    label: &str,
    since_ms: i64,
    now_ms: i64,
    width: usize,
) -> Line<'static> {
    let time = activity::elapsed(since_ms);
    // Spinner, space, label, space, time.
    let room = width.saturating_sub(2 + 1 + time.width());
    Line::from(spinner_spans(
        theme,
        &fmt::truncate(label, room),
        Some(time),
        now_ms,
    ))
}

/// The spinner, the label and, when given, the elapsed time: the one picture of "working" that
/// the chat's line and the task tree's rows share.
pub fn spinner_spans(
    theme: &Theme,
    label: &str,
    time: Option<String>,
    now_ms: i64,
) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled(activity::spinner(now_ms), theme.fg(theme.working)),
        Span::raw(" "),
        Span::styled(label.to_owned(), theme.fg(theme.working)),
    ];
    if let Some(time) = time {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(time, theme.dim()));
    }
    spans
}

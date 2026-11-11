//! The spinner lines: an agent's working line under its conversation, and the status bar's
//! long-command indicator. Pure functions of the clock and the data, so tests need no timer.

use dovetail_api::{Agent, ConversationEntry};
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
    let label = activity::label(agent.tool.as_ref(), activity::turn_seed(entries));
    spinner_line(theme, &label, since_ms, now_ms, width)
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
    let label = fmt::truncate(label, room);
    Line::from(vec![
        Span::styled(activity::spinner(now_ms), theme.fg(theme.working)),
        Span::raw(" "),
        Span::styled(label, theme.fg(theme.working)),
        Span::raw(" "),
        Span::styled(time, theme.dim()),
    ])
}

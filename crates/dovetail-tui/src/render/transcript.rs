//! A conversation as lines: speaker labels, markdown bodies, one dim line per turn of tool
//! activity, blank rows between entries, folded and truncated turns, older turns dimmed.

use std::collections::{HashMap, HashSet};

use dovetail_api::{ConversationEntry, EntryId, EntryKind};
use jiff::tz::TimeZone;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::markdown;
use super::wrap::wrap_spans;
use crate::fmt;
use crate::theme::Theme;

/// An assistant reply longer than this many lines is cut with a "show more" line.
pub const MAX_ENTRY_LINES: usize = 60;
/// Lines of a cut reply that stay visible.
const KEEP_LINES: usize = 40;

/// A user prompt and everything the agent did until the next prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// Index of its first line in [`Transcript::lines`].
    pub start_line: usize,
    /// The id of its first entry; identifies the turn for folding.
    pub key: EntryId,
    /// Every entry in it.
    pub entries: Vec<EntryId>,
}

/// A rendered conversation.
#[derive(Debug, Default)]
pub struct Transcript {
    /// All lines, oldest first.
    pub lines: Vec<Line<'static>>,
    /// The turns, oldest first.
    pub turns: Vec<Turn>,
}

struct Cached {
    text_len: usize,
    width: usize,
    lines: Vec<Line<'static>>,
}

/// Rendered markdown per entry, so a finished reply is parsed once, not on every frame. One
/// cache per agent: entry ids are only unique within a conversation.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<EntryId, Cached>,
}

impl Cache {
    fn markdown(
        &mut self,
        entry: &ConversationEntry,
        width: usize,
        theme: &Theme,
    ) -> &[Line<'static>] {
        let stale = self
            .entries
            .get(&entry.id)
            .is_none_or(|c| c.text_len != entry.text.len() || c.width != width);
        if stale {
            let lines = markdown::render(&entry.text, width, theme);
            self.entries.insert(
                entry.id,
                Cached {
                    text_len: entry.text.len(),
                    width,
                    lines,
                },
            );
        }
        self.entries
            .get(&entry.id)
            .map_or(&[][..], |c| c.lines.as_slice())
    }
}

/// What [`build`] needs besides the entries.
pub struct Params<'a> {
    /// Width of the reading column.
    pub width: usize,
    /// Colours.
    pub theme: &'a Theme,
    /// Time zone for the labels.
    pub tz: &'a TimeZone,
    /// Name shown on the agent's replies.
    pub agent: &'a str,
    /// The reply still being written, if the agent is mid-turn.
    pub streaming: Option<EntryId>,
    /// Turns folded to one line, by their first entry.
    pub folded: &'a HashSet<EntryId>,
    /// Replies shown in full despite their length.
    pub expanded: &'a HashSet<EntryId>,
}

/// Renders `entries`.
pub fn build(entries: &[ConversationEntry], p: &Params, cache: &mut Cache) -> Transcript {
    let mut groups: Vec<Vec<&ConversationEntry>> = Vec::new();
    for entry in entries {
        if entry.kind == EntryKind::User || groups.is_empty() {
            groups.push(Vec::new());
        }
        if let Some(group) = groups.last_mut() {
            group.push(entry);
        }
    }

    let mut out = Transcript::default();
    let last = groups.len().saturating_sub(1);
    for (index, group) in groups.iter().enumerate() {
        let Some(first) = group.first() else { continue };
        out.turns.push(Turn {
            start_line: out.lines.len(),
            key: first.id,
            entries: group.iter().map(|e| e.id).collect(),
        });
        let folded = p.folded.contains(&first.id);
        let older = index < last;
        for entry in group {
            if folded && entry.kind != EntryKind::User {
                let hidden = group.iter().filter(|e| e.kind != EntryKind::User).count();
                if hidden > 0 {
                    out.lines.push(Line::from(Span::styled(
                        format!("▸ {hidden} entries folded · zo to unfold"),
                        p.theme.dim(),
                    )));
                    out.lines.push(Line::default());
                }
                break;
            }
            let mut lines = entry_lines(entry, p, cache);
            if older {
                dim(&mut lines, p.theme);
            }
            out.lines.append(&mut lines);
            out.lines.push(Line::default());
        }
    }
    out
}

fn entry_lines(entry: &ConversationEntry, p: &Params, cache: &mut Cache) -> Vec<Line<'static>> {
    let theme = p.theme;
    let width = p.width;
    match entry.kind {
        EntryKind::User => {
            let mut lines = vec![label("you", entry, p, false)];
            for text in entry.text.split('\n') {
                let spans = [Span::styled(text.to_string(), theme.body())];
                for line in wrap_spans(&spans, width) {
                    lines.push(Line::from(line));
                }
            }
            lines
        }
        EntryKind::Assistant => {
            let streaming = p.streaming == Some(entry.id);
            let mut lines = vec![label(p.agent, entry, p, streaming)];
            let mut body: Vec<Line<'static>> = cache.markdown(entry, width, theme).to_vec();
            if !streaming && body.len() > MAX_ENTRY_LINES && !p.expanded.contains(&entry.id) {
                let more = body.len() - KEEP_LINES;
                body.truncate(KEEP_LINES);
                body.push(Line::from(Span::styled(
                    format!("… {more} more lines · zo to show"),
                    theme.dim(),
                )));
            }
            if streaming {
                let cursor = Span::styled("▌", theme.fg(theme.working));
                match body.last_mut() {
                    Some(line) => line.spans.push(cursor),
                    None => body.push(Line::from(cursor)),
                }
            }
            lines.append(&mut body);
            lines
        }
        EntryKind::ToolSummary => {
            let spans = [Span::styled(format!("▸ {}", entry.text), theme.dim())];
            wrap_spans(&spans, width)
                .into_iter()
                .map(Line::from)
                .collect()
        }
        EntryKind::Notice => {
            let style = theme.dim().add_modifier(Modifier::ITALIC);
            let spans = [Span::styled(format!("· {}", entry.text), style)];
            wrap_spans(&spans, width)
                .into_iter()
                .map(Line::from)
                .collect()
        }
    }
}

fn label(name: &str, entry: &ConversationEntry, p: &Params, streaming: bool) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{name} · {}", fmt::hhmm(entry.at_ms, p.tz)),
        p.theme.accent(),
    )];
    if streaming {
        spans.push(Span::styled(" ◐", p.theme.fg(p.theme.working)));
    }
    Line::from(spans)
}

/// Older turns recede: every span loses its colour for the dim one but keeps bold and italic.
fn dim(lines: &mut [Line<'static>], theme: &Theme) {
    for line in lines {
        for span in &mut line.spans {
            span.style = Style {
                fg: Some(theme.dim),
                ..span.style
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, kind: EntryKind, text: &str) -> ConversationEntry {
        ConversationEntry {
            id: EntryId(id),
            at_ms: 1_793_528_040_000,
            kind,
            text: text.into(),
        }
    }

    fn run(entries: &[ConversationEntry], mutate: impl FnOnce(&mut Params)) -> Transcript {
        let theme = Theme::dusk();
        let folded = HashSet::new();
        let expanded = HashSet::new();
        let mut p = Params {
            width: 60,
            theme: &theme,
            tz: &TimeZone::UTC,
            agent: "coordinator",
            streaming: None,
            folded: &folded,
            expanded: &expanded,
        };
        mutate(&mut p);
        build(entries, &p, &mut Cache::default())
    }

    fn texts(t: &Transcript) -> Vec<String> {
        t.lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn labels_blank_rows_and_a_folded_tool_line() {
        let entries = [
            entry(1, EntryKind::User, "What is blocking slice-7?"),
            entry(2, EntryKind::Assistant, "It needs a **key**."),
            entry(3, EntryKind::ToolSummary, "read 2 files · 4 s"),
        ];
        let t = run(&entries, |_| {});
        assert_eq!(
            texts(&t),
            vec![
                "you · 10:14",
                "What is blocking slice-7?",
                "",
                "coordinator · 10:14",
                "It needs a key.",
                "",
                "▸ read 2 files · 4 s",
                "",
            ]
        );
    }

    #[test]
    fn a_turn_starts_at_each_user_prompt() {
        let entries = [
            entry(1, EntryKind::User, "a"),
            entry(2, EntryKind::Assistant, "b"),
            entry(3, EntryKind::User, "c"),
            entry(4, EntryKind::Assistant, "d"),
        ];
        let t = run(&entries, |_| {});
        assert_eq!(t.turns.len(), 2);
        assert_eq!(t.turns[0].start_line, 0);
        assert_eq!(t.turns[1].key, EntryId(3));
        assert_eq!(t.turns[1].entries, vec![EntryId(3), EntryId(4)]);
        assert_eq!(texts(&t)[t.turns[1].start_line], "you · 10:14");
    }

    #[test]
    fn entries_before_the_first_prompt_form_a_turn_of_their_own() {
        let entries = [
            entry(1, EntryKind::Assistant, "hello"),
            entry(2, EntryKind::User, "hi"),
        ];
        assert_eq!(run(&entries, |_| {}).turns.len(), 2);
    }

    #[test]
    fn older_turns_are_dimmed_and_the_latest_is_not() {
        let entries = [
            entry(1, EntryKind::User, "old"),
            entry(2, EntryKind::Assistant, "older reply"),
            entry(3, EntryKind::User, "new"),
        ];
        let t = run(&entries, |_| {});
        let theme = Theme::dusk();
        let old_label = &t.lines[0].spans[0];
        assert_eq!(old_label.style.fg, Some(theme.dim));
        let new_label = &t.lines[t.turns[1].start_line].spans[0];
        assert_eq!(new_label.style.fg, Some(theme.bold));
    }

    #[test]
    fn a_folded_turn_keeps_the_prompt_and_hides_the_rest() {
        let entries = [
            entry(1, EntryKind::User, "do it"),
            entry(2, EntryKind::Assistant, "done"),
            entry(3, EntryKind::ToolSummary, "ran tests"),
        ];
        let mut folded = HashSet::new();
        folded.insert(EntryId(1));
        let theme = Theme::dusk();
        let expanded = HashSet::new();
        let p = Params {
            width: 60,
            theme: &theme,
            tz: &TimeZone::UTC,
            agent: "c",
            streaming: None,
            folded: &folded,
            expanded: &expanded,
        };
        let t = build(&entries, &p, &mut Cache::default());
        assert_eq!(
            texts(&t),
            vec![
                "you · 10:14",
                "do it",
                "",
                "▸ 2 entries folded · zo to unfold",
                ""
            ]
        );
    }

    #[test]
    fn the_streaming_reply_shows_a_marker_and_a_cursor() {
        let entries = [entry(1, EntryKind::Assistant, "typing")];
        let t = run(&entries, |p| p.streaming = Some(EntryId(1)));
        let t = texts(&t);
        assert_eq!(t[0], "coordinator · 10:14 ◐");
        assert_eq!(t[1], "typing▌");
    }

    #[test]
    fn a_long_reply_is_cut_with_a_show_more_line_unless_expanded() {
        let long = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let entries = [entry(1, EntryKind::Assistant, &long)];
        let cut = texts(&run(&entries, |_| {}));
        assert!(cut.iter().any(|l| l.contains("more lines · zo to show")));
        assert!(!cut.iter().any(|l| l == "line 99"));
        let theme = Theme::dusk();
        let folded = HashSet::new();
        let mut expanded = HashSet::new();
        expanded.insert(EntryId(1));
        let p = Params {
            width: 60,
            theme: &theme,
            tz: &TimeZone::UTC,
            agent: "c",
            streaming: None,
            folded: &folded,
            expanded: &expanded,
        };
        let full = texts(&build(&entries, &p, &mut Cache::default()));
        assert!(full.iter().any(|l| l == "line 99"));
    }

    #[test]
    fn user_text_keeps_its_line_breaks_and_wraps() {
        let entries = [entry(
            1,
            EntryKind::User,
            "first\nsecond line that is rather long indeed and wraps around",
        )];
        let t = texts(&run(&entries, |p| p.width = 20));
        assert_eq!(t[1], "first");
        assert!(t.len() > 4);
    }

    #[test]
    fn the_cache_reparses_only_when_text_or_width_changes() {
        let theme = Theme::dusk();
        let mut cache = Cache::default();
        let mut e = entry(1, EntryKind::Assistant, "one");
        assert_eq!(cache.markdown(&e, 40, &theme).len(), 1);
        e.text.push_str("\n\ntwo");
        assert_eq!(cache.markdown(&e, 40, &theme).len(), 3);
    }
}

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

/// The lines one entry occupies, its header and trailing blank rows included, so a row of the
/// view maps to the entry under it. A folded turn's "entries folded" line belongs to the turn's
/// first entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntrySpan {
    /// The entry.
    pub id: EntryId,
    /// First line.
    pub start: usize,
    /// One past the last line.
    pub end: usize,
}

/// The entry whose lines include `row` (the last entry when `row` is past the end).
pub fn entry_at(spans: &[EntrySpan], row: usize) -> Option<EntryId> {
    spans
        .iter()
        .find(|s| row >= s.start && row < s.end)
        .or(spans.last())
        .map(|s| s.id)
}

/// The reply a reader is on: the last assistant entry. The length rule never cuts it.
pub fn newest_reply(entries: &[ConversationEntry]) -> Option<EntryId> {
    entries
        .iter()
        .rfind(|e| e.kind == EntryKind::Assistant)
        .map(|e| e.id)
}

/// A rendered conversation.
#[derive(Debug, Default)]
pub struct Transcript {
    /// All lines, oldest first.
    pub lines: Vec<Line<'static>>,
    /// The turns, oldest first.
    pub turns: Vec<Turn>,
    /// Which lines belong to which entry, oldest first.
    pub spans: Vec<EntrySpan>,
}

struct Cached {
    /// The [`build`] that last used it; older entries are dropped when a build ends.
    used: u64,
    text_len: usize,
    width: usize,
    lines: Vec<Line<'static>>,
}

/// Rendered markdown per entry, so a finished reply is parsed once, not on every frame. One
/// cache per agent: entry ids are only unique within a conversation.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<EntryId, Cached>,
    /// Counts builds, so entries no longer in the conversation (trimmed away) do not stay.
    build: u64,
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
                    used: self.build,
                    text_len: entry.text.len(),
                    width,
                    lines,
                },
            );
        }
        self.entries.get_mut(&entry.id).map_or(&[][..], |c| {
            c.used = self.build;
            c.lines.as_slice()
        })
    }

    /// Drops what the build that just ran did not use.
    fn end_build(&mut self) {
        let build = self.build;
        self.entries.retain(|_, c| c.used == build);
    }

    /// Entries held, for the memory test.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
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
    /// Replies the user cut with `zc`, even the newest one or a shorter one.
    pub collapsed: &'a HashSet<EntryId>,
    /// The reply being read ([`newest_reply`]); only the user's `zc` cuts it.
    pub newest: Option<EntryId>,
}

/// Renders `entries`.
pub fn build(entries: &[ConversationEntry], p: &Params, cache: &mut Cache) -> Transcript {
    cache.build += 1;
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
    // Every entry ends with a blank row; each message adds one more, so two blank rows come
    // above each header (one at the very top) and a change of speaker stands apart from the
    // rows between one speaker's entries.
    let mut speaker: Option<Speaker> = None;
    for (index, group) in groups.iter().enumerate() {
        let Some(first) = group.first() else { continue };
        out.lines.push(Line::default());
        out.turns.push(Turn {
            start_line: out.lines.len() - 1,
            key: first.id,
            entries: group.iter().map(|e| e.id).collect(),
        });
        let folded = p.folded.contains(&first.id);
        let older = index < last;
        for (at, entry) in group.iter().enumerate() {
            // The turn's leading blank row belongs to its first entry.
            let start = if at == 0 {
                out.lines.len() - 1
            } else {
                out.lines.len()
            };
            if folded && entry.kind != EntryKind::User {
                let hidden = group.iter().filter(|e| e.kind != EntryKind::User).count();
                if hidden > 0 {
                    out.lines.push(Line::from(Span::styled(
                        format!("▸ {hidden} entries folded · zo to unfold"),
                        p.theme.dim(),
                    )));
                    out.lines.push(Line::default());
                }
                // The fold line joins the span of the prompt above it.
                let end = out.lines.len();
                match out.spans.last_mut().filter(|_| at > 0) {
                    Some(span) => span.end = end,
                    None => out.spans.push(EntrySpan {
                        id: entry.id,
                        start,
                        end,
                    }),
                }
                break;
            }
            let who = Speaker::of(entry.kind);
            let mut lines = Vec::new();
            if speaker != Some(who) {
                // The first entry's extra blank row came with its turn.
                if at > 0 {
                    out.lines.push(Line::default());
                }
                speaker = Some(who);
                // The marker shows while any entry of this message is still being written.
                let streaming = group[at..]
                    .iter()
                    .take_while(|e| Speaker::of(e.kind) == who)
                    .any(|e| p.streaming == Some(e.id));
                lines.push(header(who, entry, p, streaming));
                // A blank row parts the header from the text under it.
                lines.push(Line::default());
            }
            lines.append(&mut entry_lines(entry, p, cache));
            if older {
                dim(&mut lines, p.theme);
            }
            out.lines.append(&mut lines);
            out.lines.push(Line::default());
            out.spans.push(EntrySpan {
                id: entry.id,
                start,
                end: out.lines.len(),
            });
        }
    }
    cache.end_build();
    out
}

/// Who a message is from. Tool summaries and notices belong to the agent's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Speaker {
    You,
    Agent,
}

impl Speaker {
    fn of(kind: EntryKind) -> Self {
        match kind {
            EntryKind::User => Self::You,
            _ => Self::Agent,
        }
    }
}

/// The rule that opens a message, `── You · 10:14 ────…`, as wide as the reading column.
fn header(who: Speaker, entry: &ConversationEntry, p: &Params, streaming: bool) -> Line<'static> {
    let theme = p.theme;
    let (name, colour, rule) = match who {
        Speaker::You => ("You", theme.speaker_you, theme.speaker_you_rule),
        Speaker::Agent => (p.agent, theme.speaker_agent, theme.speaker_agent_rule),
    };
    let rule = theme.fg(rule);
    let mut spans = vec![
        Span::styled("── ", rule),
        Span::styled(
            name.to_string(),
            theme.fg(colour).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" · {}", fmt::hhmm(entry.at_ms, p.tz)), rule),
    ];
    if streaming {
        spans.push(Span::styled(" ◐", theme.fg(theme.working)));
    }
    let used: usize = spans.iter().map(Span::width).sum();
    let fill = p.width.saturating_sub(used + 1).max(2);
    spans.push(Span::styled(format!(" {}", "─".repeat(fill)), rule));
    Line::from(spans)
}

/// Attachment chips laid out two spaces apart, a chip never split across rows.
pub fn chip_lines(chips: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut used = 0;
    for chip in chips {
        let w = chip.width();
        match rows.last_mut() {
            Some(row) if used + 2 + w <= width => {
                row.push(Span::raw("  "));
                row.push(chip);
                used += 2 + w;
            }
            _ => {
                rows.push(vec![chip]);
                used = w;
            }
        }
    }
    rows.into_iter().map(Line::from).collect()
}

fn entry_lines(entry: &ConversationEntry, p: &Params, cache: &mut Cache) -> Vec<Line<'static>> {
    let theme = p.theme;
    let width = p.width;
    match entry.kind {
        EntryKind::User => {
            let mut lines = Vec::new();
            if !entry.attachments.is_empty() {
                let chips = entry
                    .attachments
                    .iter()
                    .enumerate()
                    .map(|(i, a)| Span::styled(fmt::image_chip(i + 1, a.size), theme.chip()))
                    .collect();
                lines.extend(chip_lines(chips, width));
            }
            // An image-only message has no text lines.
            if !entry.text.is_empty() || entry.attachments.is_empty() {
                for text in entry.text.split('\n') {
                    let spans = [Span::styled(text.to_string(), theme.body())];
                    for line in wrap_spans(&spans, width) {
                        lines.push(Line::from(line));
                    }
                }
            }
            lines
        }
        EntryKind::Assistant => {
            let streaming = p.streaming == Some(entry.id);
            let mut body: Vec<Line<'static>> = cache.markdown(entry, width, theme).to_vec();
            if !streaming {
                fold_long(&mut body, entry.id, p);
            }
            if streaming {
                let cursor = Span::styled("▌", theme.fg(theme.working));
                match body.last_mut() {
                    Some(line) => line.spans.push(cursor),
                    None => body.push(Line::from(cursor)),
                }
            }
            body
        }
        EntryKind::ToolSummary => {
            let spans = [Span::styled(format!("▸ {}", entry.text), theme.dim())];
            wrap_spans(&spans, width)
                .into_iter()
                .map(Line::from)
                .collect()
        }
        EntryKind::AgentMessage => agent_message_lines(entry, width, p),
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

/// The shared fold rule: a body longer than [`MAX_ENTRY_LINES`] is cut to [`KEEP_LINES`] plus a
/// dim "show more" line, unless the entry is in `p.expanded` (keyed by entry id) or is the newest
/// reply (`p.newest`). An entry in `p.collapsed` is cut from [`KEEP_LINES`] on, newest or not.
fn fold_long(body: &mut Vec<Line<'static>>, id: EntryId, p: &Params) {
    let forced = p.collapsed.contains(&id);
    let limit = if forced { KEEP_LINES } else { MAX_ENTRY_LINES };
    if body.len() <= limit || p.expanded.contains(&id) || (!forced && p.newest == Some(id)) {
        return;
    }
    let more = body.len() - KEEP_LINES;
    body.truncate(KEEP_LINES);
    body.push(Line::from(Span::styled(
        format!("… {more} more lines · zo to show"),
        p.theme.dim(),
    )));
}

/// A message from another agent: always one dim line until the entry is in `p.expanded`
/// (`zo`), then its whole text in the same dim italic style. `zc` removes it from the set.
fn agent_message_lines(entry: &ConversationEntry, width: usize, p: &Params) -> Vec<Line<'static>> {
    let style = p.theme.dim().add_modifier(Modifier::ITALIC);
    let Some(msg) = entry.agent_message() else {
        return Vec::new();
    };
    let wrap = |text: String| -> Vec<Line<'static>> {
        wrap_spans(&[Span::styled(text, style)], width)
            .into_iter()
            .map(Line::from)
            .collect()
    };
    if !p.expanded.contains(&entry.id) {
        return wrap(if msg.subagent {
            format!("· subagent {} finished · zo to show", msg.from)
        } else {
            format!("· message from {} · zo to show", msg.from)
        });
    }
    msg.body
        .split('\n')
        .flat_map(|text| wrap(text.to_string()))
        .collect()
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
            attachments: Vec::new(),
            effort: None,
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
            collapsed: &HashSet::new(),
            newest: None,
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
                "".to_string(),
                rule("── You · 10:14", 60),
                "".into(),
                "What is blocking slice-7?".into(),
                "".into(),
                "".into(),
                rule("── coordinator · 10:14", 60),
                "".into(),
                "It needs a key.".into(),
                "".into(),
                "▸ read 2 files · 4 s".into(),
                "".into(),
            ]
        );
    }

    /// `head` and a rule filling the rest of `width`.
    fn rule(head: &str, width: usize) -> String {
        let fill = width - head.chars().count() - 1;
        format!("{head} {}", "─".repeat(fill))
    }

    #[test]
    fn a_header_rule_spans_the_reading_column() {
        let entries = [entry(1, EntryKind::User, "hi")];
        let t = run(&entries, |p| p.width = 40);
        assert_eq!(t.lines[1].width(), 40);
    }

    #[test]
    fn consecutive_agent_entries_share_one_header() {
        let entries = [
            entry(1, EntryKind::User, "go"),
            entry(2, EntryKind::Assistant, "first chunk"),
            entry(3, EntryKind::ToolSummary, "ran tests"),
            entry(4, EntryKind::Assistant, "second chunk"),
            entry(5, EntryKind::Notice, "turn finished"),
            entry(6, EntryKind::User, "again"),
        ];
        let t = texts(&run(&entries, |_| {}));
        let headers: Vec<&String> = t.iter().filter(|l| l.starts_with("── ")).collect();
        assert_eq!(
            headers,
            vec![
                &rule("── You · 10:14", 60),
                &rule("── coordinator · 10:14", 60),
                &rule("── You · 10:14", 60),
            ]
        );
    }

    #[test]
    fn speakers_get_their_theme_colours_and_the_rule_a_dimmed_one() {
        let theme = Theme::dusk();
        let entries = [
            entry(1, EntryKind::User, "a"),
            entry(2, EntryKind::Assistant, "b"),
        ];
        let t = run(&entries, |_| {});
        let you = &t.lines[1].spans;
        assert_eq!(you[0].style.fg, Some(theme.speaker_you_rule));
        assert_eq!(you[2].style.fg, Some(theme.speaker_you_rule));
        assert_eq!(you[1].style.fg, Some(theme.speaker_you));
        let agent = &t.lines[6].spans;
        assert_eq!(agent[0].style.fg, Some(theme.speaker_agent_rule));
        assert_eq!(agent[1].style.fg, Some(theme.speaker_agent));
        assert!(
            t.lines
                .iter()
                .flat_map(|l| &l.spans)
                .all(|s| s.style.bg.is_none())
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
        // A turn starts at the blank row above its first header.
        assert_eq!(texts(&t)[t.turns[1].start_line], "");
        assert_eq!(
            texts(&t)[t.turns[1].start_line + 1],
            rule("── You · 10:14", 60)
        );
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
        let old_name = &t.lines[1].spans[1];
        assert_eq!(old_name.style.fg, Some(theme.dim));
        let new_name = &t.lines[t.turns[1].start_line + 1].spans[1];
        assert_eq!(new_name.style.fg, Some(theme.speaker_you));
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
            collapsed: &HashSet::new(),
            newest: None,
        };
        let t = build(&entries, &p, &mut Cache::default());
        assert_eq!(
            texts(&t),
            vec![
                "".to_string(),
                rule("── You · 10:14", 60),
                "".into(),
                "do it".into(),
                "".into(),
                "▸ 2 entries folded · zo to unfold".into(),
                "".into(),
            ]
        );
    }

    #[test]
    fn the_streaming_reply_shows_a_marker_and_a_cursor() {
        let entries = [entry(1, EntryKind::Assistant, "typing")];
        let t = run(&entries, |p| p.streaming = Some(EntryId(1)));
        let t = texts(&t);
        assert_eq!(t[1], rule("── coordinator · 10:14 ◐", 60));
        assert_eq!(t[3], "typing▌");
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
            collapsed: &HashSet::new(),
            newest: None,
        };
        let full = texts(&build(&entries, &p, &mut Cache::default()));
        assert!(full.iter().any(|l| l == "line 99"));
    }

    #[test]
    fn an_agent_message_is_one_line_until_expanded() {
        let report = (0..100)
            .map(|i| format!("finding {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let entries = [entry(
            1,
            EntryKind::AgentMessage,
            &dovetail_api::agent_message_text("scout", true, &report),
        )];
        let folded = texts(&run(&entries, |_| {}));
        assert_eq!(folded[3], "· subagent scout finished · zo to show");
        assert!(!folded.iter().any(|l| l.contains("finding")));
        let theme = Theme::dusk();
        let none = HashSet::new();
        let expanded = HashSet::from([EntryId(1)]);
        let p = Params {
            width: 60,
            theme: &theme,
            tz: &TimeZone::UTC,
            agent: "coordinator",
            streaming: None,
            folded: &none,
            expanded: &expanded,
            collapsed: &HashSet::new(),
            newest: None,
        };
        let full = texts(&build(&entries, &p, &mut Cache::default()));
        assert_eq!(full[3], "finding 0");
        assert!(full.iter().any(|l| l == "finding 99"));
    }

    #[test]
    fn a_message_from_a_peer_names_the_sender() {
        let text = dovetail_api::agent_message_text("slice-7", false, "hello");
        let entries = [entry(1, EntryKind::AgentMessage, &text)];
        let t = texts(&run(&entries, |_| {}));
        assert_eq!(t[3], "· message from slice-7 · zo to show");
    }

    #[test]
    fn user_text_keeps_its_line_breaks_and_wraps() {
        let entries = [entry(
            1,
            EntryKind::User,
            "first\nsecond line that is rather long indeed and wraps around",
        )];
        let t = texts(&run(&entries, |p| p.width = 20));
        assert_eq!(t[3], "first");
        assert!(t.len() > 4);
    }

    #[test]
    fn the_cache_drops_entries_the_conversation_no_longer_holds() {
        let theme = Theme::dusk();
        let folded = HashSet::new();
        let expanded = HashSet::new();
        let p = Params {
            width: 60,
            theme: &theme,
            tz: &TimeZone::UTC,
            agent: "coordinator",
            streaming: None,
            folded: &folded,
            expanded: &expanded,
            collapsed: &HashSet::new(),
            newest: None,
        };
        let mut cache = Cache::default();
        let entries: Vec<_> = (1..=10)
            .map(|i| entry(i, EntryKind::Assistant, "text"))
            .collect();
        build(&entries, &p, &mut cache);
        assert_eq!(cache.len(), 10);
        build(&entries[6..], &p, &mut cache);
        assert_eq!(cache.len(), 4);
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
